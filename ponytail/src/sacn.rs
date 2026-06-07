use core::cell::UnsafeCell;
use embassy_net::{
    Ipv4Address, Stack,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, with_timeout};
use rtt_target::rprintln;

use crate::models::DmxConfig;

const UNIVERSE_TIMEOUT: u64 = 30; // seconds

// ACN Packet Identifier at bytes 4..16 of every E1.31 packet.
const ACN_ID: &[u8; 12] = b"ASC-E1.17\0\0\0";

// Socket ring-buffer storage. Lives 'static so UdpSocket<'static> can borrow it.
// Only one Listener exists at a time; main enforces this by dropping the old one
// before creating a new one.
struct SacnBufs {
    rx_meta: UnsafeCell<[PacketMetadata; 4]>,
    rx_data: UnsafeCell<[u8; 638]>,
    tx_meta: UnsafeCell<[PacketMetadata; 1]>,
    tx_data: UnsafeCell<[u8; 64]>,
}
// SAFETY: single-task access guaranteed by the one-Listener-at-a-time invariant.
unsafe impl Sync for SacnBufs {}

static BUFS: SacnBufs = SacnBufs {
    rx_meta: UnsafeCell::new([PacketMetadata::EMPTY; 4]),
    rx_data: UnsafeCell::new([0u8; 638]),
    tx_meta: UnsafeCell::new([PacketMetadata::EMPTY; 1]),
    tx_data: UnsafeCell::new([0u8; 64]),
};

pub(crate) struct Listener {
    socket: UdpSocket<'static>,
    network_stack: Stack<'static>,
    config: DmxConfig,
    multicast: Ipv4Address,
    last_value: Option<u8>,
    dmx_value: &'static Signal<CriticalSectionRawMutex, u8>,
}

impl Listener {
    pub(crate) fn new(
        network_stack: Stack<'static>,
        config: DmxConfig,
        dmx_value: &'static Signal<CriticalSectionRawMutex, u8>,
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
        let mut pkt_buf = [0u8; 638];
        loop {
            match with_timeout(
                Duration::from_secs(UNIVERSE_TIMEOUT),
                self.socket.recv_from(&mut pkt_buf),
            )
            .await
            {
                Ok(Ok((n, _))) => {
                    if let Some(val) = parse_e131_slot(&pkt_buf[..n], self.config.universe(), self.config.address()) {
                        if Some(val) != self.last_value {
                            self.last_value = Some(val);
                            rprintln!("DMX ch {} = {}", self.config.address(), val);
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
                    self.network_stack.leave_multicast_group(self.multicast).ok();
                    self.network_stack.join_multicast_group(self.multicast).ok();
                }
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.network_stack.leave_multicast_group(self.multicast).ok();
        rprintln!("sACN listener destroyed");
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
