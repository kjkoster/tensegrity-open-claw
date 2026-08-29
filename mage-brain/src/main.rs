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
//! Brightness is the third of it, and the one nobody has to be shown: how far an arm has just
//! travelled is how bright its pair burns. Still arms sit at an ember, a sweep takes its two
//! heads to full, and the flare falls away on its own. Posture aims the beams and travel lights
//! them, so the two are read off the same arm without ever competing for it.
//!
//! Travel and not speed, which is the one thing here that had to be learned on the rig rather
//! than reasoned out. The estimator is never still: a motionless mage produces frame-to-frame
//! change big enough to swamp any gesture, so anything measured between two frames measures the
//! estimator. Jitter is fast and goes nowhere, and distance covered is what tells them apart.
//!
//! There is no geometry in any of it, and nothing measured. Tilt is a channel value off an arm,
//! pan is interpolated between pan values recorded on the rig, and brightness is an envelope on
//! how far an angle has been, so it gets carried out, set down and switched on.

mod geometry;

use cortex::Rig;
use cortex::audio_features::AudioFeatures;
use cortex::config as cfg;
use cortex::envelope::Envelope;
use cortex::eyeball::{self, Arm, Sighting};
use cortex::latest;
use cortex::moving_head::{Pose, Slew, SlewRate, aim};
use geometry::{Aim, lerp};
use std::collections::VecDeque;
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
const BORED_AFTER_S: f64 = 5.0;

// And how long somebody has to be there before the rig believes it. A handful of pose frames.
//
// This was zero, on the reasoning that a one-sided boundary cannot chatter: the vacancy counter
// resets on any frame with a mage in it, so the rig fell into magic instantly and back out only
// after five clear seconds. The reasoning was sound and the premise was wrong. Presence is one
// bit from a pose estimator, and on an empty field it is occasionally, briefly, true — the
// daemon calls a mage present when any single landmark clears its confidence floor, and a bush
// in the right light manages that for a frame. One such frame bought five seconds of magic over
// an empty field, and the rig spent its afternoon announcing mages nobody could see.
//
// So the boundary is dwelled at both ends, and asymmetrically, because the two mistakes cost
// different things: missing a mage for a fifth of a second is invisible to the mage, and
// inventing one is the rig visibly talking to nobody. Long enough that no single frame can do
// it, short enough that a kid walking up never notices it happening.
const MAGIC_AFTER_S: f64 = 0.3;

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
// where this fixture's own dimming stops being smooth, and that is a place on the channel rather
// than a proportion of whatever the show happened to ask for.
//
// Seven eighths of the channel between the ends, arrived at by trying less twice. Every earlier
// floor was a resting value that looked right standing beside a head with nobody moving, and each
// one left the flare with nowhere to travel: against a beam already well up, a wave is a change
// somebody has to be told to look for. The resting value is not the thing to get right — the
// difference is, and on a field in daylight the difference has to be most of the channel.
//
// Still not zero. A head that goes out has stopped being a head pointed at you, and the rig loses
// the thing a kid is looking along; this is an ember that says the beam is still there and still
// aimed.
const MAGIC_DIMMER_FLARE: u8 = 0xff;
const MAGIC_DIMMER_FLOOR: u8 = 0x20;
const MAGIC_COLOR_RED: u8 = 0x10;
const MAGIC_GOBO: u8 = 0x30;

// What the brightness answers to: how fast the arm flying this pair is moving. Still arms sit on
// the floor above, a fast one takes the pair to the ceiling, and it falls back on its own.
//
// Movement rather than posture, because posture is already spent — elevation is the tilt and
// where the mage stands is the pan, and a third mapping off a body part would be a third thing to
// find. Speed is the one thing a kid does that nothing else here is reading, and it is the
// mapping they find without being told: they wave, and the rig answers.
//
// How far the arm went, and not how fast the picture changed. This is the whole of why the first
// two attempts read as nothing: speed between consecutive frames is mostly the estimator, which
// is never still — a mage standing motionless produced hundreds of degrees per second, and a
// gesture could not be told from the noise it arrived in. Jitter is fast and goes nowhere.
// Movement goes somewhere, which is a thing that can be measured and cannot be faked by an
// unsteady landmark.
//
// So the input is the widest excursion either segment made inside a short window: an arm that
// wobbled around one place has a small one however violently it wobbled, and an arm that swept
// has a large one however smoothly it went.
//
// Both ends are degrees of that excursion, and the three numbers here only mean anything
// together — the window and the trim decide what a gesture is worth before these two decide what
// it is worth in light. Measured against the arm, they sit where a mage standing still lands
// under quiet and a raised arm lands on full, with a half-hearted sweep visibly short of it.
const MAGIC_FLARE_QUIET_DEG: f64 = 15.0;
const MAGIC_FLARE_FULL_DEG: f64 = 70.0;

// How far back the excursion looks, and the number that does most of the separating. Jitter's
// excursion is bounded by how far a landmark wanders and does not grow with the window; a
// gesture's keeps growing until the whole of it is inside. So a window shorter than a gesture
// costs the gesture and not the noise — at two thirds of a second an arm sweeping up over most
// of a second only ever had two thirds of its travel in view, and came out worth about the same
// as a badly behaved landmark.
//
// A second holds the whole of an ordinary gesture and still belongs to what a kid is doing
// rather than to what they did. It is also what pays for the trim below: the wider the window,
// the more frames there are to throw the worst of away from.
const MAGIC_FLARE_WINDOW_S: f64 = 1.0;

// How many samples at each end of the window are discarded before the excursion is measured.
// The estimator does not only jitter: it occasionally flips a landmark outright, and one frame
// with the elbow somewhere impossible is a hundred degrees of travel that never happened.
//
// Two rather than one, because the flips do not arrive alone — two inside a second is ordinary,
// and trimming one from each end leaves the second one standing as the whole excursion. Two
// costs a real sweep a few degrees at each end and nothing else, because a sweep's samples are
// spread across the window rather than piled at its extremes.
const MAGIC_FLARE_TRIM: usize = 2;

// Up in a tenth of a second, because the flare has to land while the arm is still moving — a
// beam that brightens after the gesture reads as a coincidence rather than as an answer.
const MAGIC_FLARE_ATTACK_S: f64 = 0.08;

// And down over rather longer than it went up: the flare should be gone before a kid looks for
// it, but not so fast that the beam is dark again while their arm is still coming to rest.
//
// The floor under this number is the pose stream, not taste. A value is held until the next frame
// lands, so a release anywhere near that gap renders a held gesture as a stutter, and a stutter
// reads as a fault rather than as responsiveness. This rig manages twelve to fifteen frames a
// second, which puts the gap near seventy milliseconds and leaves this several times clear of
// it — a couple of times the gap is the tightest it should ever be tuned.
// Shorter than the release the speed version wanted, because the window above is already most of
// the fall: the excursion does not drop until the samples carrying it age out, so a gesture keeps
// its brightness for the rest of the window and only then lets go. The two together put a wave
// back on the floor about a second after the arm stops, which is a decay somebody can see happen.
//
// A blanked arm needs nothing special here either. It contributes no sample, the samples it left
// age out on their own, and an arm that stays gone for the whole window has no excursion left to
// hold its pair up.
const MAGIC_FLARE_DECAY_S: f64 = 0.25;

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

/// One pose frame's worth of an arm, kept only long enough to be measured against the others.
///
/// The two segments stay apart rather than being reduced to one number here. They answer
/// different gestures — an arm raised is the upper segment travelling, an arm waved sideways at
/// head height is the forearm travelling while the upper barely moves — and averaging them first
/// would let one cancel the other and lose both.
struct Reach {
    age_s: f64,
    upper: Option<f64>,
    fore: Option<f64>,
}

/// How far an arm has actually been in the last little while.
///
/// The measurement the whole flare rests on, and the reason it is a window rather than a
/// difference between two frames: an estimator that cannot hold a landmark still produces
/// enormous frame-to-frame change and no travel at all, so anything derived from consecutive
/// frames measures the estimator. Distance covered inside a window separates them, because
/// jitter comes back to where it started and a gesture does not.
struct Travel {
    samples: VecDeque<Reach>,
}

impl Travel {
    fn new() -> Self {
        Self {
            samples: VecDeque::new(),
        }
    }

    fn clear(&mut self) {
        self.samples.clear();
    }

    /// Takes one pose frame, and forgets whatever has fallen out of the back of the window.
    ///
    /// Ages everything whether or not this frame had an arm in it, so a blanked arm costs the
    /// window a sample rather than stopping its clock — an arm that has genuinely gone empties
    /// the window and stops holding its pair up.
    fn observe(&mut self, arm: &Arm, gap_s: f64) {
        for sample in &mut self.samples {
            sample.age_s += gap_s;
        }
        while self
            .samples
            .front()
            .is_some_and(|sample| sample.age_s > MAGIC_FLARE_WINDOW_S)
        {
            self.samples.pop_front();
        }
        if arm.upper.is_some() || arm.fore.is_some() {
            self.samples.push_back(Reach {
                age_s: 0.0,
                upper: arm.upper,
                fore: arm.fore,
            });
        }
    }

    /// The widest excursion either segment made across the window, in degrees, or `None` where
    /// there is not enough of an arm in it to say.
    ///
    /// The larger of the two rather than their sum or their mean: a gesture is usually one
    /// segment's, and a segment that stayed put should not dilute the one that moved.
    fn excursion_deg(&self) -> Option<f64> {
        let upper = self.spread(|sample| sample.upper);
        let fore = self.spread(|sample| sample.fore);
        match (upper, fore) {
            (Some(upper), Some(fore)) => Some(upper.max(fore)),
            (found, None) | (None, found) => found,
        }
    }

    /// One segment's spread across the window, with the extremes at both ends thrown away.
    ///
    /// `None` until there are enough samples left after trimming to have a spread at all, which
    /// is what keeps a pair dark through the first frames of a magic rather than flaring on a
    /// window holding one reading.
    fn spread(&self, of: impl Fn(&Reach) -> Option<f64>) -> Option<f64> {
        let mut found: Vec<f64> = self.samples.iter().filter_map(of).collect();
        if found.len() < 2 * MAGIC_FLARE_TRIM + 2 {
            return None;
        }
        found.sort_by(f64::total_cmp);
        Some(found[found.len() - 1 - MAGIC_FLARE_TRIM] - found[MAGIC_FLARE_TRIM])
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
    /// Where this arm has been across the last window, which is what the flare is measured from.
    travel: Travel,
    /// The excursion that window currently holds, or `None` where there is not enough arm in it
    /// to say. Kept rather than consumed on the spot because it is also what the eyeball log line
    /// prints, and the two thresholds above are tuned by reading it — where an arm that stayed
    /// put and an arm nobody can find have to be different lines, or the number that says the
    /// dead zone is right is the same number that says the estimator has lost the mage.
    excursion_deg: Option<f64>,
    /// The brightness that travel is currently worth, rising fast and falling slowly.
    flare: Envelope,
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
            travel: Travel::new(),
            excursion_deg: None,
            flare: Envelope::new(MAGIC_FLARE_ATTACK_S, MAGIC_FLARE_DECAY_S),
        }
    }

    /// Starts a magic on this pair: no arm remembered, and dark rather than wherever the last
    /// mage left it.
    ///
    /// Without this the first sighting of a new mage would be differenced against an arm seen
    /// minutes ago, over a gap of one frame, and every kid would be greeted by a flare they did
    /// not earn.
    fn begin(&mut self) {
        self.travel.clear();
        self.excursion_deg = None;
        self.flare.reset();
    }

    /// Everything this arm says, taken once per frame.
    ///
    /// `gap_s` is how long it has been since the last sighting, and `Some` only on the frame a
    /// new one arrives: the angles cannot change between sightings, so aiming and measuring are
    /// pose-rate work. The flare is stepped every frame regardless, because a decay rendered at
    /// pose rate would come down in visible steps.
    fn steer(&mut self, arm: &Arm, gap_s: Option<f64>, dt: f64) {
        if let Some(gap) = gap_s {
            self.aim(arm);
            self.travel.observe(arm, gap);
            self.excursion_deg = self.travel.excursion_deg();
        }
        self.flare
            .step(flare_of(self.excursion_deg.unwrap_or(0.0)), dt);
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
    fn aim(&mut self, arm: &Arm) {
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

/// An excursion for the log line, or a dash where there was no arm to measure.
fn travel_or_dash(excursion_deg: Option<f64>) -> String {
    excursion_deg.map_or_else(|| "—".to_string(), |degrees| format!("{degrees:.0}°"))
}

/// What an excursion is worth as brightness, 0 at the quiet end and 1 at the full one.
fn flare_of(excursion_deg: f64) -> f64 {
    ((excursion_deg - MAGIC_FLARE_QUIET_DEG) / (MAGIC_FLARE_FULL_DEG - MAGIC_FLARE_QUIET_DEG))
        .clamp(0.0, 1.0)
}

/// A brightness between the two ends it was given, `at` running 0 to 1.
///
/// Never to zero and never near it: a light that reaches black reads as broken, and one that
/// falls under the lamp's own striking point reads as broken twice — it goes out, and then it
/// comes back, which looks like a fault rather than like a rig at rest.
fn lit(floor: u8, ceiling: u8, at: f64) -> u8 {
    let span = f64::from(ceiling) - f64::from(floor);
    (f64::from(floor) + span * at.clamp(0.0, 1.0)).round() as u8
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
    //
    // Two counters and not one signed number: each is a run of frames that all said the same
    // thing, and either resets the moment a frame disagrees. A single frame of noise therefore
    // costs the other counter its run and nothing more.
    let mut unseen_s = BORED_AFTER_S;
    let mut seen_s = 0.0f64;

    // The rig starts bored, which is also what it does if the eyeball never comes up at all:
    // an empty field and a dead daemon are the same observation from here.
    let mut show = Show::Bored;

    // Where the mage is standing, held across frames the way a pair holds its tilt. One number
    // for the whole rig rather than one per pair: there is one mage, and four heads disagreeing
    // about where they are standing is not a thing the field can produce. The middle until
    // somebody has been seen there, which is where the heads already point.
    let mut across = 0.0f64;

    // The pose frame last taken, and how long ago it landed. Both belong to the rig rather than
    // to a pair: one sighting carries both arms, so both pairs age their windows by the same
    // interval or neither does.
    let mut last_seq = 0u64;
    let mut since_pose_s = 0.0f64;

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

                // The breath is the pinspot's alone. The heads answer to the mage: their aim is
                // where an arm put it and their brightness is how fast that arm is moving, and
                // neither of those is a clock.

                let sighting = sightings.snapshot();

                // How long since the pose frame before this one, and `Some` only on the frame a
                // new one lands. Accumulated from the show's own `dt` rather than read off
                // either end's clock: it is the interval a speed gets divided by, and a sighting
                // the show never saw is then covered by the gap rather than lost from it.
                since_pose_s += dt;
                let pose_gap_s = (sighting.seq != last_seq).then(|| {
                    last_seq = sighting.seq;
                    let gap = since_pose_s;
                    since_pose_s = 0.0;
                    gap
                });
                let has_vision = sighting.fresh();
                let seen = has_vision && sighting.present;
                unseen_s = if seen { 0.0 } else { unseen_s + dt };
                seen_s = if seen { seen_s + dt } else { 0.0 };

                // Crossing into or out of freshness is logged the moment it happens rather
                // than at the next tick, because the interesting question during bring-up is
                // *when* vision went away.
                since_sighting_log_s += dt;
                if has_vision != had_vision || since_sighting_log_s >= cfg::EYEBALL_LOG_INTERVAL_S {
                    since_sighting_log_s = 0.0;
                    had_vision = has_vision;
                    if has_vision {
                        // The excursions ride along on the line that already exists, because
                        // they are what the two flare thresholds get tuned against and a rig
                        // being tuned in a field has a journal and no browser.
                        eprintln!(
                            "eyeball: seq {} — {} @ {:.1} Hz, mage {}, travel {} {} {} {}",
                            sighting.seq,
                            sighting.source,
                            sighting.fps,
                            if sighting.present { "seen" } else { "not seen" },
                            pairs[0].side,
                            travel_or_dash(pairs[0].excursion_deg),
                            pairs[1].side,
                            travel_or_dash(pairs[1].excursion_deg),
                        );
                    } else {
                        eprintln!("eyeball: no sightings — heads wander");
                    }
                }

                // Being seen is the whole of it. Nothing here asks what a mage is doing with
                // their arms, because the arms are already steering and a mage standing still
                // steers the heads to where standing still points them.
                //
                // Only a run long enough to have meant something moves the rig, and everything
                // between the two thresholds leaves it where it is. The counters cannot both be
                // running, so the order of these two tests is not a tie-break: a frame that sees
                // a mage has already reset the other one to zero.
                let wanted = if seen_s >= MAGIC_AFTER_S {
                    Show::Magic
                } else if unseen_s >= BORED_AFTER_S {
                    Show::Bored
                } else {
                    show
                };

                if wanted != show {
                    show = wanted;
                    if show == Show::Magic {
                        for pair in &mut pairs {
                            pair.begin();
                        }
                    }
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
                        pair.steer(arm, pose_gap_s, dt);
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
                                    lit(MAGIC_DIMMER_FLOOR, MAGIC_DIMMER_FLARE, pair.flare.value()),
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
