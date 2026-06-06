use embassy_executor::Spawner;
use embassy_net::{
    udp::{PacketMetadata, UdpSocket},
    Ipv4Address, Stack,
};
use rtt_target::rprintln;
use static_cell::StaticCell;

use crate::storage::Storage;

const SACN_PORT: u16 = 5568;

// ACN Packet Identifier at bytes 4..16 of every E1.31 packet.
const ACN_ID: &[u8; 12] = b"ASC-E1.17\0\0\0";

// A full universe is 126 bytes of header + 512 slots = 638 bytes.
// Senders may send partial universes, so we size for the maximum.
static RX_META: StaticCell<[PacketMetadata; 4]> = StaticCell::new();
static RX_DATA: StaticCell<[u8; 638]> = StaticCell::new();
static TX_META: StaticCell<[PacketMetadata; 1]> = StaticCell::new();
static TX_DATA: StaticCell<[u8; 64]> = StaticCell::new();

pub fn spawn(spawner: Spawner, stack: Stack<'static>, storage: &'static Storage) {
    spawner.spawn(task(stack, storage).unwrap());
}

/// Listens for sACN (E1.31) packets on UDP 5568, reads the DMX slot at
/// `storage.read_dmx_base_address()` from each packet for the configured universe,
/// and prints the value via RTT whenever it changes.
#[embassy_executor::task]
async fn task(stack: Stack<'static>, storage: &'static Storage) -> ! {
    let universe = storage.read_universe();

    // sACN multicast address: 239.255.(universe_hi).(universe_lo)
    let multicast = Ipv4Address::new(239, 255, (universe >> 8) as u8, universe as u8);
    stack.join_multicast_group(multicast).ok();

    let mut socket = UdpSocket::new(
        stack,
        RX_META.init([PacketMetadata::EMPTY; 4]),
        RX_DATA.init([0; 638]),
        TX_META.init([PacketMetadata::EMPTY; 1]),
        TX_DATA.init([0; 64]),
    );
    socket.bind(SACN_PORT).ok();

    let mut last_value: Option<u8> = None;
    let mut pkt_buf = [0u8; 638];

    loop {
        let Ok((n, _)) = socket.recv_from(&mut pkt_buf).await else {
            continue;
        };
        let slot = storage.read_dmx_base_address();
        let Some(val) = parse_e131_slot(&pkt_buf[..n], universe, slot) else {
            continue;
        };
        if Some(val) != last_value {
            last_value = Some(val);
            rprintln!("DMX ch {} = {}", slot, val);
        }
    }
}

/// Extracts DMX `slot` (1-indexed) from an E1.31 data packet for `universe`.
/// Returns None if the packet is invalid, for a different universe, uses a
/// non-zero start code, or does not contain the requested slot.
///
/// E1.31 byte offsets used:
///   4..16   ACN Packet Identifier
///   18..22  Root vector    = 0x00000004
///   40..44  Framing vector = 0x00000002
///   113..115 Universe (BE u16)
///   117     DMP vector     = 0x02
///   123..125 Property count (includes start code at slot 0)
///   125     DMX start code = 0x00
///   126+    DMX slots 1..N
fn parse_e131_slot(pkt: &[u8], universe: u16, slot: u16) -> Option<u8> {
    if pkt.len() < 126 {
        return None;
    }
    if &pkt[4..16] != ACN_ID {
        return None;
    }
    if u32::from_be_bytes([pkt[18], pkt[19], pkt[20], pkt[21]]) != 0x00000004 {
        return None;
    }
    if u32::from_be_bytes([pkt[40], pkt[41], pkt[42], pkt[43]]) != 0x00000002 {
        return None;
    }
    if u16::from_be_bytes([pkt[113], pkt[114]]) != universe {
        return None;
    }
    if pkt[117] != 0x02 {
        return None;
    }
    if pkt[125] != 0x00 {
        return None;
    }
    let prop_count = u16::from_be_bytes([pkt[123], pkt[124]]);
    let offset = 125 + slot as usize;
    if slot >= prop_count || offset >= pkt.len() {
        return None;
    }
    Some(pkt[offset])
}
