//! The per-frame DMX loop, shared by every rig: tick, ask the rig's show to fill a slot
//! buffer, decide who owns the universe, and emit one frame to both transports.
//!
//! What the rig contributes is the fill, plus the e-stop's mask and button. That is the whole
//! seam — no show trait, no stage vocabulary — because a boundary carrying more meaning than
//! "the rig fills the slot buffer" would formalise two shows as two code bases and invite them
//! to drift apart.
//!
//! This is also where an external console takes the rig over, and where the rig state machine
//! has the last word over whoever owns it. Ownership is decided once per frame and one slot
//! buffer goes to both outputs, so the wire and the network can never be driven from
//! different sources.
//!
//! The rig state says whether, and what, goes out. Its machine is the layer above the show's,
//! and the show knows nothing of it:
//!
//! | from      | when                     | to                                            |
//! |-----------|--------------------------|-----------------------------------------------|
//! | boot hold | the e-stop is released   | running                                       |
//! | boot hold | the e-stop is pressed    | e-stop, holding nothing                       |
//! | running   | the e-stop is pressed    | e-stop, holding the last frame under the mask |
//! | e-stop    | the e-stop is released   | running                                       |
//! | any       | SIGINT or SIGTERM        | shutdown                                      |
//!
//! Boot hold is where the machine starts, and it leaves on the first frame. Nothing goes out
//! in an e-stop entered from it: until a frame has gone out the brain does not know what the
//! fixtures are doing, and silence leaves them where they were, by their own signal-loss
//! behaviour. Neither boot hold nor shutdown can be returned to.
//!
//! The rig state is published on `brain/state/rig`, retained, on every change.

use crate::Rig;
use crate::clock;
use crate::config as cfg;
use crate::dmx::{self, Frame};
use crate::estop;
use crate::sacn_out;
use crate::latest::LatestRx;
use crate::sacn_in::Takeover;
use crate::telemetry;
use embassy_time::{Duration, Ticker};
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use zihatec_rs_485_dmx::{DmxHat, DmxTiming};

/// Whether, and what, goes out.
enum RigState {
    /// Before the first frame: nothing has gone out yet.
    BootHold,
    /// The frame goes out as the show or the console made it, and is kept as the last frame
    /// that went out.
    Running(Frame),
    /// The held frame goes out, unchanged, every frame; or nothing, when the e-stop was
    /// pressed before anything had gone out.
    Estop(Option<Frame>),
    /// The sACN source is released, the telemetry says goodbye, and the process exits.
    Shutdown,
}

impl RigState {
    /// The rig state as it is published on `brain/state/rig`.
    fn word(&self) -> &'static str {
        match self {
            RigState::BootHold => "boot_hold",
            RigState::Running(_) => "running",
            RigState::Estop(_) => "estop",
            RigState::Shutdown => "shutdown",
        }
    }

    /// The rig state this frame moves to, if it moves, given whether the e-stop is pressed and
    /// whether shutdown was asked for. Running starts with no frame of its own yet, which the
    /// frame loop fills before anything reads it.
    fn next(
        &self,
        pressed: bool,
        shutdown: bool,
        overrides: &[estop::StopOverride],
    ) -> Option<RigState> {
        if shutdown {
            return Some(RigState::Shutdown);
        }
        match (self, pressed) {
            (RigState::BootHold, false) | (RigState::Estop(_), false) => {
                Some(RigState::Running(dmx::BLACKOUT))
            }
            (RigState::BootHold, true) => Some(RigState::Estop(None)),
            (RigState::Running(last), true) => {
                Some(RigState::Estop(Some(estop::held(last, overrides))))
            }
            _ => None,
        }
    }
}

#[embassy_executor::task]
pub async fn frame_task(
    socket: UdpSocket,
    cid: [u8; 16],
    takeover: LatestRx<Takeover>,
    shutdown: Arc<AtomicBool>,
    mut rig: Rig,
) -> ! {
    // The wired HAT mirrors the same slot buffer as the sACN send. A broken serial setup is
    // a deploy-time gate, not a runtime hazard, so panic with the crate's remediation
    // message rather than limp along without the wire.
    let mut hat = DmxHat::open(cfg::SERIAL_DEVICE, DmxTiming::default())
        .unwrap_or_else(|e| panic!("dmx hat: {}: {e}", cfg::SERIAL_DEVICE));

    let dmx_slots = crate::frame_width(rig.patch);

    let mut ticker = Ticker::every(Duration::from_micros(1_000_000 / cfg::FRAME_RATE_HZ));
    let mut sequence: u8 = 0;
    let dt = 1.0 / cfg::FRAME_RATE_HZ as f64;
    let mut overridden = false;
    let mut state = RigState::BootHold;
    rig.telemetry.publish("state/rig", state.word(), true);

    loop {
        ticker.next().await;

        // A full 512-slot universe for the wire; the sACN frame carries only the live head.
        // Slots outside a patched fixture's block stay zero, and so do the ones the rig's
        // show says nothing about.
        let mut slots = dmx::BLACKOUT;
        (rig.show)(dt, &mut slots);

        // The show above runs every frame whether or not it is driving. Its phases and slews
        // stay continuous, so handback resumes mid-stride instead of springing out of a frozen
        // frame.
        //
        // One decision, one buffer: everything below ships whatever `slots` holds, to both
        // the wire and the network, so the two can never follow different sources. Takeover
        // is whole-universe — the external frame replaces every slot, never some of them.
        let external = takeover.snapshot();
        let external_driving = external.in_force(clock::now_us());
        if external_driving {
            slots = external.slots;
        }

        if external_driving != overridden {
            overridden = external_driving;
            if overridden {
                eprintln!("sacn: takeover by {}", external.describe());
            } else {
                eprintln!(
                    "sacn: released by {} — internal engine resumes",
                    external.describe()
                );
            }
        }

        // Last, on the frame as it will go out, so it beats the show and a console alike.
        if let Some(next) = state.next((rig.estop)(), shutdown.load(Ordering::Relaxed), rig.stop) {
            match &next {
                RigState::BootHold => unreachable!("boot hold cannot be returned to"),
                RigState::Running(_) => eprintln!("rig: e-stop released — the frame goes out"),
                RigState::Estop(None) => {
                    eprintln!("rig: e-stop pressed at boot — nothing goes out until it is released")
                }
                RigState::Estop(Some(_)) => eprintln!(
                    "rig: e-stop pressed — holding the last frame, {} channels overridden",
                    rig.stop.len()
                ),
                RigState::Shutdown => eprintln!(
                    "rig: shutdown — releasing the sACN source ({} terminate frames)",
                    cfg::SACN_RELEASE_FRAMES
                ),
            }
            rig.telemetry.publish("state/rig", next.word(), true);
            state = next;
        }

        match &mut state {
            RigState::BootHold | RigState::Estop(None) => continue,
            RigState::Running(last) => *last = slots,
            RigState::Estop(Some(held)) => slots = *held,
            RigState::Shutdown => {
                // A burst rather than one frame, because UDP is lossy and any one arriving
                // hands the fixtures over at once instead of after the data-loss timeout.
                for _ in 0..cfg::SACN_RELEASE_FRAMES {
                    let packet = sacn_out::encode_release(
                        rig.sacn.universe,
                        sequence,
                        cfg::SACN_PRIORITY,
                        &cid,
                        dmx_slots,
                    );
                    sacn_out::send_multicast(&socket, rig.sacn.universe, rig.sacn.port, &packet);
                    sequence = sequence.wrapping_add(1);
                }
                // After the fixtures are handed back, because they are what the delay would
                // cost. Without this every ordinary restart would look like a crash: the will
                // fires whenever the connection dies without a goodbye, and `systemctl
                // restart` is the most common way for that to happen.
                telemetry::farewell(&rig.farewells);
                std::process::exit(0);
            }
        }

        rig.frames.publish(slots);

        let packet = sacn_out::encode(
            rig.sacn.universe,
            sequence,
            cfg::SACN_PRIORITY,
            0,
            &cid,
            &slots[..dmx_slots],
        );
        sacn_out::send_multicast(&socket, rig.sacn.universe, rig.sacn.port, &packet);
        // A wire write error is logged, not fatal — a yanked cable must not take the
        // sculpture down.
        if let Err(e) = hat.send(&slots) {
            eprintln!("dmx hat: write error: {e}");
        }
        sequence = sequence.wrapping_add(1);
    }
}
