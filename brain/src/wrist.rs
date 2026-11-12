//! Wrist, as the brain hears it: its OSC, received where `clockwork.dot` has the brain's `osc`
//! port.
//!
//! Wrist sends its state bundle ten times a second, as `fixtures/wrist.oscquery.json` has it.
//! Only `/wrist/pluck` is read so far: whether a pluck can be started. Its address, type tag
//! and values are the file's, generated into `osc::wrist`; only OSC's own framing, bundles and
//! padded strings, is decoded here. What the brain asks of Wrist in return is the show's, in
//! `show`.

use cortex::latest::{LatestTx, LatestRx, latest};
use std::io::ErrorKind;
use std::net::{SocketAddrV4, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

/// `/wrist/pluck`: whether a pluck can be started. `Can` in Hold; `Busy` from Pluck's start
/// until the shaft is back where it began; `Cannot` at every other time.
pub use crate::osc::wrist::WristPluck as Pluck;

/// The longest wait between attempts to bind, while the access point's address is not up.
const BIND_RETRY_MAX_S: u64 = 30;
/// Larger than any datagram Wrist sends: its packets are built in 1024 bytes.
const MAX_PACKET_LEN: usize = 1536;
/// A gap in `/wrist/pluck` this long is logged when it ends: five of Wrist's state bundles.
const GAP_LOGGED: Duration = Duration::from_millis(500);

/// `/wrist/pluck` as last heard, and when; what stale means is the show's to say.
pub type Heard = Option<(Pluck, Instant)>;

/// Starts the receiver on its own thread and returns the reader of what it hears.
pub fn spawn(bind: SocketAddrV4) -> LatestRx<Heard> {
    let (reports, reader) = latest(None);
    thread::Builder::new()
        .name("wrist-osc".into())
        .spawn(move || {
            cortex::sched::background();
            let mut backoff_s = 1u64;
            loop {
                // The address not being there yet is the access point not being up, which the
                // backoff waits out. Any other bind failure, such as a second brain holding the
                // port, will not go away by waiting.
                match UdpSocket::bind(bind) {
                    Ok(socket) => {
                        eprintln!("wrist: listening for OSC on {bind}");
                        backoff_s = 1;
                        receive(&socket, &reports);
                    }
                    Err(e) if e.kind() == ErrorKind::AddrNotAvailable => {
                        eprintln!("wrist: bind {bind}: {e}, again in {backoff_s}s");
                    }
                    Err(e) => panic!("wrist: bind {bind}: {e}"),
                }
                thread::sleep(Duration::from_secs(backoff_s));
                backoff_s = (backoff_s * 2).min(BIND_RETRY_MAX_S);
            }
        })
        .expect("failed to spawn the wrist-osc thread");
    reader
}

/// Receives until the socket fails.
fn receive(socket: &UdpSocket, reports: &LatestTx<Heard>) {
    let mut buffer = [0u8; MAX_PACKET_LEN];
    let mut heard = None;
    // When `/wrist/pluck` last came, and what else came since: bundles without it, and single
    // messages. Wrist sends both ten times a second, so a gap with the others still arriving is
    // Wrist sending and the link up, and one with nothing is the link or Wrist gone quiet.
    let mut last_pluck: Option<Instant> = None;
    let mut bundles_without = 0u32;
    let mut messages = 0u32;
    loop {
        let len = match socket.recv(&mut buffer) {
            Ok(len) => len,
            Err(e) => {
                eprintln!("wrist: receive failed: {e}");
                return;
            }
        };
        let packet = &buffer[..len];
        let Some(pluck) = pluck_in(packet) else {
            if packet.starts_with(b"#bundle\0") {
                bundles_without += 1;
            } else {
                messages += 1;
            }
            continue;
        };
        let now = Instant::now();
        if let Some(gap) = last_pluck.map(|at| now - at)
            && gap >= GAP_LOGGED
        {
            eprintln!(
                "wrist: /wrist/pluck again after {} ms without; meanwhile {bundles_without} \
                 bundles without it and {messages} other messages",
                gap.as_millis()
            );
        }
        last_pluck = Some(now);
        bundles_without = 0;
        messages = 0;
        if heard != Some(pluck) {
            eprintln!("wrist: pluck {}", pluck.word());
            heard = Some(pluck);
        }
        reports.publish(Some((pluck, now)));
    }
}

/// `/wrist/pluck` in an OSC packet, a message or a bundle of them, if it is there and well
/// formed.
fn pluck_in(packet: &[u8]) -> Option<Pluck> {
    if let Some(mut elements) = packet.strip_prefix(b"#bundle\0") {
        // The time tag, which Wrist always sends as "immediately".
        elements = elements.get(8..)?;
        while elements.len() >= 4 {
            let size = u32::from_be_bytes(elements[..4].try_into().unwrap()) as usize;
            let element = elements.get(4..4 + size)?;
            if let Some(pluck) = pluck_in(element) {
                return Some(pluck);
            }
            elements = &elements[4 + size..];
        }
        return None;
    }
    let mut at = 0;
    if osc_string(packet, &mut at)? != Pluck::PATH || osc_string(packet, &mut at)? != Pluck::TAG {
        return None;
    }
    Pluck::from_word(osc_string(packet, &mut at)?)
}

/// The OSC string at `at`, moving `at` past it and its padding to four bytes.
fn osc_string<'a>(packet: &'a [u8], at: &mut usize) -> Option<&'a str> {
    let rest = packet.get(*at..)?;
    let len = rest.iter().position(|&b| b == 0)?;
    *at += (len / 4 + 1) * 4;
    std::str::from_utf8(&rest[..len]).ok()
}
