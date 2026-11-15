//! Wrist, as the brain hears it: its OSC, received where `clockwork.dot` has the brain's `osc`
//! port.
//!
//! Wrist sends its state bundle ten times a second, as `fixtures/wrist.oscquery.json` has it.
//! Three of its addresses are read: `/wrist/pluck`, whether a pluck can be started;
//! `/wrist/mode`; and `/wrist/run/phase`, Run's φ. Their addresses, type tags and values are
//! the file's, generated into `osc::wrist`; OSC's own framing is `cortex::osc`'s. What the
//! brain asks of Wrist in return is the show's, in `show`.

use cortex::latest::{LatestRx, LatestTx, latest};
use cortex::osc::{self, Value};
use std::io::ErrorKind;
use std::net::{SocketAddrV4, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

/// `/wrist/mode`: the mode Wrist is in, whoever sent it there.
pub use crate::osc::wrist::WristMode as Mode;
/// `/wrist/pluck`: whether a pluck can be started. `Can` in Hold; `Busy` from Pluck's start
/// until the shaft is back where it began; `Cannot` at every other time.
pub use crate::osc::wrist::WristPluck as Pluck;
use crate::osc::wrist::WristRunPhase as RunPhase;

/// The longest wait between attempts to bind, while the access point's address is not up.
const BIND_RETRY_MAX_S: u64 = 30;
/// Larger than any datagram Wrist sends: its packets are built in 1472 bytes.
const MAX_PACKET_LEN: usize = 1536;
/// A gap in the state bundle this long is logged when it ends: five of them.
const GAP_LOGGED: Duration = Duration::from_millis(500);

/// What one state bundle says, of what the brain reads.
#[derive(Clone, Copy)]
pub struct Report {
    pub pluck: Pluck,
    pub mode: Mode,
    /// Run's φ, in radians; NaN outside Run.
    pub phase: f32,
}

/// The last state bundle, and when it came; what stale means is the show's to say.
pub type Heard = Option<(Report, Instant)>;

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
    let mut heard_pluck = None;
    let mut heard_mode = None;
    // When the state bundle last came, and what else came since: other bundles, and single
    // messages. Wrist sends both ten times a second, so a gap with the others still arriving is
    // Wrist sending and the link up, and one with nothing is the link or Wrist gone quiet.
    let mut last_report: Option<Instant> = None;
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
        let Some(report) = report_in(packet) else {
            if packet.starts_with(b"#bundle\0") {
                bundles_without += 1;
            } else {
                messages += 1;
            }
            continue;
        };
        let now = Instant::now();
        if let Some(gap) = last_report.map(|at| now - at)
            && gap >= GAP_LOGGED
        {
            eprintln!(
                "wrist: state bundle again after {} ms without; meanwhile {bundles_without} \
                 other bundles and {messages} other messages",
                gap.as_millis()
            );
        }
        last_report = Some(now);
        bundles_without = 0;
        messages = 0;
        if heard_pluck != Some(report.pluck) {
            eprintln!("wrist: pluck {}", report.pluck.word());
            heard_pluck = Some(report.pluck);
        }
        if heard_mode != Some(report.mode) {
            eprintln!("wrist: mode {}", report.mode.word());
            heard_mode = Some(report.mode);
        }
        reports.publish(Some((report, now)));
    }
}

/// The state bundle's report, if `packet` is a bundle holding all three addresses, well formed.
fn report_in(packet: &[u8]) -> Option<Report> {
    if !packet.starts_with(b"#bundle\0") {
        return None;
    }
    let (mut pluck, mut mode, mut phase) = (None, None, None);
    for (address, values) in osc::decode(packet).ok()? {
        match (address.as_str(), values.as_slice()) {
            (Pluck::PATH, [Value::Text(word)]) => pluck = Pluck::from_word(word),
            (Mode::PATH, [Value::Text(word)]) => mode = Mode::from_word(word),
            (RunPhase::PATH, [Value::Float(radians)]) => phase = Some(*radians),
            _ => {}
        }
    }
    Some(Report {
        pluck: pluck?,
        mode: mode?,
        phase: phase?,
    })
}
