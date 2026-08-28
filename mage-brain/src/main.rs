//! The mage rig: four moving heads, a pinspot and a laser, driven by pose detection from a
//! camera rather than by sound. Nothing in this show listens to the room.
//!
//! The pinspot breathes purple, because it is the bench and bring-up fixture and it says the
//! frame loop is alive whatever the heads are doing. It reuses the claw's breath period,
//! floor and ceiling — one set of numbers in `cortex`, so the two rigs cannot end up
//! breathing differently by accident.
//!
//! The rig runs three states, and they belong to the mage rather than to a head: all four
//! snakes are one creature's attention. **Bored** is nobody in front of the camera — each
//! head walks between two poses at its own speed, white and dim, which is an attract loop and
//! also the only way to see whether the position pipeline is smooth. **Attentive** is a mage
//! seen with both hands below the waist: all four go to their recorded homes and breathe.
//! **Magic** is a hand raised, and it takes the whole rig.
//!
//! Inside magic the arms still work one at a time — the left arm flies snakes 1 and 2 and the
//! right arm 3 and 4 — so a kid who raises one hand moves that pair and finds the other pair
//! follows the arm they left hanging.
//!
//! There is no geometry in any of it. The arms drive pan and tilt as channel values, and the
//! only thing measured about a head is where it was pointing when somebody drove it onto the
//! mage by hand.

mod geometry;

use cortex::Rig;
use cortex::audio_features::AudioFeatures;
use cortex::config as cfg;
use cortex::eyeball::{self, Arm, Sighting};
use cortex::latest;
use cortex::moving_head::{Pose, Slew, SlewRate, aim};
use geometry::Aim;
use std::f64::consts::TAU;

// ── Tunables ─────────────────────────────────────────────────────────────────
// This rig's numbers live in this rig's crate. `cortex::config` describes the *cabinet* —
// the serial device, the audio interface, the universe — and a value that only means
// anything on the mage's field would be a rig fact sitting in the shared half, where the claw
// would carry it around for nothing and the two could quietly come to disagree.
//
// Only what is a choice about the show is here. What a head *is* — how far it pans, which
// end of its speed channel is fast, where its dead bands sit — comes from the fixture
// definition through the generated patch, so none of it is repeated here to go stale. Where
// each head points in attentive is in `geometry`, because that is recorded afresh every
// deployment.

/// One head's bored motion: the ceiling on its angular speed, and the two poses it walks
/// between. One struct per head rather than parallel arrays, so a head is a thing that can be
/// handed around whole and cannot be assembled from mismatched rows.
#[derive(Clone, Copy)]
struct HeadWander {
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
const WANDER_HOLD_S: f64 = 30.0;

// Unreferenced degrees: zero is wherever a head's own zero falls, which depends on how it is
// clamped and how the stand sits, so these get retuned on the rig. What must hold is that
// every span is a long move, and that none of them outruns the rest above.
//
// Snake 1 runs as slowly as these heads can be driven and still look like they are moving,
// with a span long enough that the traverse takes over twenty seconds anyway. That is the
// lesson the bring-up produced: on this hardware a slow move is a long one, not a lazy one,
// and asking for a lazy one gets the same speed with the request quietly rounded up.
const SNAKE_1_WANDER: HeadWander = HeadWander {
    slew: SlewRate::SLOWEST,
    from: Pose::new(200.0, 120.0),
    to: Pose::new(350.0, 150.0),
};
const SNAKE_2_WANDER: HeadWander = HeadWander {
    slew: SlewRate::new(9.0),
    from: Pose::new(150.0, 100.0),
    to: Pose::new(300.0, 160.0),
};
const SNAKE_3_WANDER: HeadWander = HeadWander {
    slew: SlewRate::new(11.0),
    from: Pose::new(120.0, 90.0),
    to: Pose::new(360.0, 180.0),
};
const SNAKE_4_WANDER: HeadWander = HeadWander {
    slew: SlewRate::new(16.0),
    from: Pose::new(90.0, 80.0),
    to: Pose::new(400.0, 175.0),
};

// How fast a head is driven while an arm is steering it. Near the mechanism's ceiling and
// deliberately nothing like the wander speeds above: the personality slew is what makes a
// bored head funny, and the same number under a pointing arm reads as a fault, because the
// kid is watching their own arm and the beam at the same time.
const MAGIC_SLEW: SlewRate = SlewRate::new(120.0);

// How fast a head crosses into attentive. Between the two, on purpose: at a personality rate
// a head takes twenty seconds to notice somebody walking up, and at the magic rate it snaps,
// which is the one transition a kid is actually watching for.
const ATTENTIVE_SLEW: SlewRate = SlewRate::new(40.0);

// The shortest a state is allowed to last. Every boundary here is a continuous quantity
// crossing a threshold, and the hysteresis bands alone do not cover a hand that hovers or a
// mage the model keeps losing and refinding — both of which would otherwise re-spin a colour
// wheel and restage the whole rig several times a second, which reads as a malfunction rather
// than as a show. A state held for a beat also gives a kid time to notice they caused it.
const STATE_DWELL_S: f64 = 2.0;

// How long the field has to stay empty before the heads give up and go back to wandering.
// Long enough to ride out a mage who turns side-on to the camera or steps behind another
// child, short enough that an empty field stops looking attended.
const BORED_AFTER_S: f64 = 5.0;

// Bored is watched from across a field with nobody in it, so it is lit to be visible rather
// than to land on anyone.
const BORED_DIMMER: u8 = 0x40;

// White and an open gate for both of the watching states: attentive is a beam on a person and
// wants nothing in the way of it.
const WHITE: u8 = 0x00;
const GOBO_OPEN: u8 = 0x00;

// The bottom of attentive's breath. An absolute value rather than a fraction of the peak,
// because what matters is where this fixture's lamp stops striking, and that is a place on the
// channel rather than a proportion of whatever the show happened to ask for — a floor written
// as a fraction of a dim ceiling lands far lower than it reads.
const ATTENTIVE_DIMMER: u8 = 0x40;
const ATTENTIVE_DIMMER_FLOOR: u8 = 0x30;

// Magic is the state the show is for, so it is the bright one, and it is the only one with a
// colour and a pattern in the beam — which is what makes an arm coming up read as a change in
// kind rather than a change in aim.
const MAGIC_DIMMER: u8 = 0xc8;
const MAGIC_DIMMER_FLOOR: u8 = 0x90;
const MAGIC_COLOR_RED: u8 = 0x10;
const MAGIC_GOBO: u8 = 0x30;

// Tilt against the upper arm: hanging straight down puts the beam up over the mage, straight
// up runs it out over the audience. The two ends are chosen rather than mechanical — the
// arm-down end is deliberately not the channel's own end, because these heads sit at lens
// height and the extreme would be a beam through a child.
const MAGIC_TILT_ARM_DOWN: u16 = 0xa000;
const MAGIC_TILT_ARM_UP: u16 = 0x0000;

// Pan against the forearm: hanging straight down is the centre, and swinging it either way
// carries the pair with it. Both heads in a pair take the same value, so they swing together
// and keep the fan they were set up with — which is what reads as *both my snakes turned that
// way* rather than as two heads doing two things.
const MAGIC_PAN_CENTRE: u16 = 0xaa00;
const MAGIC_PAN_FULL_LEFT: u16 = 0xe000;
const MAGIC_PAN_FULL_RIGHT: u16 = 0x7000;

// How far off vertical the forearm has to swing to reach those ends. A quarter turn, so a kid
// reaches the extremes without having to fold their arm behind them.
const MAGIC_FORE_FULL_DEG: f64 = 90.0;

// The upper arm's own range, which is what the daemon reports: 0° hanging straight down to
// 180° straight up.
const UPPER_ARM_RANGE_DEG: f64 = 180.0;

// How far a wrist has to clear its own hip to put the rig into magic, and how far it has to
// fall back before it leaves. In the picture's own normalised height, and the gap between the
// two is the hysteresis on that boundary — a hand resting on the line would otherwise restage
// the whole rig on the model's noise.
//
// The camera sits on the ground looking up, so a wrist reaching *toward* the lens projects
// lower in the image than it really is. That is what the margins are for as much as the
// chatter is — a kid pointing at the camera would otherwise drop out of magic.
const MAGIC_WRIST_RISE: f32 = 0.04;
const MAGIC_WRIST_FALL: f32 = 0.01;

// How far the breath swings the beam either side of where the state put it. The same breath
// that rides the dimmer rides the aim, so a head that is holding still is never quite still —
// which is most of what makes four lights read as four creatures rather than four lamps.
//
// Pan and tilt take it a quarter turn apart, so the beam walks a small slow circle instead of
// sliding up and down one diagonal. A diagonal reads as a fader somebody is moving.
const BREATH_ROVE_DEG: f64 = 3.0;

// `patch` and `scenes`, generated from mage.qxw by the ingest. The workspace and the `.qxf`
// definitions are the source of truth; nothing about the addressing is hand-maintained.
// Generating them into this crate rather than the shared one is also what keeps the rigs
// apart: the claw's fixtures do not exist in this binary to be reached for. The camera is not
// a DMX device and appears in neither.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

/// One head with everything that drives it: where it points when watching, how it wanders
/// when bored, and where it actually is right now.
///
/// Assembled per head and never by index, so a head cannot end up wearing another head's
/// recorded home — which would send it somewhere nobody looked while every number involved
/// still looked reasonable.
struct Snake {
    head: &'static geometry::Head,
    wander: HeadWander,
    slew: Slew,
}

impl Snake {
    fn new(head: &'static geometry::Head, wander: HeadWander) -> Self {
        Self {
            head,
            wander,
            slew: Slew::new(wander.from, wander.slew),
        }
    }
}

/// What the whole rig is doing.
///
/// The mage's state and not a head's: four snakes are one creature's attention, and two of
/// them watching while the other two fly reads as a rig with a fault rather than as a trick.
/// Which arm moves which pair stays a per-pair question — the state says whether anybody is
/// flying at all.
#[derive(Clone, Copy, PartialEq)]
enum Show {
    Bored,
    Attentive,
    Magic,
}

impl Show {
    fn label(self) -> &'static str {
        match self {
            Self::Bored => "bored — no mage",
            Self::Attentive => "attentive — mage seen, hands down",
            Self::Magic => "magic — a hand is up",
        }
    }
}

/// One arm and the two heads it flies.
struct Pair {
    /// Which arm steers it, and the prefix its landmarks carry. Anatomical, never
    /// side-of-frame: the camera faces the mage, so the picture is mirrored and a
    /// side-of-frame test binds every arm to the wrong pair.
    side: &'static str,
    heads: [usize; 2],
    /// The last aim the arm produced. Held rather than recomputed when a segment blanks: a
    /// forearm pointed at the lens has no readable direction, and following the noise is
    /// worse than holding still.
    aim: Aim,
}

impl Pair {
    fn new(side: &'static str, heads: [usize; 2]) -> Self {
        Self {
            side,
            heads,
            aim: Aim::new(MAGIC_PAN_CENTRE, MAGIC_TILT_ARM_DOWN),
        }
    }

    /// Where this arm is pointing its pair, or the last answer if it cannot say.
    ///
    /// The two segments are read independently, so a readable upper arm still drives tilt
    /// while an unreadable forearm leaves pan where it was.
    fn steer(&mut self, arm: &Arm) {
        if let Some(upper) = arm.upper {
            self.aim.tilt = lerp(
                MAGIC_TILT_ARM_DOWN,
                MAGIC_TILT_ARM_UP,
                upper / UPPER_ARM_RANGE_DEG,
            );
        }
        if let Some(fore) = arm.fore {
            // Positive is toward the picture's right, which the mirror makes the mage's left.
            // The two halves are interpolated separately because the ends are not the same
            // distance from the centre, and one straight line through all three would put the
            // centre somewhere the arm never hangs.
            let end = if fore >= 0.0 {
                MAGIC_PAN_FULL_LEFT
            } else {
                MAGIC_PAN_FULL_RIGHT
            };
            self.aim.pan = lerp(MAGIC_PAN_CENTRE, end, fore.abs() / MAGIC_FORE_FULL_DEG);
        }
    }
}

/// Whether either hand has cleared its own hip, or `None` when the picture cannot say.
///
/// Image coordinates run down the picture, so a wrist above its hip carries the smaller
/// number. `already` picks which side of the hysteresis band to measure against, so a hand
/// resting near the line does not flutter the whole rig between two states.
///
/// A side whose landmarks are missing simply does not vote. Only when neither side is
/// readable is the answer `None`, which the caller turns into "whatever it was" — a mage the
/// model briefly lost is not a mage who put their hands down.
fn hand_up(sighting: &Sighting, already: bool) -> Option<bool> {
    let margin = if already {
        MAGIC_WRIST_FALL
    } else {
        MAGIC_WRIST_RISE
    };
    let mut answer = None;
    for side in ["left", "right"] {
        let landmark = |name: &str| sighting.keypoints.get(&format!("{side}_{name}"));
        if let (Some(wrist), Some(hip)) = (landmark("wrist"), landmark("hip")) {
            answer = Some(answer.unwrap_or(false) || hip[1] - wrist[1] > margin);
        }
    }
    answer
}

/// Walks from one channel value to another, `at` running 0 to 1 and clamped at both ends.
///
/// In the channel's own units rather than in degrees, because both ends of every one of these
/// walks was read off a slider and the arithmetic between them should not have to make a trip
/// through a travel constant to get back to where it started.
fn lerp(from: u16, to: u16, at: f64) -> u16 {
    let at = at.clamp(0.0, 1.0);
    let span = f64::from(to) - f64::from(from);
    (f64::from(from) + span * at)
        .round()
        .clamp(0.0, f64::from(u16::MAX)) as u16
}

/// A pose nudged by the breath and held inside the head's own travel.
fn roved(pose: Pose, pan_deg: f64, tilt_deg: f64) -> Pose {
    Pose::new(
        (pose.pan_deg + pan_deg).clamp(
            0.0,
            <patch::Zq02015 as cortex::qlc_plus::Position>::PAN_RANGE_DEG,
        ),
        (pose.tilt_deg + tilt_deg).clamp(
            0.0,
            <patch::Zq02015 as cortex::qlc_plus::Position>::TILT_RANGE_DEG,
        ),
    )
}

/// A state's brightness with the breath on it, swinging between the two ends it was given.
///
/// Never to zero and never near it: a light that reaches black reads as broken, and one that
/// falls under the lamp's own striking point reads as broken twice — it goes out, and then it
/// comes back, which looks like a fault rather than like breathing.
fn breathed(floor: u8, ceiling: u8, breath: f64) -> u8 {
    let span = f64::from(ceiling) - f64::from(floor);
    (f64::from(floor) + span * breath.clamp(0.0, 1.0)).round() as u8
}

fn main() {
    // Phase carried across frames rather than read off the wall clock, so the breath is
    // continuous through anything that stalls a frame, and wrapped to one period so it stays
    // exact however many days the rig runs.
    let mut phase_s = 0.0f64;

    // The wander runs on the same carried clock, over a full there-and-back cycle. One clock
    // for all four heads: they are sent off together and separate on the way, because they
    // travel at different speeds, which is the whole reason to drive four of them here.
    let mut wander_s = 0.0f64;

    let mut snakes = [
        Snake::new(&geometry::SNAKE_1, SNAKE_1_WANDER),
        Snake::new(&geometry::SNAKE_2, SNAKE_2_WANDER),
        Snake::new(&geometry::SNAKE_3, SNAKE_3_WANDER),
        Snake::new(&geometry::SNAKE_4, SNAKE_4_WANDER),
    ];

    // The mage's own left and right, each flying the pair standing on that side of them, so
    // the arm and the heads it moves are on the same side of the body.
    let mut pairs = [Pair::new("left", [0, 1]), Pair::new("right", [2, 3])];

    // The binding, said once at startup. The camera faces the mage, so the picture is
    // mirrored, and "which heads should move when I raise my left hand" is the one question
    // whose wrong answer looks like everything else working — it is worth being able to check
    // against a line of log rather than against the source.
    for pair in &pairs {
        eprintln!(
            "show: {} arm flies {} and {}",
            pair.side, snakes[pair.heads[0]].head.name, snakes[pair.heads[1]].head.name,
        );
    }

    let (sightings_tx, sightings) = latest::latest(Sighting::default());
    let _eyeball = eyeball::spawn_receiver(sightings_tx);
    let mut since_sighting_log_s = 0.0f64;
    let mut had_vision = false;

    // Counted rather than tested against a timestamp, so the fall back into bored is the same
    // few seconds whether vision died, wedged, or is simply looking at an empty field.
    let mut unseen_s = BORED_AFTER_S;

    // The rig starts bored, and starts already free to leave: the first mage to walk up should
    // not have to wait out a dwell nobody was there for.
    let mut show = Show::Bored;
    let mut held_s = STATE_DWELL_S;

    cortex::run(Rig {
        name: env!("CARGO_PKG_NAME"),
        patch: &patch::PATCH,
        scenes: &scenes::SCENES,
        // The audio features are ignored, deliberately: the mage show is pose-driven and the
        // capture thread runs anyway, because it belongs to the cabinet rather than to either
        // rig.
        show: Box::new(
            move |_features: &AudioFeatures, dt: f64, slots: &mut [u8]| {
                phase_s = (phase_s + dt) % cfg::SPARKLE_BREATH_PERIOD_S;
                let turn = TAU * phase_s / cfg::SPARKLE_BREATH_PERIOD_S;

                // A cosine breath between the floor and the ceiling. It never reaches zero: a
                // light that goes fully dark reads as broken, while one that keeps an ember
                // reads as alive.
                let breath = 0.5 - 0.5 * turn.cos();
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

                // The heads take the same breath between their own two ends, which are
                // nothing like the pinspot's: the pinspot is a bench lamp allowed to fall to an
                // ember, while these are beams on a child and a deep dip would read as the rig
                // losing them.
                let rove_pan_deg = BREATH_ROVE_DEG * turn.sin();
                let rove_tilt_deg = BREATH_ROVE_DEG * turn.cos();

                let sighting = sightings.snapshot();
                let has_vision = sighting.fresh();
                let seen = has_vision && sighting.present;
                unseen_s = if seen { 0.0 } else { unseen_s + dt };
                let bored = unseen_s >= BORED_AFTER_S;

                // Crossing into or out of freshness is logged the moment it happens rather
                // than at the next tick, because the interesting question during bring-up is
                // *when* vision went away.
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
                    } else {
                        eprintln!("eyeball: no sightings — heads wander");
                    }
                }

                // What the picture asks for, which is not always what the rig does: a state
                // has to have stood for its dwell before another can replace it.
                let magic = show == Show::Magic;
                let wanted = if bored {
                    Show::Bored
                } else if hand_up(&sighting, magic).unwrap_or(magic) {
                    Show::Magic
                } else {
                    Show::Attentive
                };

                held_s += dt;
                if wanted != show && held_s >= STATE_DWELL_S {
                    show = wanted;
                    held_s = 0.0;
                    // Logged as it happens, because the person best placed to judge whether
                    // the state changed when they raised their hand is the one waving it, and
                    // they are therefore not reading a screen. The journal answers it after.
                    eprintln!("show: {}", show.label());
                }

                // Both arms steer their own pair whenever anything is flying. One hand up puts
                // the whole rig in magic, and the pair belonging to the hand that stayed down
                // follows it down — which is the arm doing what it was asked, not a pair left
                // behind.
                if show == Show::Magic {
                    for pair in &mut pairs {
                        let arm = match pair.side {
                            "left" => &sighting.arms.left,
                            _ => &sighting.arms.right,
                        };
                        pair.steer(arm);
                    }
                }

                // Every bored head is held at one pose long enough to settle, then sent to the
                // other. What reaches the wire is each slew's own position, never the target,
                // so the step in the target becomes a traverse at that head's bounded speed.
                wander_s = (wander_s + dt) % (2.0 * WANDER_HOLD_S);
                let outbound = wander_s < WANDER_HOLD_S;

                for pair in &pairs {
                    for index in pair.heads {
                        let snake = &mut snakes[index];
                        let head = snake.head.fixture;

                        // Bored is the only state that keeps a head's own speed. The two
                        // watching states are being judged against a kid's own arm, where
                        // slowness reads as a fault rather than as character.
                        let (target, dimmer, color, gobo) = match show {
                            Show::Bored => {
                                snake.slew.set_rate(snake.wander.slew);
                                let pose = if outbound {
                                    snake.wander.to
                                } else {
                                    snake.wander.from
                                };
                                (pose, BORED_DIMMER, WHITE, GOBO_OPEN)
                            }
                            Show::Attentive => {
                                snake.slew.set_rate(ATTENTIVE_SLEW);
                                (
                                    roved(snake.head.attentive.pose(), rove_pan_deg, rove_tilt_deg),
                                    breathed(ATTENTIVE_DIMMER_FLOOR, ATTENTIVE_DIMMER, breath),
                                    WHITE,
                                    GOBO_OPEN,
                                )
                            }
                            Show::Magic => {
                                snake.slew.set_rate(MAGIC_SLEW);
                                (
                                    roved(pair.aim.pose(), rove_pan_deg, rove_tilt_deg),
                                    breathed(MAGIC_DIMMER_FLOOR, MAGIC_DIMMER, breath),
                                    MAGIC_COLOR_RED,
                                    MAGIC_GOBO,
                                )
                            }
                        };

                        aim(head, slots, snake.slew.step(target, dt));
                        head.dimmer.set(slots, dimmer);
                        head.color.set(slots, color);
                        head.gobo.set(slots, gobo);

                        head.strobe
                            .set(slots, patch::Zq02015::STROBE_SHUTTER_OPEN.center());
                        // Both ends of this channel were tried on the rig and neither changed
                        // how the head tracks a stream of positions, so the fast end is chosen
                        // on principle rather than on evidence: the software owns the
                        // interpolation, and a ramp inside the fixture would be a second one we
                        // cannot see.
                        head.motor_speed
                            .set(slots, patch::Zq02015::MOTOR_SPEED_FASTEST);
                        // The channels that must never drift, on values the definition names.
                        head.automatic_mode.park(slots);
                        head.reset.park(slots);
                        head.light_strips
                            .set(slots, patch::Zq02015::LIGHT_STRIPS_OFF.min);
                    }
                }
            },
        ),
    })
}
