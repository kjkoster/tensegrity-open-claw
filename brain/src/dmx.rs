//! The dmx-sender stage (SPARKLE.md §0.3): E1.31 sACN packet encoding and
//! multicast transmission. Formerly `sacn.rs`.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::{SystemTime, UNIX_EPOCH};

/// Generates a CID that is stable for the lifetime of the process.
pub fn new_cid() -> [u8; 16] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let pid = std::process::id();
    let mut cid = [0u8; 16];
    cid[0..4].copy_from_slice(&nanos.to_le_bytes());
    cid[4..8].copy_from_slice(&pid.to_le_bytes());
    cid
}

/// Encodes an E1.31 sACN data packet.
/// Layout per DESIGN.md Appendix A; validated against the ponytail's parse_e131_slots().
pub fn encode(universe: u16, sequence: u8, priority: u8, cid: &[u8; 16], slots: &[u8]) -> Vec<u8> {
    let n = slots.len();
    let total = 126 + n;
    let mut p = vec![0u8; total];

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
    p[113..115].copy_from_slice(&universe.to_be_bytes());

    // DMP layer: flags/len, vector, addr/data type, first prop addr, increment, property count, start code, slots
    let dmp_fl = 0x7000u16 | (total - 115) as u16;
    p[115..117].copy_from_slice(&dmp_fl.to_be_bytes());
    p[117] = 0x02;
    p[118] = 0xa1;
    p[121..123].copy_from_slice(&0x0001u16.to_be_bytes());
    p[123..125].copy_from_slice(&((n as u16) + 1).to_be_bytes());
    // p[125] = 0x00  (DMX start code, already zero)
    p[126..126 + n].copy_from_slice(slots);

    p
}

/// The sACN multicast group address for the given universe.
pub fn multicast_addr(universe: u16) -> Ipv4Addr {
    Ipv4Addr::new(239, 255, (universe >> 8) as u8, universe as u8)
}

/// Sends a packet to the sACN multicast group for the given universe.
pub fn send_multicast(socket: &UdpSocket, universe: u16, port: u16, packet: &[u8]) {
    let dest = SocketAddrV4::new(multicast_addr(universe), port);
    if let Err(e) = socket.send_to(packet, dest) {
        eprintln!("sACN send error: {e}");
    }
}
