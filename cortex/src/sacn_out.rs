//! The sACN sender: E1.31 packet encoding and unicast transmission.

use crate::dmx::BLACKOUT;
use std::net::{SocketAddrV4, UdpSocket};
use std::time::{SystemTime, UNIX_EPOCH};

/// E1.31 framing options bit (offset 112) that marks a stream as terminated. A
/// receiver honouring it drops this source immediately rather than waiting out the
/// 2.5 s network-data-loss timeout.
pub const STREAM_TERMINATED: u8 = 0x40;

/// The headers before the first slot: root, framing and DMP layers, and the start code.
const HEADER_LEN: usize = 126;

/// One encoded packet, sized for a full universe and carrying as many slots as the rig
/// spans, so encoding a frame never allocates.
pub struct Packet {
    bytes: [u8; HEADER_LEN + BLACKOUT.len()],
    len: usize,
}

impl Packet {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Generates a CID that is stable for the lifetime of the process.
pub fn new_cid() -> [u8; 16] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is before 1970")
        .subsec_nanos();
    let pid = std::process::id();
    let mut cid = [0u8; 16];
    cid[0..4].copy_from_slice(&nanos.to_le_bytes());
    cid[4..8].copy_from_slice(&pid.to_le_bytes());
    cid
}

/// Encodes an E1.31 sACN data packet. `options` is the framing options byte (offset
/// 112): 0 for a normal data frame, `STREAM_TERMINATED` for a clean stop.
/// Layout per ANSI E1.31.
pub fn encode(
    universe: u16,
    sequence: u8,
    priority: u8,
    options: u8,
    cid: &[u8; 16],
    slots: &[u8],
) -> Packet {
    let n = slots.len();
    let total = HEADER_LEN + n;
    let mut packet = Packet {
        bytes: [0; HEADER_LEN + BLACKOUT.len()],
        len: total,
    };
    let p = &mut packet.bytes;

    // Preamble
    p[1] = 0x10;

    // ACN Packet Identifier
    p[4..16].copy_from_slice(b"ASC-E1.17\0\0\0");

    // Root layer: flags/len, vector, CID
    let root_fl = 0x7000u16 | (total - 16) as u16;
    p[16..18].copy_from_slice(&root_fl.to_be_bytes());
    p[18..22].copy_from_slice(&0x0000_0004u32.to_be_bytes());
    p[22..38].copy_from_slice(cid);

    // Framing layer: flags/len, vector, source name, priority, sequence, universe
    let framing_fl = 0x7000u16 | (total - 38) as u16;
    p[38..40].copy_from_slice(&framing_fl.to_be_bytes());
    p[40..44].copy_from_slice(&0x0000_0002u32.to_be_bytes());
    p[44..49].copy_from_slice(b"brain");
    p[108] = priority;
    p[111] = sequence;
    p[112] = options;
    p[113..115].copy_from_slice(&universe.to_be_bytes());

    // DMP layer: flags/len, vector, addr/data type, first prop addr, increment, property count, start code, slots
    let dmp_fl = 0x7000u16 | (total - 115) as u16;
    p[115..117].copy_from_slice(&dmp_fl.to_be_bytes());
    p[117] = 0x02;
    p[118] = 0xa1;
    p[121..123].copy_from_slice(&0x0001u16.to_be_bytes());
    p[123..125].copy_from_slice(&((n as u16) + 1).to_be_bytes());
    // p[125] = 0x00  (DMX start code, already zero)
    p[HEADER_LEN..total].copy_from_slice(slots);

    packet
}

/// Encodes a stream-terminated frame for `universe`: a normal data packet carrying
/// zeroed slots with the `STREAM_TERMINATED` options bit set. Sent on shutdown so the
/// fixtures release the brain's source at once. The slot width matches the live stream so
/// every fixture — including the highest-addressed — finds its slots and honours the flag;
/// only the flag matters, not the contents.
///
/// The width comes from the rig rather than a constant: how many slots the patch spans is
/// decided in QLC+, not in this crate.
pub fn encode_release(
    universe: u16,
    sequence: u8,
    priority: u8,
    cid: &[u8; 16],
    dmx_slots: usize,
) -> Packet {
    encode(universe, sequence, priority, STREAM_TERMINATED, cid, &BLACKOUT[..dmx_slots])
}

/// Sends a packet to every recipient, by unicast.
///
/// Unicast, not the universe's multicast group: multicast over WiFi goes at the lowest basic
/// rate, unacknowledged, and arrives bunched, and a fixture that is not joined to the group
/// hears nothing. One send per recipient instead, each acknowledged by the access point.
///
/// A recipient that is off the network is no reason to stop the frame: the send fails or
/// goes nowhere, and the next frame tries again. A failure is logged once until a send to it
/// succeeds, so a fixture that is away does not fill the journal at frame rate. `failing` is
/// one bit per recipient, owned by the caller, so sending allocates nothing.
pub fn send_unicast(
    socket: &UdpSocket,
    recipients: &[SocketAddrV4],
    failing: &mut u32,
    packet: &[u8],
) {
    for (i, recipient) in recipients.iter().take(MAX_RECIPIENTS).enumerate() {
        let bit = 1u32 << i;
        match socket.send_to(packet, recipient) {
            Ok(_) if *failing & bit != 0 => {
                eprintln!("sacn: sending to {recipient} again");
                *failing &= !bit;
            }
            Ok(_) => {}
            Err(e) if *failing & bit == 0 => {
                eprintln!("sacn: send to {recipient}: {e}");
                *failing |= bit;
            }
            Err(_) => {}
        }
    }
}

/// The most fixtures the brain sends to by unicast: one bit each in `send_unicast`'s record of
/// which are failing. The brain refuses to start with more, rather than skip any.
pub const MAX_RECIPIENTS: usize = u32::BITS as usize;
