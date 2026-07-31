//! The sACN receive stage: an external E1.31 console taking the universe away from the
//! internal engine.
//!
//! `dmx.rs` is the send side; this is its counterpart. The packet layout and the source
//! arbitration are a port of the receiver the ponytail firmware used, which is out of the
//! project now but recoverable — `git show 22dd9a0:ponytail/src/sacn.rs`. That one was
//! `no_std` and extracted a single fixture's slots; this one is `std` and takes the whole
//! universe, but the offsets, the 2.5 s timeout and the source table carry across unchanged.
//!
//! Ownership is whole-universe and priority-gated. A source must transmit **strictly above**
//! the brain's own `SACN_PRIORITY` to take over, and when it does it owns every slot — no
//! HTP or LTP merge, no per-fixture claims. Merging is not a missing feature but a wrong
//! one here: a fixture's channels are positions, wheel indices and modes, and max() of two
//! wheel positions is an index neither source asked for.
//!
//! This module only decides which external source is a *candidate*. Whether that candidate
//! is actually driving is decided once per frame in the orchestrator, so a single decision
//! feeds both the wire and the network.
//!
//! The socket binds one address — the Pi's WireGuard address — rather than the wildcard, so
//! the brain's own multicast is never delivered back to its own receiver.

use crate::clock;
use crate::config as cfg;
use crate::dmx::STREAM_TERMINATED;
use crate::latest::LatestTx;
use std::error::Error;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use zihatec_rs_485_dmx::SLOTS;

// ── E1.31 packet layout (ANSI E1.31-2018) ────────────────────────────────────
const ACN_ID: &[u8; 12] = b"ASC-E1.17\0\0\0";
const ACN_ID_OFFSET: usize = 4;
const ROOT_VECTOR_OFFSET: usize = 18;
/// Root vector 0x04 is E1.31 data. Universe Discovery uses the extended root 0x08, so this
/// one comparison drops discovery traffic before anything else looks at it.
const ROOT_VECTOR: u32 = 0x0000_0004;
const CID_OFFSET: usize = 22;
pub const CID_LEN: usize = 16;
const FRAMING_VECTOR_OFFSET: usize = 40;
/// Framing vector 0x02 is a data packet; synchronization packets carry 0x01 and are dropped
/// here. We are a non-synchronizing receiver — legal under E1.31, and QLC+ does not use
/// sync — so the synchronization address and the force-synchronization option bit are
/// ignored on purpose.
const FRAMING_VECTOR: u32 = 0x0000_0002;
const PRIORITY_OFFSET: usize = 108;
const SEQUENCE_OFFSET: usize = 111;
const OPTIONS_OFFSET: usize = 112;
const UNIVERSE_OFFSET: usize = 113;
const DMP_VECTOR_OFFSET: usize = 117;
const DMP_VECTOR: u8 = 0x02;
const PROP_COUNT_OFFSET: usize = 123;
const START_CODE_OFFSET: usize = 125;
const DMX_NULL_START: u8 = 0x00;

/// Options bit 7: the source marks this frame as preview/blind data. It must never reach
/// real fixtures, so it is dropped rather than relayed.
const PREVIEW_DATA: u8 = 0x80;

/// Slot data begins immediately after the start code.
const HEADER_LEN: usize = START_CODE_OFFSET + 1;
/// Largest legal E1.31 data packet: header plus a full universe.
const MAX_PACKET_LEN: usize = HEADER_LEN + SLOTS;

/// E1.31 §6.7.2 out-of-order window. A packet whose sequence is behind the last one by less
/// than this is a straggler and is discarded; further behind than this, the source is taken
/// to have restarted and the packet is accepted.
const SEQUENCE_WINDOW: i8 = -20;

/// What the receive stage publishes to the orchestrator: a full universe from an external
/// source, plus who sent it and when. Always present — [`Takeover::idle`] is the
/// nobody-is-driving value — so the orchestrator reads one snapshot per frame and never has
/// to branch on absence.
#[derive(Clone)]
pub struct Takeover {
    pub slots: [u8; SLOTS],
    pub cid: [u8; CID_LEN],
    /// Kept for the log only; arbitration is by CID, which survives a source changing address.
    pub source: Ipv4Addr,
    pub priority: u8,
    pub timestamp_us: u64,
    /// Cleared by a stream-terminated frame while the identity above is kept, so the
    /// release log can still name who was driving.
    pub live: bool,
}

impl Takeover {
    /// The no-external-source value. Priority 0 can never clear `SACN_PRIORITY`, so this is
    /// inert under [`Takeover::in_force`] whatever else happens to it.
    pub fn idle() -> Self {
        Self {
            slots: [0u8; SLOTS],
            cid: [0u8; CID_LEN],
            source: Ipv4Addr::UNSPECIFIED,
            priority: 0,
            timestamp_us: 0,
            live: false,
        }
    }

    /// Whether this snapshot owns the universe right now — the whole arbitration rule, in
    /// one place, evaluated once per frame so the wire and the network can never be driven
    /// from different decisions.
    ///
    /// The receive thread expires sources too, on its own read timeout. This check is not
    /// redundant with it: evaluating ownership here means the decision holds even if that
    /// thread stalls or dies, so a wedged receiver hands the rig back rather than pinning it
    /// to whatever the last console said.
    pub fn in_force(&self, now_us: u64) -> bool {
        self.live
            && self.priority > cfg::SACN_PRIORITY
            && now_us.saturating_sub(self.timestamp_us) < cfg::SACN_SOURCE_TIMEOUT_US
    }

    /// Identity for a takeover or release line. Called only on the edges, never per frame.
    pub fn describe(&self) -> String {
        let cid: String = self.cid.iter().map(|b| format!("{b:02x}")).collect();
        format!("{} cid={cid} priority={}", self.source, self.priority)
    }
}

/// One sACN source we have heard from, keyed by its 16-byte CID.
struct Source {
    cid: [u8; CID_LEN],
    ip: Ipv4Addr,
    priority: u8,
    last_seen_us: u64,
    last_sequence: u8,
}

/// Per-universe source arbitration: tracks the live external senders and names the one to
/// obey. Highest priority wins, ties broken by the smaller CID so the choice is stable and
/// never flaps between equal sources. Entries expire after the data-loss timeout or are
/// released at once on a stream-terminated packet.
///
/// The brain itself is deliberately *not* in this table. It is not one candidate among
/// several — it is the floor the winner has to clear, which is a fixed comparison against
/// `SACN_PRIORITY` rather than a row that could be outranked by CID.
struct SourceTable {
    sources: Vec<Source>,
}

impl SourceTable {
    fn new() -> Self {
        Self { sources: Vec::new() }
    }

    /// Drop sources not heard from within the data-loss timeout.
    fn expire(&mut self, now_us: u64) {
        self.sources.retain(|s| {
            let alive = now_us.saturating_sub(s.last_seen_us) < cfg::SACN_SOURCE_TIMEOUT_US;
            if !alive {
                eprintln!("sacn: source timed out: {} priority {}", s.ip, s.priority);
            }
            alive
        });
    }

    /// Record a packet: refresh an existing source or insert a new one. A new source past
    /// `SACN_MAX_SOURCES` is dropped — an expiring entry frees a slot within the timeout,
    /// and the table only fills if several rogue senders are live at once, which a closed
    /// control network does not produce.
    fn observe(&mut self, cid: [u8; CID_LEN], ip: Ipv4Addr, priority: u8, sequence: u8, now_us: u64) {
        if let Some(source) = self.sources.iter_mut().find(|s| s.cid == cid) {
            source.ip = ip;
            source.priority = priority;
            source.last_seen_us = now_us;
            source.last_sequence = sequence;
            return;
        }
        if self.sources.len() < cfg::SACN_MAX_SOURCES {
            self.sources.push(Source {
                cid,
                ip,
                priority,
                last_seen_us: now_us,
                last_sequence: sequence,
            });
            eprintln!("sacn: source added: {ip} priority {priority}");
        }
    }

    /// Forget a source that sent a stream-terminated packet.
    fn release(&mut self, cid: &[u8; CID_LEN]) {
        if let Some(i) = self.sources.iter().position(|s| s.cid == *cid) {
            let source = self.sources.swap_remove(i);
            eprintln!(
                "sacn: source terminated: {} priority {}",
                source.ip, source.priority
            );
        }
    }

    /// Whether this packet's sequence number should be honoured (E1.31 §6.7.2): discard a
    /// straggler that arrived after a newer one, but accept a big jump backwards, which is
    /// a source that restarted rather than a reordering.
    fn in_sequence(&self, cid: &[u8; CID_LEN], sequence: u8) -> bool {
        match self.sources.iter().find(|s| s.cid == *cid) {
            None => true,
            Some(source) => {
                let delta = sequence.wrapping_sub(source.last_sequence) as i8;
                delta > 0 || delta <= SEQUENCE_WINDOW
            }
        }
    }

    /// The source to obey: highest priority among the live ones, ties broken by smaller CID.
    fn is_winner(&self, cid: &[u8; CID_LEN]) -> bool {
        self.sources
            .iter()
            .max_by_key(|s| (s.priority, std::cmp::Reverse(s.cid)))
            .is_some_and(|winner| &winner.cid == cid)
    }
}

/// The arbitration fields of an accepted packet. Slot bytes are deliberately not copied
/// here: most packets the socket sees are the brain's own multicast looping back, and those
/// are rejected on CID before the 512-byte copy is paid for.
struct Header {
    cid: [u8; CID_LEN],
    priority: u8,
    sequence: u8,
    terminated: bool,
    /// Level slots this packet actually carries, already clamped to the datagram and to 512.
    slots: usize,
}

/// Big-endian field readers. `parse_header` length-checks the packet first, so every offset
/// is in range and the slice is the exact width — `try_into` cannot fail.
fn be_u16(pkt: &[u8], off: usize) -> u16 {
    u16::from_be_bytes(pkt[off..off + 2].try_into().unwrap())
}

fn be_u32(pkt: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(pkt[off..off + 4].try_into().unwrap())
}

/// Validates an E1.31 data packet for our universe and returns its arbitration fields.
/// `None` for anything that is not level data we may act on.
fn parse_header(pkt: &[u8], universe: u16) -> Option<Header> {
    if pkt.len() < HEADER_LEN {
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
    if pkt[DMP_VECTOR_OFFSET] != DMP_VECTOR {
        return None;
    }
    if be_u16(pkt, UNIVERSE_OFFSET) != universe {
        return None;
    }

    let options = pkt[OPTIONS_OFFSET];
    if options & PREVIEW_DATA != 0 {
        return None;
    }

    // Only the null start code carries levels. An alternate start code is a different
    // protocol sharing the universe — 0xDD per-address-priority is the one seen in the
    // wild, and reading its priority bytes as levels would drive the rig from a table of
    // priorities.
    if pkt[START_CODE_OFFSET] != DMX_NULL_START {
        return None;
    }

    // The property count includes the start code, so it is one more than the slot count.
    // Trust neither it nor the datagram alone: take the smaller, and never more than a
    // universe.
    let declared = be_u16(pkt, PROP_COUNT_OFFSET).saturating_sub(1) as usize;

    Some(Header {
        cid: pkt[CID_OFFSET..CID_OFFSET + CID_LEN].try_into().ok()?,
        priority: pkt[PRIORITY_OFFSET],
        sequence: pkt[SEQUENCE_OFFSET],
        terminated: options & STREAM_TERMINATED != 0,
        slots: declared.min(pkt.len() - HEADER_LEN).min(SLOTS),
    })
}

/// Copies an accepted packet's levels into a full universe. Slots the source does not carry
/// read as zero, not as the engine's last value: a takeover is whole-universe, so a short
/// frame means those fixtures are commanded dark. Holding the engine there would be exactly
/// the per-fixture merge this design refuses.
fn slots_of(pkt: &[u8], count: usize) -> [u8; SLOTS] {
    let mut slots = [0u8; SLOTS];
    slots[..count].copy_from_slice(&pkt[HEADER_LEN..HEADER_LEN + count]);
    slots
}

/// Spawns the sACN receive stage on its own thread and returns immediately.
///
/// `recv_from` blocks indefinitely — and no console at all is the normal state of this rig,
/// so the block is unbounded rather than occasional. It cannot run on the single-threaded
/// embassy executor without stalling the frame loop, and embassy has no reactor for `std`
/// sockets. Same shape as audio capture: a blocking producer feeding the frame loop through
/// a latest-value seam.
pub fn spawn_receiver(own_cid: [u8; 16], publisher: LatestTx<Takeover>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("sacn-in".into())
        .spawn(move || {
            let mut backoff_s = 1u64;
            loop {
                let started = clock::now_us();
                if let Err(e) = receive(&own_cid, &publisher) {
                    eprintln!("sacn: receive failed: {e}");
                }
                // Hand the universe back now rather than letting the timeout do it: we can
                // no longer see the stream, so we can no longer vouch for it.
                publisher.publish(Takeover::idle());

                if clock::now_us().saturating_sub(started) > 60_000_000 {
                    backoff_s = 1;
                }
                eprintln!(
                    "sacn: rebinding :{} in {backoff_s}s (internal engine keeps driving)",
                    cfg::SACN_PORT
                );
                thread::sleep(Duration::from_secs(backoff_s));
                backoff_s = (backoff_s * 2).min(cfg::SACN_BIND_RETRY_MAX_S);
            }
        })
        .expect("failed to spawn sacn-in thread")
}

/// Binds, joins, and runs the receive loop until an unrecoverable socket error. Only
/// returns on error; the caller rebinds with backoff.
fn receive(own_cid: &[u8; 16], publisher: &LatestTx<Takeover>) -> Result<(), Box<dyn Error>> {
    // One address, no multicast joins: see SACN_BIND_ADDRESS. A failure here is normally the
    // tunnel not being up yet, which the caller's backoff handles.
    let socket = UdpSocket::bind(SocketAddrV4::new(cfg::SACN_BIND_ADDRESS, cfg::SACN_PORT))?;
    eprintln!(
        "sacn: listening on {}:{} universe {} — takeover above priority {}",
        cfg::SACN_BIND_ADDRESS,
        cfg::SACN_PORT,
        cfg::UNIVERSE,
        cfg::SACN_PRIORITY,
    );

    // Without a read timeout this loop parks in recv_from forever once a source goes quiet,
    // and a source going quiet is exactly what the expiry check exists to notice. Waking at
    // a quarter of the data-loss timeout bounds detection at 1.25× that timeout while
    // leaving an idle rig essentially asleep.
    socket.set_read_timeout(Some(Duration::from_micros(cfg::SACN_SOURCE_TIMEOUT_US / 4)))?;

    let mut pkt = [0u8; MAX_PACKET_LEN];
    let mut sources = SourceTable::new();

    loop {
        let received = match socket.recv_from(&mut pkt) {
            Ok(received) => Some(received),
            // A quiet interval, not a failure. Both kinds appear because platforms disagree
            // on which one a receive timeout raises.
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => None,
            Err(e) => return Err(e.into()),
        };

        // Sweep on every wake, and *before* any of the filters below can skip past it. A
        // packet having arrived is no evidence that the source which timed out is the one
        // that sent it, and with a quiet tunnel the read timeout is the only thing that
        // wakes us at all.
        let now = clock::now_us();
        sources.expire(now);

        let Some((n, from)) = received else { continue };
        let SocketAddr::V4(from) = from else { continue };
        let Some(header) = parse_header(&pkt[..n], cfg::UNIVERSE) else {
            continue;
        };

        // Belt and braces. Binding one unicast address already means the brain's own
        // multicast is never delivered here, but that is a property of the bind address and
        // this stays correct if it is ever widened.
        if header.cid == *own_cid {
            continue;
        }

        // A clean stop hands the rig back at once instead of waiting out the timeout. The
        // identity is kept in the published value so the orchestrator's release line can
        // still say who it was.
        if header.terminated {
            sources.release(&header.cid);
            publisher.publish(Takeover {
                slots: [0u8; SLOTS],
                cid: header.cid,
                source: *from.ip(),
                priority: header.priority,
                timestamp_us: now,
                live: false,
            });
            continue;
        }

        if !sources.in_sequence(&header.cid, header.sequence) {
            continue;
        }
        sources.observe(header.cid, *from.ip(), header.priority, header.sequence, now);

        // Two gates, in order. First the table: among external sources, only the winner is
        // heard. Then the floor: the winner takes the universe only if it is strictly above
        // the brain's own priority. Strict is load-bearing — every sACN source ships
        // defaulted to 100, so a laptop joining the network with a live universe would
        // otherwise seize the rig.
        if sources.is_winner(&header.cid) && header.priority > cfg::SACN_PRIORITY {
            publisher.publish(Takeover {
                slots: slots_of(&pkt[..n], header.slots),
                cid: header.cid,
                source: *from.ip(),
                priority: header.priority,
                timestamp_us: now,
                live: true,
            });
        }
    }
}
