use embassy_net::{
    Stack,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Instant, with_deadline};
use rtt_target::rprintln;

use crate::models::{DmxConfig, DmxSender, DmxState, DmxValue};

// The E1.31 network-data-loss timeout: a universe silent for this long is lost, and what
// must not be left running on a stale value is told so.
const DATA_LOSS_TIMEOUT: Duration = Duration::from_millis(2500);
// A universe silent for this long is taken to be a dead socket and not a quiet sender.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

// ACN Packet Identifier at bytes 4..16 of every E1.31 packet.
const ACN_ID: &[u8; 12] = b"ASC-E1.17\0\0\0";

// E1.31 packet byte offsets and field values.
const ACN_ID_OFFSET: usize = 4;
const ROOT_VECTOR_OFFSET: usize = 18;
const ROOT_VECTOR: u32 = 0x0000_0004;
const FRAMING_VECTOR_OFFSET: usize = 40;
const FRAMING_VECTOR: u32 = 0x0000_0002;
const UNIVERSE_OFFSET: usize = 113;
const DMP_VECTOR_OFFSET: usize = 117;
const DMP_VECTOR: u8 = 0x02;
const PROP_COUNT_OFFSET: usize = 123;
const START_CODE_OFFSET: usize = 125;
const DMX_NULL_START: u8 = 0x00;

// Maximum E1.31 UDP payload: 126-byte header + 512 DMX slots.
const MAX_PACKET_LEN: usize = 638;

/// The socket's ring-buffer storage. Owned by the caller of [`listen`] so that it outlives
/// each socket and is reused by the next one.
pub(crate) struct Buffers {
    rx_meta: [PacketMetadata; 4],
    rx_data: [u8; MAX_PACKET_LEN],
    tx_meta: [PacketMetadata; 1],
    tx_data: [u8; 64],
}

impl Buffers {
    pub(crate) fn new() -> Self {
        Self {
            rx_meta: [PacketMetadata::EMPTY; 4],
            rx_data: [0; MAX_PACKET_LEN],
            tx_meta: [PacketMetadata::EMPTY; 1],
            tx_data: [0; 64],
        }
    }
}

/// Publishes to `dmx_state` whenever the DMX value at the configured address changes, and
/// when the universe has been silent for the data-loss timeout.
/// Returns on the socket timeout so the caller can call it again, which binds a fresh socket and
/// sends a fresh IGMP join. Rejoining the multicast group on the same socket after a
/// timeout is not sufficient: the socket may be stale from the WiFi disconnect and
/// embassy_net may not deliver packets to it after reconnect.
///
/// No source arbitration here: every packet for our universe is obeyed, whoever sent it.
/// Choosing between a console and the brain happens on the Pi, which leaves the brain as
/// the only sender on the access point.
#[allow(
    clippy::large_stack_frames,
    reason = "pkt_buf (638 bytes) must be held across the recv_from await"
)]
pub(crate) async fn listen(
    network_stack: Stack<'static>,
    config: DmxConfig,
    dmx_state: &DmxSender,
    buffers: &mut Buffers,
) {
    let multicast = config.multicast_address();
    if let Err(e) = network_stack.join_multicast_group(multicast) {
        rprintln!("multicast join error: {:?}", e);
    }

    let mut socket = UdpSocket::new(
        network_stack,
        &mut buffers.rx_meta,
        &mut buffers.rx_data,
        &mut buffers.tx_meta,
        &mut buffers.tx_data,
    );
    if let Err(e) = socket.bind(config.sacn_port()) {
        rprintln!("socket bind error: {:?}", e);
    }

    rprintln!("dmx address:    {}", config.address());
    rprintln!("dmx universe:   {}", config.universe());
    rprintln!("sacn multicast: {}:{}", multicast, config.sacn_port());

    let publish = |state: DmxState| {
        dmx_state.send_if_modified(|current| {
            let changed = *current != Some(state);
            *current = Some(state);
            changed
        });
    };

    let mut pkt_buf = [0u8; MAX_PACKET_LEN];
    // Counted from the last packet for our universe and not from the last datagram, so a
    // stranger on the port cannot keep a lost universe looking alive.
    let mut last_heard = Instant::now();
    loop {
        let lost = last_heard.elapsed() >= DATA_LOSS_TIMEOUT;
        let deadline = last_heard + if lost { SOCKET_TIMEOUT } else { DATA_LOSS_TIMEOUT };
        match with_deadline(deadline, socket.recv_from(&mut pkt_buf)).await {
            Ok(Ok((n, _))) => {
                let value = parse_e131_slots(&pkt_buf[..n], config.universe(), config.address());
                if let Some(value) = value {
                    last_heard = Instant::now();
                    publish(DmxState::Live(value));
                }
            }
            Ok(Err(e)) => {
                rprintln!("recv_from error: {:?}", e);
            }
            Err(_) if lost => {
                rprintln!(
                    "no universe for {} seconds, recreating socket",
                    SOCKET_TIMEOUT.as_secs()
                );
                return;
            }
            Err(_) => publish(DmxState::Lost),
        }
    }
}

/// Big-endian field readers. `parse_e131_slots` length-checks the packet up front, so
/// every offset these are called with is in range and the slice is the exact width —
/// `try_into` cannot fail.
fn be_u16(pkt: &[u8], off: usize) -> u16 {
    u16::from_be_bytes(pkt[off..off + 2].try_into().unwrap())
}
fn be_u32(pkt: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(pkt[off..off + 4].try_into().unwrap())
}

/// Parses an E1.31 data packet for our universe into the ball's consecutive DMX slots.
/// `base_address` is the 1-indexed DMX address of the first slot.
/// Returns None if the packet is invalid, for a different universe, uses a non-zero
/// start code, or does not contain all of the ball's slots.
fn parse_e131_slots(pkt: &[u8], universe: u16, base_address: u16) -> Option<DmxValue> {
    if pkt.len() < START_CODE_OFFSET + 1 {
        return None;
    }
    if &pkt[ACN_ID_OFFSET..ACN_ID_OFFSET + ACN_ID.len()] != ACN_ID {
        return None;
    }
    if be_u32(pkt, ROOT_VECTOR_OFFSET) != ROOT_VECTOR {
        return None;
    }
    if be_u32(pkt, FRAMING_VECTOR_OFFSET) != FRAMING_VECTOR {
        return None;
    }
    if be_u16(pkt, UNIVERSE_OFFSET) != universe {
        return None;
    }
    if pkt[DMP_VECTOR_OFFSET] != DMP_VECTOR {
        return None;
    }
    if pkt[START_CODE_OFFSET] != DMX_NULL_START {
        return None;
    }

    let prop_count = be_u16(pkt, PROP_COUNT_OFFSET);
    let last_slot = base_address + DmxValue::LEN as u16 - 1;
    let base = START_CODE_OFFSET + base_address as usize;
    let last_offset = START_CODE_OFFSET + last_slot as usize;

    if last_slot >= prop_count || last_offset >= pkt.len() {
        return None;
    }
    let slots: [u8; DmxValue::LEN] = pkt[base..base + DmxValue::LEN].try_into().ok()?;
    Some(DmxValue::new(slots))
}
