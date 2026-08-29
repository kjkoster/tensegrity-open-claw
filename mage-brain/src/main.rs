//! The mage rig: four moving heads, a pinspot and a laser, driven by pose detection from a
//! camera rather than by sound. Nothing in this show listens to the room.
//!
//! The pinspot breathes purple, because it is the bench and bring-up fixture and it says the
//! frame loop is alive whatever the heads are doing. It reuses the claw's breath period,
//! floor and ceiling — one set of numbers in `cortex`, so the two rigs cannot end up
//! breathing differently by accident.
//!
//! The rig runs two states, and they belong to the mage rather than to a head: all four
//! snakes are one creature's attention. **Bored** is nobody in front of the camera — each
//! head walks between two poses at its own speed, white and dim, which is an attract loop and
//! also the only way to see whether the position pipeline is smooth. **Magic** is a mage in
//! front of it, and it takes the whole rig.
//!
//! Being seen is the whole boundary. There is no gesture to get right and nothing to hold
//! above a line, because the arms drive the heads continuously: a mage standing with their
//! arms hanging is already flying them, and the beams go where hanging arms send them.
//!
//! Inside magic each arm works its own pair — the left arm flies snakes 1 and 2 and the right
//! arm 3 and 4 — so a kid who moves one arm moves that pair, and finds the other pair has been
//! following the arm they left hanging all along.
//!
//! The arms are half of it, and they carry one axis: how far up an arm is, is how far up its
//! pair points. Raised arms bring the beams up and back over the mage, hanging arms lay them out
//! over the audience.
//!
//! The other half is not a gesture at all. Where the mage *stands* is what swings the heads
//! sideways, so the two axes hang off two different parts of a body: arms up and down, feet left
//! and right. Nothing a kid does with an arm can slide the beams sideways, and walking cannot
//! change how high they point — which is what an arm driving both of them cost, and why one of
//! them was moved off the arms. Walking is also the mapping nobody has to be told about.
//!
//! There is no geometry in any of it, and nothing measured. Tilt is a channel value off an arm,
//! and pan is interpolated between pan values recorded on the rig, so it gets carried out, set
//! down and switched on.

mod geometry;

use cortex::Rig;
use cortex::audio_features::AudioFeatures;
use cortex::config as cfg;
use cortex::eyeball::{self, Arm, Sighting};
use cortex::latest;
use cortex::moving_head::{Pose, Slew, SlewRate, aim};
use geometry::{Aim, lerp};
use std::f64::consts::TAU;

// ── Tunables ─────────────────────────────────────────────────────────────────
// This rig's numbers live in this rig's crate. `cortex::config` describes the *cabinet* —
// the serial device, the audio interface, the universe — and a value that only means
// anything on the mage's field would be a rig fact sitting in the shared half, where the claw
// would carry it around for nothing and the two could quietly come to disagree.
//
// Only what is a choice about the show is here. What a head *is* — how far it pans, which
// end of its speed channel is fast, where its dead bands sit — comes from the fixture
// definition through the generated patch, so none of it is repeated here to go stale.

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

// How long the field has to stay empty before the heads give up and go back to wandering.
// Long enough to ride out a mage who turns side-on to the camera or steps behind another
// child, short enough that an empty field stops looking attended.
//
// This is the whole of the rig's hysteresis, and it is enough because of how one-sided it is:
// the counter resets on any frame with a mage in it, so the boundary falls into magic in an
// instant and back out only after five clear seconds. A boundary that cannot chatter wants no
// dwell on top of it.
const BORED_AFTER_S: f64 = 5.0;

// Bored is watched from across a field with nobody in it, so it is lit to be visible rather
// than to land on anyone.
const BORED_DIMMER: u8 = 0x40;

// White and an open gate, which is what bored looks like: nothing in the beam at all, so the
// colour and the pattern belong to magic alone and arrive with it.
const WHITE: u8 = 0x00;
const GOBO_OPEN: u8 = 0x00;

// Magic is the state the show is for, so it is the bright one, and it is the only one with a
// colour and a pattern in the beam — which is what makes a mage walking up read as a change
// in kind rather than a change in aim.
//
// The floor is an absolute value rather than a fraction of the peak, because what matters is
// where this fixture's lamp stops striking, and that is a place on the channel rather than a
// proportion of whatever the show happened to ask for.
const MAGIC_DIMMER: u8 = 0xc8;
const MAGIC_DIMMER_FLOOR: u8 = 0x90;
const MAGIC_COLOR_RED: u8 = 0x10;
const MAGIC_GOBO: u8 = 0x30;

// Tilt against the arm's elevation: an arm hanging straight down lays the beam out over the
// audience, and raising it lifts the beam with it, back up and over the mage. The beam follows
// the hand, which is the reading a kid arrives with — the ends of the channel are the same two
// as before, and only which arm posture reaches them has changed.
//
// Both ends are chosen rather than mechanical: these heads sit at lens height, so the
// audience-facing end is the one that has to stop short, and it is the end a mage standing
// still now holds. The clamp is what enforces it, not this pair of numbers.
const MAGIC_TILT_ARM_DOWN: u16 = 0x0000;
const MAGIC_TILT_ARM_UP: u16 = 0xa000;

// The range an arm is measured on, and what the daemon's angles arrive as: 0° hanging straight
// down to 180° straight up, per segment and therefore for the two of them averaged.
const ARM_RANGE_DEG: f64 = 180.0;

// `patch` and `scenes`, generated from mage.qxw by the ingest. The workspace and the `.qxf`
// definitions are the source of truth; nothing about the addressing is hand-maintained.
// Generating them into this crate rather than the shared one is also what keeps the rigs
// apart: the claw's fixtures do not exist in this binary to be reached for. The camera is not
// a DMX device and appears in neither.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

/// One head with everything that drives it: how it wanders when bored, and where it actually
/// is right now.
///
/// Assembled per head and never by index, so a head cannot end up wearing another head's
/// wander — which would walk it a span nobody chose, at a speed nobody picked for it, while
/// every number involved still looked reasonable.
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
/// them wandering while the other two fly reads as a rig with a fault rather than as a trick.
/// Which arm moves which pair stays a per-pair question — the state says whether anybody is
/// flying at all.
#[derive(Clone, Copy, PartialEq)]
enum Show {
    Bored,
    Magic,
}

impl Show {
    fn label(self) -> &'static str {
        match self {
            Self::Bored => "bored — no mage",
            Self::Magic => "magic — a mage is here",
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
    /// The last tilt the arm produced. Held rather than recomputed when the arm blanks: an arm
    /// pointed at the lens has no readable elevation, and following the noise is worse than
    /// holding still.
    tilt: u16,
}

impl Pair {
    fn new(side: &'static str, heads: [usize; 2]) -> Self {
        Self {
            side,
            heads,
            // The arm-up end until an arm has actually been read, which is the harmless one:
            // magic can begin on a frame whose arms all blanked, and a pair that starts where a
            // hanging arm would put it starts pointed at the audience on no evidence at all.
            tilt: MAGIC_TILT_ARM_UP,
        }
    }

    /// How far up this arm is holding its pair, or the last answer if the picture cannot say.
    ///
    /// The whole arm and not either segment of it: the two are averaged, which on a straight
    /// arm is the arm's own elevation and on a bent one falls between the halves — a folded
    /// elbow puts the beams between where each half points, which is what a bent arm looks
    /// like it should do.
    ///
    /// Whichever segments the picture could read are what gets averaged, because the daemon's
    /// gate is per segment and half an arm still says roughly how far up the arm is. With
    /// neither of them readable the tilt stays where it was.
    fn steer(&mut self, arm: &Arm) {
        // The forearm arrives signed — which side of the body it swung out to — and magic no
        // longer asks: only how far off hanging it is, which is the scale the upper arm is
        // already on.
        let mut sum = 0.0;
        let mut segments = 0u32;
        for angle in [arm.upper, arm.fore.map(f64::abs)].into_iter().flatten() {
            sum += angle;
            segments += 1;
        }
        if segments == 0 {
            return;
        }
        self.tilt = lerp(
            MAGIC_TILT_ARM_DOWN,
            MAGIC_TILT_ARM_UP,
            sum / f64::from(segments) / ARM_RANGE_DEG,
        );
    }
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

    // The rig starts bored, which is also what it does if the eyeball never comes up at all:
    // an empty field and a dead daemon are the same observation from here.
    let mut show = Show::Bored;

    // Where the mage is standing, held across frames the way a pair holds its tilt. One number
    // for the whole rig rather than one per pair: there is one mage, and four heads disagreeing
    // about where they are standing is not a thing the field can produce. The middle until
    // somebody has been seen there, which is where the heads already point.
    let mut across = 0.0f64;

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

                // The heads take the same breath on the dimmer alone, between their own two
                // ends, which are nothing like the pinspot's: the pinspot is a bench lamp
                // allowed to fall to an ember, while these are beams on a child and a deep dip
                // would read as the rig losing them. Nothing breathes on the aim — where a head
                // points is what a mage put there, and only that.

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

                // Being seen is the whole of it. Nothing here asks what a mage is doing with
                // their arms, because the arms are already steering and a mage standing still
                // steers the heads to where standing still points them.
                let wanted = if bored { Show::Bored } else { Show::Magic };

                if wanted != show {
                    show = wanted;
                    // Logged as it happens, because the person best placed to judge whether
                    // the state changed when they walked up is the one standing there, and
                    // they are therefore not reading a screen. The journal answers it after.
                    eprintln!("show: {}", show.label());
                }

                // Everything that steers steers whenever anything is flying, an arm hanging at
                // a mage's side and a mage standing still included: that pair points where a
                // hanging arm points and every head sits where a standing mage puts it, which is
                // the rig doing what it was asked rather than waiting to be told again.
                if show == Show::Magic {
                    // Held rather than zeroed when the torso blanks: a mage turned side-on is
                    // still standing where they were standing, and swinging every head back to
                    // the middle is the one answer that is certainly wrong.
                    if let Some(seen) = sighting.body.across {
                        across = seen;
                    }
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

                        // Bored keeps a head's own speed and magic does not: a beam under an
                        // arm is judged against that arm, where slowness reads as a fault
                        // rather than as character.
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
                            Show::Magic => {
                                snake.slew.set_rate(MAGIC_SLEW);
                                (
                                    // Pan off this head's own recorded track, tilt off the arm
                                    // flying this pair: the two axes meet here and nowhere else.
                                    Aim::new(snake.head.pan.at(across), pair.tilt).pose(),
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
