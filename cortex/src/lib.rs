//! The shared crate: everything both rigs use, and the loop that runs their shows.
//!
//! The cabinet drives more than one sculpture, and only one runs at a time. The rigs diverge
//! in the show and converge in everything under it, so they are separate binaries over this
//! one library: a rig's fixtures are generated inside the rig's own crate, which makes
//! reaching into the other rig's fixtures impossible rather than merely discouraged.
//!
//! A rig hands over a [`Rig`] and gets the frame loop, the audio pipeline, both transports
//! and the takeover arbitration for it.

pub mod audio_features;
pub mod clock;
pub mod config;
pub mod eyeball;
pub mod latest;
pub mod moving_head;
pub mod perlin;
pub mod qlc_plus;
pub mod scenes;
pub mod sparkle;
pub mod telemetry;

mod capture;
mod dmx;
mod orchestrator;
mod sacn_in;

use audio_features::AudioFeatures;
use config::{FRAME_RATE_HZ, SACN_PORT, SACN_PRIORITY, SACN_RELEASE_FRAMES, UNIVERSE};
use embassy_executor::Executor;
use qlc_plus::PatchEntry;
use scenes::Scene;
use signal_hook::{consts::SIGTERM, iterator::Signals};
use std::net::UdpSocket;

/// The name the brain wears on the broker: its health topic and the prefix on everything else
/// it publishes. Not the binary's name — see where it is used.
const TELEMETRY_SERVICE: &str = "brain";

/// One rig: what its QLC+ workspace turned into, plus the show that drives it.
///
/// The workspace itself is not here and never crosses into the running program. It is a
/// build artifact: the ingest consumes it and the patch below is everything that survives.
///
/// `show` is called once per frame with the latest audio features, the frame period in
/// seconds, and the full 512-slot buffer to fill. It is a closure rather than a trait
/// implementation because the boundary carries no meaning beyond "the rig fills the slot
/// buffer" — when the intent frame lands it replaces the closure and neither rig's content
/// moves.
pub struct Rig {
    /// The binary's own name, which prefixes every log line. `journalctl` then says which
    /// rig is live without anyone having to read the symlink. Rigs pass
    /// `env!("CARGO_PKG_NAME")`, so it cannot drift from the binary the symlink points at.
    pub name: &'static str,
    /// The generated patch, as plain data, so the startup log can name what this binary was
    /// built against — and so the frame width can be derived from it.
    pub patch: &'static [PatchEntry],
    /// The generated scenes, likewise.
    pub scenes: &'static [Scene],
    pub show: Box<dyn FnMut(&AudioFeatures, f64, &mut [u8])>,
}

/// Slots the sACN frame spans: 1 through the last slot any patched fixture occupies.
///
/// Derived rather than carried beside the patch. The two would have to agree and nothing
/// could check that they did — a wrong constant is a panic on the first frame if it is too
/// large, and a permanently short network frame, invisible on the padded wire, if it is too
/// small.
pub(crate) fn frame_width(patch: &[PatchEntry]) -> usize {
    patch
        .iter()
        .flat_map(|fixture| fixture.channels)
        .map(|channel| channel.slot())
        .max()
        .map_or(0, |slot| slot + 1)
}

/// Brings the cabinet up, then runs `rig`'s frame loop forever.
///
/// The executor lives here rather than in the rig, so a rig binary is its patch, its scenes
/// and its show, and nothing else — `fn main` is one call. Audio capture always runs, on
/// every rig: a rig with no use for the interface simply never reads the features it is
/// handed, and a capture thread with nothing listening is cheaper than two configurations
/// that can disagree about which device the cabinet has.
pub fn run(rig: Rig) -> ! {
    let (tx, rx) = latest::latest(AudioFeatures::default());

    // Audio capture runs on its own OS thread, alongside Embassy.
    let _audio = capture::spawn_capture(tx);

    let socket = UdpSocket::bind("0.0.0.0:0").expect("socket bind failed");
    socket
        .set_multicast_ttl_v4(1)
        .expect("set multicast TTL failed");
    let cid = dmx::new_cid();

    // The external-takeover receiver: a second OS thread alongside audio capture, because a
    // blocking recv_from would stall the executor's single task. It publishes candidates;
    // the frame loop decides once per frame whether one is actually driving.
    let (takeover_tx, takeover_rx) = latest::latest(sacn_in::Takeover::idle());
    let _sacn_in = sacn_in::spawn_receiver(cid, takeover_tx);

    let group = dmx::multicast_addr(UNIVERSE);
    let name = rig.name;
    eprintln!("{name}: universe {UNIVERSE} → {group}:{SACN_PORT}  @ {FRAME_RATE_HZ} Hz");

    // The patch and scenes the build compiled in. Logged so a running binary can be checked
    // against the workspace that produced it: a fixture at the wrong address, or an edited
    // scene missing from these lines, means the build never saw the save.
    eprintln!("{name}: {} fixtures", rig.patch.len());
    for fixture in rig.patch {
        // The slot span, not the channel count: the mode name already carries the count,
        // and the last slot is what you check a patch against to see nothing collides.
        eprintln!(
            "{name}:   {} @ {}–{} — {}",
            fixture.name,
            fixture.address,
            fixture.address as usize + fixture.channels.len() - 1,
            fixture.profile,
        );
    }
    eprintln!("{name}: {} scenes", rig.scenes.len());
    for scene in rig.scenes {
        eprintln!("{name}:   {} ({} values)", scene.name, scene.values.len());
    }

    // The brain's own client. `brain` rather than the binary's name: only one rig runs at a
    // time, and a subscriber asking whether the brain is alive should not have to know which
    // sculpture is standing. Which one it is goes on `brain/identity`, retained, so that
    // arrives with the answer rather than instead of it.
    let (_telemetry, publisher, farewell) = telemetry::spawn(TELEMETRY_SERVICE);
    publisher.publish("identity/rig", name, true);
    publisher.publish("identity/universe", UNIVERSE.to_string(), true);
    publisher.publish("identity/frame_rate_hz", FRAME_RATE_HZ.to_string(), true);
    publisher.publish("identity/fixtures", rig.patch.len().to_string(), true);
    publisher.publish("identity/scenes", rig.scenes.len().to_string(), true);

    // systemd stops the brain with SIGTERM. Catch it to release the sACN source — a burst of
    // terminate frames — so a higher-priority console or the fixtures' fallback takes over at
    // once instead of waiting out the 2.5 s data-loss timeout. signal_hook's iterator runs on
    // its own thread, outside async-signal context, so the log line and socket sends here are
    // safe.
    let shutdown_socket = socket.try_clone().expect("socket clone failed");
    let dmx_slots = frame_width(rig.patch);
    let mut signals = Signals::new([SIGTERM]).expect("SIGTERM registration failed");
    std::thread::spawn(move || {
        if signals.forever().next().is_some() {
            eprintln!("{name}: SIGTERM — releasing sACN source ({SACN_RELEASE_FRAMES} terminate frames)");
            for sequence in 0..SACN_RELEASE_FRAMES {
                let packet = dmx::encode_release(UNIVERSE, sequence, SACN_PRIORITY, &cid, dmx_slots);
                dmx::send_multicast(&shutdown_socket, UNIVERSE, SACN_PORT, &packet);
            }
            // After the fixtures are handed back, because they are what the delay would cost.
            // Without this every ordinary restart would look like a crash: the will fires
            // whenever the connection dies without a goodbye, and `systemctl restart` is the
            // most common way for that to happen.
            farewell.say();
            std::process::exit(0);
        }
    });

    // `Executor::run` needs `&'static mut`, and never returns, so leaking one allocation buys
    // that lifetime honestly — this is what the `#[embassy_executor::main]` macro does with a
    // transmute instead.
    let executor: &'static mut Executor = Box::leak(Box::new(Executor::new()));
    executor.run(|spawner| {
        spawner.spawn(orchestrator::frame_task(socket, cid, rx, takeover_rx, rig).unwrap());
    })
}
