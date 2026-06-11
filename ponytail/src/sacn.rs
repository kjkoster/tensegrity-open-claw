use core::cell::UnsafeCell;
use embassy_net::{
    Ipv4Address, Stack,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, with_timeout};
use rtt_target::rprintln;

use crate::models::{DmxConfig, DmxValue};

const UNIVERSE_TIMEOUT: u64 = 30; // seconds

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

// Socket ring-buffer storage. Lives 'static so UdpSocket<'static> can borrow it.
// Only one Listener exists at a time; main enforces this by dropping the old one
// before creating a new one.
struct SacnBufs {
    rx_meta: UnsafeCell<[PacketMetadata; 4]>,
    rx_data: UnsafeCell<[u8; MAX_PACKET_LEN]>,
    tx_meta: UnsafeCell<[PacketMetadata; 1]>,
    tx_data: UnsafeCell<[u8; 64]>,
}
// SAFETY: single-task access guaranteed by the one-Listener-at-a-time invariant.
unsafe impl Sync for SacnBufs {}

static BUFS: SacnBufs = SacnBufs {
    rx_meta: UnsafeCell::new([PacketMetadata::EMPTY; 4]),
    rx_data: UnsafeCell::new([0u8; MAX_PACKET_LEN]),
    tx_meta: UnsafeCell::new([PacketMetadata::EMPTY; 1]),
    tx_data: UnsafeCell::new([0u8; 64]),
};

pub(crate) struct Listener {
    socket: UdpSocket<'static>,
    network_stack: Stack<'static>,
    config: DmxConfig,
    multicast: Ipv4Address,
    last_value: Option<DmxValue>,
    dmx_value: &'static Signal<CriticalSectionRawMutex, DmxValue>,
}

impl Listener {
    pub(crate) fn new(
        network_stack: Stack<'static>,
        config: DmxConfig,
        dmx_value: &'static Signal<CriticalSectionRawMutex, DmxValue>,
    ) -> Self {
        let multicast = config.multicast_address();
        network_stack.join_multicast_group(multicast).ok();

        // SAFETY: only one Listener exists at a time; main drops the previous
        // Listener before calling new(), so these buffers have no live borrowers.
        let mut socket = unsafe {
            UdpSocket::new(
                network_stack,
                &mut *BUFS.rx_meta.get(),
                &mut *BUFS.rx_data.get(),
                &mut *BUFS.tx_meta.get(),
                &mut *BUFS.tx_data.get(),
            )
        };
        socket.bind(config.sacn_port()).ok();

        rprintln!(
            "sACN listener: address={} universe={} multicast={}:{}",
            config.address(),
            config.universe(),
            multicast,
            config.sacn_port()
        );

        Self {
            socket,
            network_stack,
            config,
            multicast,
            last_value: None,
            dmx_value,
        }
    }

    /// Runs forever, signaling `dmx_value` whenever the DMX value at the
    /// configured address changes. Handles packet timeouts by rejoining the
    /// multicast group internally.
    #[allow(
        clippy::large_stack_frames,
        reason = "pkt_buf (638 bytes) must be held across the recv_from await"
    )]
    pub(crate) async fn run(&mut self) -> ! {
        let mut pkt_buf = [0u8; MAX_PACKET_LEN];
        loop {
            match with_timeout(
                Duration::from_secs(UNIVERSE_TIMEOUT),
                self.socket.recv_from(&mut pkt_buf),
            )
            .await
            {
                Ok(Ok((n, _))) => {
                    if let Some(val) = parse_e131_slots(
                        &pkt_buf[..n],
                        self.config.universe(),
                        self.config.address(),
                    ) {
                        if Some(val) != self.last_value {
                            self.last_value = Some(val);
                            rprintln!(
                                "DMX ch {} = I:{} R:{} G:{} B:{} W:{}",
                                self.config.address(),
                                val.intensity(),
                                val.red(),
                                val.green(),
                                val.blue(),
                                val.white(),
                            );
                            self.dmx_value.signal(val);
                        }
                    }
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    rprintln!(
                        "no universe for {} seconds, rejoining multicast group",
                        UNIVERSE_TIMEOUT
                    );
                    self.network_stack
                        .leave_multicast_group(self.multicast)
                        .ok();
                    self.network_stack.join_multicast_group(self.multicast).ok();
                }
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.network_stack
            .leave_multicast_group(self.multicast)
            .ok();
        rprintln!("sACN listener destroyed");
    }
}

/// Extracts five consecutive DMX slots (I, R, G, B, W) from an E1.31 data packet.
/// `base_address` is the 1-indexed DMX address of the Intensity slot; Red through
/// White follow at base_address+1 through base_address+4.
/// Returns None if the packet is invalid, for a different universe, uses a
/// non-zero start code, or does not contain all five slots.
fn parse_e131_slots(pkt: &[u8], universe: u16, base_address: u16) -> Option<DmxValue> {
    if pkt.len() < START_CODE_OFFSET + 1 {
        return None;
    }
    if &pkt[ACN_ID_OFFSET..ACN_ID_OFFSET + ACN_ID.len()] != ACN_ID {
        return None;
    }
    if u32::from_be_bytes([pkt[ROOT_VECTOR_OFFSET], pkt[ROOT_VECTOR_OFFSET + 1], pkt[ROOT_VECTOR_OFFSET + 2], pkt[ROOT_VECTOR_OFFSET + 3]]) != ROOT_VECTOR {
        return None;
    }
    if u32::from_be_bytes([pkt[FRAMING_VECTOR_OFFSET], pkt[FRAMING_VECTOR_OFFSET + 1], pkt[FRAMING_VECTOR_OFFSET + 2], pkt[FRAMING_VECTOR_OFFSET + 3]]) != FRAMING_VECTOR {
        return None;
    }
    if u16::from_be_bytes([pkt[UNIVERSE_OFFSET], pkt[UNIVERSE_OFFSET + 1]]) != universe {
        return None;
    }
    if pkt[DMP_VECTOR_OFFSET] != DMP_VECTOR {
        return None;
    }
    if pkt[START_CODE_OFFSET] != DMX_NULL_START {
        return None;
    }

    let prop_count = u16::from_be_bytes([pkt[PROP_COUNT_OFFSET], pkt[PROP_COUNT_OFFSET + 1]]);
    let last_slot = base_address + DmxValue::LEN as u16 - 1;
    let base = START_CODE_OFFSET + base_address as usize;
    let last_offset = START_CODE_OFFSET + last_slot as usize;

    if last_slot >= prop_count || last_offset >= pkt.len() {
        return None;
    }
    let slots: [u8; DmxValue::LEN] = pkt[base..base + DmxValue::LEN].try_into().ok()?;
    Some(DmxValue::new(slots))
}
