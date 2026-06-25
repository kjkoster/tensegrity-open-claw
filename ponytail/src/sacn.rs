use core::cell::UnsafeCell;
use core::cmp::Reverse;
use embassy_net::{
    IpAddress, Stack,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Instant, with_timeout};
use rtt_target::rprintln;

use crate::models::{DmxConfig, DmxSender, DmxValue};

const UNIVERSE_TIMEOUT: u64 = 5; // seconds

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

// Source-arbitration fields. The root-layer CID identifies the sender; the
// framing-layer priority and options byte rank it and signal a clean stop.
const CID_OFFSET: usize = 22;
const CID_LEN: usize = 16;
const PRIORITY_OFFSET: usize = 108;
const OPTIONS_OFFSET: usize = 112;
const STREAM_TERMINATED: u8 = 0x40; // options bit: this source is releasing the stream

// A source is dropped this long after its last packet — the E1.31 network-data-loss
// timeout — so a silent higher-priority source yields to a lower one automatically.
const SOURCE_TIMEOUT: Duration = Duration::from_millis(2500);
// Distinct senders we track at once. Normally just the brain plus one console; a few
// more absorbs stray senders without unbounded storage.
const MAX_SOURCES: usize = 4;

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

/// A parsed E1.31 data packet for our universe: who sent it, at what priority, whether
/// it cleanly terminated the stream, and the fixture's slot values.
struct E131Frame {
    cid: [u8; CID_LEN],
    priority: u8,
    terminated: bool,
    value: DmxValue,
}

/// One sACN source we have heard from, keyed by its 16-byte CID. The IP is kept only
/// for logging — arbitration is by CID, which survives a source changing address.
struct Source {
    cid: [u8; CID_LEN],
    ip: IpAddress,
    priority: u8,
    last_seen: Instant,
}

/// Per-universe source arbitration. Tracks the live senders and names the
/// one the fixture should obey: highest priority wins, ties broken by CID so the choice
/// is stable and never flaps between equal sources. Entries expire after
/// `SOURCE_TIMEOUT` or are released at once on a stream-terminated packet. This is what
/// lets a console (priority 200) override the brain (100) live, and the brain reclaim
/// control the moment the console stops — independent of whether the brain is healthy.
struct SourceTable {
    sources: heapless::Vec<Source, MAX_SOURCES>,
}

impl SourceTable {
    fn new() -> Self {
        Self {
            sources: heapless::Vec::new(),
        }
    }

    /// Drop sources not heard from within `SOURCE_TIMEOUT`.
    fn expire(&mut self, now: Instant) {
        self.sources.retain(|s| {
            let alive = now.duration_since(s.last_seen) < SOURCE_TIMEOUT;
            if !alive {
                rprintln!("sACN source timed out: {} priority {}", s.ip, s.priority);
            }
            alive
        });
    }

    /// Record a packet from `cid` at `priority`: refresh an existing source or insert a
    /// new one. A new source past `MAX_SOURCES` is dropped; an expiring entry frees a
    /// slot within 2.5 s, and the table only fills if several rogue senders are live at
    /// once — which a closed control network does not produce.
    fn observe(&mut self, cid: [u8; CID_LEN], ip: IpAddress, priority: u8, now: Instant) {
        if let Some(src) = self.sources.iter_mut().find(|s| s.cid == cid) {
            src.ip = ip;
            src.priority = priority;
            src.last_seen = now;
            return;
        }
        let source = Source {
            cid,
            ip,
            priority,
            last_seen: now,
        };
        if self.sources.push(source).is_ok() {
            rprintln!("sACN source added: {} priority {}", ip, priority);
        }
    }

    /// Forget `cid` — it sent a stream-terminated packet.
    fn release(&mut self, cid: &[u8; CID_LEN]) {
        if let Some(i) = self.sources.iter().position(|s| s.cid == *cid) {
            let s = self.sources.swap_remove(i);
            rprintln!("sACN source terminated: {} priority {}", s.ip, s.priority);
        }
    }

    /// True if `cid` is the source the fixture should currently obey: the highest
    /// priority among live sources, ties broken by the smaller CID.
    fn is_winner(&self, cid: &[u8; CID_LEN]) -> bool {
        self.sources
            .iter()
            .max_by_key(|s| (s.priority, Reverse(s.cid)))
            .is_some_and(|w| &w.cid == cid)
    }
}

pub(crate) struct Listener {
    socket: UdpSocket<'static>,
    config: DmxConfig,
    last_value: Option<DmxValue>,
    dmx_value: DmxSender,
    sources: SourceTable,
}

impl Listener {
    pub(crate) fn new(
        network_stack: Stack<'static>,
        config: DmxConfig,
        dmx_value: DmxSender,
    ) -> Self {
        let multicast = config.multicast_address();
        if let Err(e) = network_stack.join_multicast_group(multicast) {
            rprintln!("multicast join error: {:?}", e);
        }

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
        if let Err(e) = socket.bind(config.sacn_port()) {
            rprintln!("socket bind error: {:?}", e);
        }

        rprintln!("dmx address:    {}", config.address(),);
        rprintln!("dmx universe:   {}", config.universe(),);
        rprintln!("sacn multicast: {}:{}", multicast, config.sacn_port());

        Self {
            socket,
            config,
            last_value: None,
            dmx_value,
            sources: SourceTable::new(),
        }
    }

    /// Signals `dmx_value` whenever the DMX value at the configured address changes.
    /// Returns on timeout so the caller can drop and recreate the Listener, which
    /// rebinds the socket and sends a fresh IGMP join. Rejoining the multicast group
    /// in-place after a timeout is not sufficient: the socket may be stale from the
    /// WiFi disconnect and embassy_net may not deliver packets to it after reconnect.
    #[allow(
        clippy::large_stack_frames,
        reason = "pkt_buf (638 bytes) must be held across the recv_from await"
    )]
    pub(crate) async fn run(&mut self) {
        let mut pkt_buf = [0u8; MAX_PACKET_LEN];
        loop {
            match with_timeout(
                Duration::from_secs(UNIVERSE_TIMEOUT),
                self.socket.recv_from(&mut pkt_buf),
            )
            .await
            {
                Ok(Ok((n, meta))) => {
                    if let Some(frame) = parse_e131_slots(
                        &pkt_buf[..n],
                        self.config.universe(),
                        self.config.address(),
                    ) {
                        crate::metrics::record_universe();
                        let now = Instant::now();
                        self.sources.expire(now);

                        if frame.terminated {
                            // A clean stop from this source: forget it now so a
                            // lower-priority source (or the held value) takes over
                            // without waiting out the 2.5 s timeout.
                            self.sources.release(&frame.cid);
                        } else {
                            self.sources
                                .observe(frame.cid, meta.endpoint.addr, frame.priority, now);
                            // Obey only the highest-priority live source; a source
                            // a higher one is overriding has its slots ignored.
                            if self.sources.is_winner(&frame.cid)
                                && Some(frame.value) != self.last_value
                            {
                                self.last_value = Some(frame.value);
                                self.dmx_value.send(frame.value);
                                crate::metrics::record_change();
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    rprintln!("recv_from error: {:?}", e);
                }
                Err(_) => {
                    rprintln!(
                        "no universe for {} seconds, recreating socket",
                        UNIVERSE_TIMEOUT
                    );
                    return;
                }
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        rprintln!("sACN listener destroyed");
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

/// Parses an E1.31 data packet for our universe into an [`E131Frame`]: the source CID
/// and priority, the stream-terminated flag, and the fixture's six consecutive DMX
/// slots (I, R, G, B, W, Gobo). `base_address` is the 1-indexed DMX address of the
/// Intensity slot; the other five channels follow through `base_address + 5`.
/// Returns None if the packet is invalid, for a different universe, uses a non-zero
/// start code, or does not contain all six slots.
fn parse_e131_slots(pkt: &[u8], universe: u16, base_address: u16) -> Option<E131Frame> {
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
    let cid: [u8; CID_LEN] = pkt[CID_OFFSET..CID_OFFSET + CID_LEN].try_into().ok()?;
    Some(E131Frame {
        cid,
        priority: pkt[PRIORITY_OFFSET],
        terminated: pkt[OPTIONS_OFFSET] & STREAM_TERMINATED != 0,
        value: DmxValue::new(slots),
    })
}
