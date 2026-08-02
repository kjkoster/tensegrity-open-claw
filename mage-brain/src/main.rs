//! The mage rig: four moving heads, a pinspot and a laser, driven by pose detection from a
//! camera rather than by sound. Nothing in this show listens to the room.
//!
//! The pinspot breathes purple, because it is the bench and bring-up fixture and it says the
//! frame loop is alive whatever the heads are doing. It reuses the claw's breath period,
//! floor and ceiling — one set of numbers in `cortex`, so the two rigs cannot end up
//! breathing differently by accident.
//!
//! The heads do one of two things. Left alone they run a metronome between two poses, each at
//! its own speed over its own span: not a show and not meant to become one, it exists to put
//! slow, long, 16-bit moves on the rig, which is the only way to see whether the position
//! pipeline is smooth. Given a point on standard input they all converge on it instead, which
//! is how the geometry gets proved: a place on the field, typed in metres, and four beams
//! either meet there or say what is wrong. Typing `m` hands them back to the metronome.
//!
//! Nothing here decides where to go on its own yet. Motion that picks its own targets waits
//! for the limits clamp.

mod geometry;

use cortex::Rig;
use cortex::audio_features::AudioFeatures;
use cortex::config as cfg;
use cortex::eyeball::{self, Sighting};
use cortex::geometry::{Chooser, Point};
use cortex::latest;
use cortex::moving_head::{Pose, Slew, SlewRate, aim};
use std::f64::consts::TAU;
use std::io::BufRead;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

// ── Tunables ─────────────────────────────────────────────────────────────────
// This rig's numbers live in this rig's crate. `cortex::config` describes the *cabinet* —
// the serial device, the audio interface, the universe — and a pose that only means anything
// on the mage's field would be a rig fact sitting in the shared half, where the claw would
// carry it around for nothing and the two could quietly come to disagree.
//
// Only what is a choice about the show is here. What a head *is* — how far it pans, which
// end of its speed channel is fast, where its dead bands sit — comes from the fixture
// definition through the generated patch, so none of it is repeated here to go stale. Where
// each head *stands* is in `geometry`, because that is measured afresh every deployment.

/// One head's bring-up motion: the ceiling on its angular speed, and the two poses it moves
/// between. One struct per head rather than parallel arrays, so a head is a thing that can be
/// handed around whole and cannot be assembled from mismatched rows.
#[derive(Clone, Copy)]
struct HeadMetronome {
    slew: SlewRate,
    from: Pose,
    to: Pose,
}

// How long a head rests at a pose before being sent to the other. Shared across the heads
// while the speeds are not: they leave together and arrive apart, which is what makes four of
// them read as four rather than as one wide machine. Every head's travel has to fit inside
// the rest, or its target moves while it is still under way and it never reaches an end to be
// judged at. The longest traverse is snake 3's two hundred and forty degrees, which takes
// just under 22 s, so 30 leaves every head time to settle without the dead air a longer hold
// would give the faster ones.
const METRONOME_HOLD_S: f64 = 30.0;

// Unreferenced degrees: zero is wherever a head's own zero falls, which depends on how it is
// clamped and how the stand sits, so these get retuned on the rig. What must hold is that
// every span is a long move, and that none of them outruns the rest above.
//
// Snake 1 runs as slowly as these heads can be driven and still look like they are moving,
// with a span long enough that the traverse takes over twenty seconds anyway. That is the
// lesson the bring-up produced: on this hardware a slow move is a long one, not a lazy one,
// and asking for a lazy one gets the same speed with the request quietly rounded up.
const SNAKE_1_METRONOME: HeadMetronome = HeadMetronome {
    slew: SlewRate::SLOWEST,
    from: Pose::new(200.0, 120.0),
    to: Pose::new(350.0, 150.0),
};
const SNAKE_2_METRONOME: HeadMetronome = HeadMetronome {
    slew: SlewRate::new(9.0),
    from: Pose::new(150.0, 100.0),
    to: Pose::new(300.0, 160.0),
};
const SNAKE_3_METRONOME: HeadMetronome = HeadMetronome {
    slew: SlewRate::new(11.0),
    from: Pose::new(120.0, 90.0),
    to: Pose::new(360.0, 180.0),
};
const SNAKE_4_METRONOME: HeadMetronome = HeadMetronome {
    slew: SlewRate::new(16.0),
    from: Pose::new(90.0, 80.0),
    to: Pose::new(400.0, 175.0),
};

// Enough light to see the beam land without lighting the room. The traverse is judged on
// smoothness, and a hot beam hides stepping in its own glare.
const TEST_DIMMER: f64 = 0.25;

// `patch` and `scenes`, generated from mage.qxw by the ingest. The workspace and the `.qxf`
// definitions are the source of truth; nothing about the addressing is hand-maintained.
// Generating them into this crate rather than the shared one is also what keeps the rigs
// apart: the claw's fixtures do not exist in this binary to be reached for. The camera is not
// a DMX device and appears in neither.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

/// One head with everything that drives it: where it stands, how it is allowed to move, and
/// which of the ways to hit a target it is currently using.
///
/// Assembled per head and never by index, so a head cannot end up wearing another head's
/// mount — which would aim it into the crowd while every number involved still looked
/// reasonable.
struct Snake {
    head: &'static geometry::Head,
    metronome: HeadMetronome,
    slew: Slew,
    chooser: Chooser,
}

impl Snake {
    fn new(head: &'static geometry::Head, metronome: HeadMetronome) -> Self {
        Self {
            head,
            metronome,
            slew: Slew::new(metronome.from, metronome.slew),
            chooser: Chooser::new(head.mount, head.travel(), metronome.slew),
        }
    }
}

/// What the operator has asked the heads to do.
enum Command {
    Converge(Point),
    Metronome,
}

/// Reads commands from standard input, on its own thread because a blocking read would stall
/// the executor.
///
/// A bring-up path, and only that: the brain runs from a shell for this, and under systemd
/// standard input is empty, the thread ends at once and the metronome runs as it always did.
fn spawn_console() -> Receiver<Command> {
    let (commands, received) = channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let words: Vec<&str> = line.split_whitespace().collect();
            let command = match words.as_slice() {
                [] => continue,
                ["m"] => Some(Command::Metronome),
                [x, y, z] => match (x.parse(), y.parse(), z.parse()) {
                    (Ok(x), Ok(y), Ok(z)) => Some(Command::Converge(Point::new(x, y, z))),
                    _ => None,
                },
                _ => None,
            };
            match command {
                Some(command) => {
                    if commands.send(command).is_err() {
                        return;
                    }
                }
                None => eprintln!("console: `x y z` in metres to converge, `m` for the metronome"),
            }
        }
    });
    received
}

fn main() {
    // Phase carried across frames rather than read off the wall clock, so the breath is
    // continuous through anything that stalls a frame, and wrapped to one period so it stays
    // exact however many days the rig runs.
    let mut phase_s = 0.0f64;

    // The metronome runs on the same carried clock, over a full there-and-back cycle. One
    // clock for all four heads: they are sent off together and separate on the way, because
    // they travel at different speeds, which is the whole reason to drive four of them here.
    let mut metronome_s = 0.0f64;

    let mut snakes = [
        Snake::new(&geometry::SNAKE_1, SNAKE_1_METRONOME),
        Snake::new(&geometry::SNAKE_2, SNAKE_2_METRONOME),
        Snake::new(&geometry::SNAKE_3, SNAKE_3_METRONOME),
        Snake::new(&geometry::SNAKE_4, SNAKE_4_METRONOME),
    ];

    let console = spawn_console();
    // No target until one is typed, which is what leaves the metronome running for a rig that
    // came up on its own.
    let mut target: Option<Point> = None;

    // The landmark stream, read but not yet acted on: nothing here decides where to go from a
    // pose, and will not until the limits clamp exists. Logging it is how the vision half gets
    // proved end to end — camera, daemon, socket, schema — while the show stays a metronome.
    let (sightings_tx, sightings) = latest::latest(Sighting::default());
    let _eyeball = eyeball::spawn_receiver(sightings_tx);
    let mut since_sighting_log_s = 0.0f64;
    let mut had_vision = false;

    cortex::run(Rig {
        name: env!("CARGO_PKG_NAME"),
        patch: &patch::PATCH,
        scenes: &scenes::SCENES,
        // The audio features are ignored, deliberately: the mage show is pose-driven and
        // there is no pose stage yet. The capture thread runs anyway, because it belongs to
        // the cabinet rather than to either rig.
        show: Box::new(
            move |_features: &AudioFeatures, dt: f64, slots: &mut [u8]| {
                phase_s = (phase_s + dt) % cfg::SPARKLE_BREATH_PERIOD_S;

                // A cosine breath between the floor and the ceiling. It never reaches zero: a
                // light that goes fully dark reads as broken, while one that keeps an ember
                // reads as alive.
                let breath = 0.5 - 0.5 * (TAU * phase_s / cfg::SPARKLE_BREATH_PERIOD_S).cos();
                let level = cfg::SPARKLE_BREATH_FLOOR
                    + breath * (cfg::SPARKLE_BREATH_CEIL - cfg::SPARKLE_BREATH_FLOOR);
                // Gamma applies to the level, not to the mix: the breath is shaped in perceived
                // brightness, and the colour rides on it.
                let level = level.powf(1.0 / cfg::GAMMA);

                // Purple is red and blue together, with the green emitter held off rather than
                // left wherever it was.
                patch::PINSPOT.red.set_unit(slots, level);
                patch::PINSPOT.green.set(slots, 0);
                patch::PINSPOT.blue.set_unit(slots, level);
                // Effect below 64 is what hands the emitters to the colour channels at all, and
                // Speed only feeds the internal programs that Effect would start.
                patch::PINSPOT.effect.set(slots, 0);
                patch::PINSPOT.speed.set(slots, 0);

                // Whatever was typed since the last frame. Which heads can reach the point is
                // answered here and now, because an operator who typed a spot no head can see is
                // owed that news rather than four beams that quietly do not move.
                loop {
                    match console.try_recv() {
                        Ok(Command::Converge(point)) => {
                            target = Some(point);
                            for snake in &snakes {
                                if !snake.chooser.reaches(point) {
                                    eprintln!(
                                        "console: {} cannot reach that point",
                                        snake.head.name
                                    );
                                }
                            }
                        }
                        Ok(Command::Metronome) => target = None,
                        Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                    }
                }

                // The landmark stream, reported and nowhere else consumed. Crossing into or out
                // of freshness is logged the moment it happens rather than at the next tick,
                // because the interesting question during bring-up is *when* vision went away.
                let sighting = sightings.snapshot();
                let has_vision = sighting.fresh();
                since_sighting_log_s += dt;
                if has_vision != had_vision || since_sighting_log_s >= cfg::EYEBALL_LOG_INTERVAL_S {
                    since_sighting_log_s = 0.0;
                    had_vision = has_vision;
                    if has_vision {
                        eprintln!(
                            "eyeball: seq {} — {} @ {:.1} Hz, mage {}",
                            sighting.seq,
                            sighting.source,
                            sighting.fps,
                            if sighting.present { "seen" } else { "not seen" },
                        );
                        for (name, [x, y, confidence]) in &sighting.keypoints {
                            eprintln!("eyeball:   {name} {x:.3} {y:.3} ({confidence:.2})");
                        }
                    } else {
                        eprintln!("eyeball: no sightings — show holds without vision");
                    }
                }

                // Every head is held at one pose for long enough to settle, then sent to the
                // other. What reaches the wire is each slew's own position, never the target, so
                // the step in the target becomes a traverse at that head's bounded speed.
                metronome_s = (metronome_s + dt) % (2.0 * METRONOME_HOLD_S);
                let outbound = metronome_s < METRONOME_HOLD_S;

                for snake in &mut snakes {
                    let head = snake.head.fixture;
                    let (pose, lit) = match target {
                        Some(point) => {
                            // Judged from where the head actually stands, which the rate
                            // limiter below is already holding — the metronome may have been
                            // driving it a frame ago.
                            let beam = snake.chooser.toward(snake.slew.pose(), point, dt);
                            (beam.pose, beam.lit)
                        }
                        None if outbound => (snake.metronome.to, 1.0),
                        None => (snake.metronome.from, 1.0),
                    };
                    aim(head, slots, snake.slew.step(pose, dt));

                    head.dimmer.set_unit(slots, TEST_DIMMER * lit);
                    head.strobe
                        .set(slots, patch::Zq02015::STROBE_SHUTTER_OPEN.center());
                    // Both ends of this channel were tried on the rig and neither changed how the
                    // head tracks a stream of positions, so the fast end is chosen on principle
                    // rather than on evidence: the software owns the interpolation, and a ramp
                    // inside the fixture would be a second one we cannot see.
                    head.motor_speed
                        .set(slots, patch::Zq02015::MOTOR_SPEED_FASTEST);
                    // Open beam, no filter: this step is about motion, and a gobo in the way
                    // makes a soft edge that hides exactly the stepping being looked for.
                    head.color.set(slots, patch::Zq02015::COLOR_WHITE.center());
                    head.gobo
                        .set(slots, patch::Zq02015::GOBO_OPEN_WHITE.center());
                    // The channels that must never drift, on values the definition names.
                    head.automatic_mode.park(slots);
                    head.reset.park(slots);
                    head.light_strips
                        .set(slots, patch::Zq02015::LIGHT_STRIPS_OFF.min);
                }
            },
        ),
    })
}
