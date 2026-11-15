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
//! | from         | when                                       | to                                            |
//! |--------------|--------------------------------------------|-----------------------------------------------|
//! | await e-stop | pressed, unbroken, for `ESTOP_ARM_HOLD_US` | e-stop, holding the stop frame                |
//! | e-stop       | the e-stop is released                     | running                                       |
//! | running      | the e-stop is pressed                      | e-stop, holding the last frame under the mask |
//! | any          | SIGINT or SIGTERM                          | shutdown                                      |
//!
//! Await e-stop is where the machine starts, every time the brain does, so nothing runs
//! without a person at the cabinet to hold the e-stop and release it. A press released before
//! the hold is done starts the wait again. Until the first release the stop frame goes out:
//! blackout under the mask. Neither await e-stop nor shutdown can be returned to.
//!
//! The rig state is handed to the rig with the takeover as a [`Decision`], on every change, for
//! it to report.

use crate::{Decision, Rig};
use crate::clock;
use crate::config as cfg;
use crate::dmx::{self, Frame};
use crate::estop;
use crate::sacn_out;
use crate::latest::LatestRx;
use crate::sacn_in::Takeover;
use embassy_time::{Duration, Ticker};
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use zihatec_rs_485_dmx::{DmxHat, DmxTiming};

/// Whether, and what, goes out.
enum RigState {
    /// Since the brain started: the stop frame goes out while the e-stop is waited for, and
    /// when the current press began, if it is pressed.
    AwaitEstop { pressed_since_us: Option<u64> },
    /// The frame goes out as the show or the console made it, and is kept as the last frame
    /// that went out.
    Running(Frame),
    /// The held frame goes out, unchanged, every frame.
    Estop(Frame),
    /// The sACN source is released, and the process exits.
    Shutdown,
}

impl Decision {
    /// When the brain starts: awaiting the e-stop, nobody having taken the universe over.
    pub const BOOT: Decision = Decision {
        rig: RigState::AwaitEstop {
            pressed_since_us: None,
        }
        .word(),
        takeover: None,
    };
}

impl RigState {
    /// The rig state as it is reported in a [`Decision`].
    const fn word(&self) -> &'static str {
        match self {
            RigState::AwaitEstop { .. } => "await_estop",
            RigState::Running(_) => "running",
            RigState::Estop(_) => "estop",
            RigState::Shutdown => "shutdown",
        }
    }

    /// The rig state this frame moves to, if it moves, given whether the e-stop is pressed,
    /// whether shutdown was asked for, and the time. Awaiting the e-stop, it keeps the time
    /// the current press began itself. Running starts with no frame of its own yet, which the
    /// frame loop fills before anything reads it.
    fn next(
        &mut self,
        pressed: bool,
        shutdown: bool,
        now_us: u64,
        overrides: &[estop::StopOverride],
    ) -> Option<RigState> {
        if shutdown {
            return Some(RigState::Shutdown);
        }
        match (self, pressed) {
            (RigState::AwaitEstop { pressed_since_us }, true) => {
                let since = *pressed_since_us.get_or_insert(now_us);
                (now_us - since >= cfg::ESTOP_ARM_HOLD_US)
                    .then(|| RigState::Estop(stop_frame(overrides)))
            }
            (RigState::AwaitEstop { pressed_since_us }, false) => {
                *pressed_since_us = None;
                None
            }
            (RigState::Estop(_), false) => Some(RigState::Running(dmx::BLACKOUT)),
            (RigState::Running(last), true) => {
                Some(RigState::Estop(estop::held(last, overrides)))
            }
            _ => None,
        }
    }
}

/// What goes out until the e-stop is first released: blackout under the e-stop's mask, so the
/// rig is dark and still, whatever the fixtures were doing before the brain started.
fn stop_frame(overrides: &[estop::StopOverride]) -> Frame {
    estop::held(&dmx::BLACKOUT, overrides)
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
    // Which recipients' last send failed, one bit each, so each failure is logged once.
    let mut failing = 0u32;
    let mut state = RigState::AwaitEstop {
        pressed_since_us: None,
    };
    let mut decided = Decision::BOOT;
    eprintln!(
        "rig: awaiting the e-stop — hold it for {} ms, then release it; the stop frame goes out \
         until then",
        cfg::ESTOP_ARM_HOLD_US / 1000
    );

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
        let pressed = (rig.estop)();
        let asked = shutdown.load(Ordering::Relaxed);
        if let Some(next) = state.next(pressed, asked, clock::now_us(), rig.stop) {
            let awaiting = matches!(state, RigState::AwaitEstop { .. });
            match &next {
                RigState::AwaitEstop { .. } => unreachable!("await e-stop cannot be returned to"),
                RigState::Running(_) => eprintln!("rig: e-stop released — the frame goes out"),
                RigState::Estop(_) if awaiting => {
                    eprintln!("rig: e-stop held — release it to start")
                }
                RigState::Estop(_) => eprintln!(
                    "rig: e-stop pressed — holding the last frame, {} channels overridden",
                    rig.stop.len()
                ),
                RigState::Shutdown => eprintln!(
                    "rig: shutdown — releasing the sACN source ({} terminate frames)",
                    cfg::SACN_RELEASE_FRAMES
                ),
            }
            state = next;
        }

        let now_decided = Decision {
            rig: state.word(),
            takeover: external_driving.then_some((external.source, external.priority)),
        };
        if now_decided != decided {
            decided = now_decided;
            rig.decisions.publish(decided);
        }

        match &mut state {
            RigState::AwaitEstop { .. } => slots = stop_frame(rig.stop),
            RigState::Running(last) => *last = slots,
            RigState::Estop(held) => slots = *held,
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
                    sacn_out::send_unicast(
                        &socket,
                        rig.sacn_recipients,
                        &mut failing,
                        packet.as_bytes(),
                    );
                    sequence = sequence.wrapping_add(1);
                }
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
        sacn_out::send_unicast(&socket, rig.sacn_recipients, &mut failing, packet.as_bytes());
        // The HAT is the Pi's own UART: a pulled DMX lead never reaches it, so a write that
        // fails is the serial setup, and it will fail again.
        hat.send(&slots)
            .unwrap_or_else(|e| panic!("dmx hat: {}: {e}", cfg::SERIAL_DEVICE));
        sequence = sequence.wrapping_add(1);
    }
}
