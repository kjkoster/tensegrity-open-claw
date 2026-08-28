//! The receiving end of the vision sidecar: socket, schema, staleness. No vision code.
//!
//! The eyeball daemon is a separate process for reasons that outlast any language argument —
//! the show must survive it crashing, and a rig aiming moving heads at children cannot have
//! vision sharing an address space with the DMX loop. This module is the whole of what the
//! show side knows about it.
//!
//! Landmarks arrive as one JSON datagram per pose frame, over UDP on the loopback. UDP is not
//! a shortcut here: a pose frame that arrives late is worthless, so losing it is the correct
//! outcome, where a stream socket would faithfully deliver a queue of stale poses that look
//! fresh. Silence and a wedged sender are then the same observation, which is what the
//! staleness window is for.
//!
//! The key set inside `keypoints` is deliberately not fixed. The estimator is expected to be
//! replaced — a model today, contour extremities under infrared later — and a schema that
//! named the seventeen joints of one particular model would have to change with it.

use crate::clock;
use crate::config as cfg;
use crate::latest::LatestTx;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::io::ErrorKind;
use std::net::{SocketAddrV4, UdpSocket};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The largest datagram a pose frame can occupy. Seventeen keypoints of three floats plus
/// their names is well under a kilobyte; the rest is room for the sector values, envelope and
/// throw events that land on this same wire later.
const MAX_DATAGRAM_LEN: usize = 4096;

/// One pose frame, exactly as the daemon sends it, plus when it landed here.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Sighting {
    /// Counts frames the daemon produced, so a gap is visible as a gap rather than as a
    /// slower frame rate.
    pub seq: u64,
    /// The daemon's wall clock. Reported, never trusted: staleness is judged on the monotonic
    /// clock below, because the two ends have no reason to agree on the time of day.
    pub sent_at: f64,
    /// Which estimator produced this — `movenet`, `silhouette`, or whatever replaces them.
    /// It names the key set in `keypoints` and belongs in any log line about them.
    pub source: String,
    /// The rate the daemon is achieving, which is the number that says whether the vision
    /// half is healthy long before the show looks wrong.
    pub fps: f32,
    /// The daemon found a mage this frame, and with nothing else on the rig reporting a body,
    /// this *is* occupancy: presence and vision are the same fact, so a dead eyeball reads as a
    /// rig with nobody in front of it. The show's own staleness path is what covers that.
    pub present: bool,
    /// Name → `[x, y, confidence]`, normalised `0..1` within the crop. Normalised, so a change
    /// of camera resolution or crop moves nothing downstream.
    pub keypoints: BTreeMap<String, [f32; 3]>,
    /// Both arms, reduced to angles by the daemon. What the show steers on.
    ///
    /// Defaulted rather than required, because the two ends restart independently: a daemon
    /// old enough to send no arms should cost the show its pointing, not its vision. Without
    /// this the whole datagram fails to parse and the rig reads as blind.
    #[serde(default)]
    pub arms: Arms,

    /// Arrival on the shared monotonic clock. Filled here, never sent.
    #[serde(skip)]
    pub received_us: u64,
}

/// Both of the mage's arms, named anatomically.
///
/// Anatomical and never side-of-frame: the camera faces the mage, so the picture is mirrored
/// and a left-of-frame test would bind every arm to the wrong pair of heads.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Arms {
    pub left: Arm,
    pub right: Arm,
}

/// One arm's two segments as angles in the picture, with the lengths that say whether to
/// believe them.
///
/// Every angle is optional and the gate is per segment, not per arm: a forearm pointed at the
/// lens collapses to a few pixels and its direction becomes noise, while the upper arm above
/// it still reads fine. Half a readable arm still steers half a pair, so the halves blank
/// independently and whatever consumes them holds its last value rather than following the
/// noise.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Arm {
    /// Shoulder to elbow, unsigned: 0° hanging straight down, 180° straight up. Unsigned
    /// because which side of the body the arm swings out to says nothing about how far up it
    /// has come, and up is the whole of what the tilt channel wants.
    pub upper: Option<f64>,
    /// Elbow to wrist, in the picture, signed from straight down. Positive toward the
    /// picture's right, which — the picture being mirrored — is the mage's left.
    pub fore: Option<f64>,
    /// The elbow's own bend: the same forearm measured against the upper arm rather than
    /// against the ground. The other way of reading one limb, carried so the choice between
    /// the two can be made by watching a child instead of by rebuilding.
    pub bend: Option<f64>,
    /// How long each segment looks against the body's own scale. A segment reading no angle
    /// can be told from one whose landmarks were never found.
    pub upper_length: Option<f64>,
    pub fore_length: Option<f64>,
}

impl Sighting {
    /// Whether this sighting is recent enough to act on.
    ///
    /// The whole degrade path hangs off this one question: past the window, the show falls
    /// back to plain attentive and holds, whether the daemon died, wedged, or lost its camera.
    /// A default `Sighting` has never been received and reads stale from the first frame, so a
    /// show that starts before the daemon does behaves the same as one whose daemon left.
    pub fn fresh(&self) -> bool {
        self.seq > 0 && clock::now_us().saturating_sub(self.received_us) < cfg::EYEBALL_STALE_US
    }
}

/// Spawns the listener on its own thread and returns immediately.
///
/// Its own thread, like the audio capture and the sACN receiver, because a blocking receive
/// would stall the executor's single task. The thread never dies: a socket that cannot be
/// bound is retried with backoff, so the show keeps running and vision reappears whenever the
/// daemon does.
pub fn spawn_receiver(publisher: LatestTx<Sighting>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("eyeball".into())
        .spawn(move || {
            let mut backoff_s = 1u64;
            loop {
                if let Err(e) = receive(&publisher) {
                    eprintln!("eyeball: listener failed: {e}");
                }
                eprintln!("eyeball: rebinding in {backoff_s}s (show keeps running, degraded)");
                thread::sleep(Duration::from_secs(backoff_s));
                backoff_s = (backoff_s * 2).min(cfg::EYEBALL_BIND_RETRY_MAX_S);
            }
        })
        .expect("failed to spawn eyeball thread")
}

/// Binds and runs the receive loop until an unrecoverable socket error.
fn receive(publisher: &LatestTx<Sighting>) -> Result<(), Box<dyn Error>> {
    let socket = UdpSocket::bind(SocketAddrV4::new(
        cfg::EYEBALL_BIND_ADDRESS,
        cfg::EYEBALL_PORT,
    ))?;
    eprintln!(
        "eyeball: listening on {}:{} — stale after {} ms",
        cfg::EYEBALL_BIND_ADDRESS,
        cfg::EYEBALL_PORT,
        cfg::EYEBALL_STALE_US / 1_000,
    );

    // A read timeout rather than a blocking receive, so a daemon that goes quiet does not park
    // this thread forever with no way to log that it has gone.
    socket.set_read_timeout(Some(Duration::from_micros(cfg::EYEBALL_STALE_US)))?;

    let mut datagram = [0u8; MAX_DATAGRAM_LEN];
    let mut malformed = 0u64;

    loop {
        let n = match socket.recv(&mut datagram) {
            Ok(n) => n,
            // Both kinds appear because platforms disagree about which one a receive timeout
            // raises. Neither is a failure: it is the daemon being quiet, which the consumer
            // already sees as staleness.
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        match serde_json::from_slice::<Sighting>(&datagram[..n]) {
            Ok(mut sighting) => {
                sighting.received_us = clock::now_us();
                publisher.publish(sighting);
            }
            // Counted and reported sparsely rather than logged per datagram: a schema mismatch
            // arrives at pose rate, and filling the journal is how the one line that explains
            // it gets lost.
            Err(e) => {
                malformed += 1;
                if malformed.is_power_of_two() {
                    eprintln!("eyeball: {malformed} malformed datagrams, latest: {e}");
                }
            }
        }
    }
}
