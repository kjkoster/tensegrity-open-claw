//! The world frame a rig's geometry is stated in, and the reductions everything above it
//! needs: where one point lies as seen from another, and how far away it is.
//!
//! Right-handed, metres, degrees, **+Z up**. Where the origin sits and which way +X points
//! belong to the rig, not to this module: each rig picks them once against something physical,
//! writes them down beside its own measurements, and never re-derives them. What is fixed here
//! is only what a shared solver and a shared IK have to agree on — **+Y is Z × X**, a bearing
//! runs counter-clockwise from +X about +Z, and an elevation runs from the horizontal plane,
//! positive up.
//!
//! None of those angles has anything to do with a fixture's own pan and tilt, which are
//! unreferenced and start wherever the head happened to be clamped. Turning one into the other
//! is what a head's mount describes, and that is the only place the two meet.
//!
//! Metres and degrees throughout, with the suffix on the angles only. The frame has one
//! length unit, and repeating it on every coordinate of every type here would say nothing —
//! while degrees against radians is the mistake that compiles.

use crate::moving_head::{Pose, SlewRate};
use crate::qlc_plus::Position;

/// A point in the world frame.
#[derive(Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A direction in the world frame, as seen from somewhere in it.
#[derive(Clone, Copy, PartialEq)]
pub struct Direction {
    pub bearing_deg: f64,
    pub elevation_deg: f64,
}

impl Point {
    /// Wherever the rig anchored its frame.
    pub const ORIGIN: Self = Self::new(0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Where `target` lies as seen from here.
    ///
    /// A point asked about itself gives zero for both angles rather than a NaN. There is no
    /// direction to hand back, and a head aimed at its own position is a fault upstream — a
    /// quiet zero surfaces it at the beam, where it is visible, instead of poisoning every
    /// number downstream of it.
    pub fn toward(self, target: Point) -> Direction {
        let east = target.x - self.x;
        let north = target.y - self.y;
        let up = target.z - self.z;
        Direction {
            bearing_deg: north.atan2(east).to_degrees(),
            // Against the horizontal run rather than the straight-line distance: the run goes
            // to zero only for a target directly overhead, where atan2 still answers ±90,
            // while dividing by a length would need that case handled by hand.
            elevation_deg: up.atan2(east.hypot(north)).to_degrees(),
        }
    }

    pub fn distance_to(self, other: Point) -> f64 {
        (other.x - self.x)
            .hypot(other.y - self.y)
            .hypot(other.z - self.z)
    }
}

impl Direction {
    pub const fn new(bearing_deg: f64, elevation_deg: f64) -> Self {
        Self {
            bearing_deg,
            elevation_deg,
        }
    }

    /// The direction as a unit vector one metre out.
    ///
    /// Angles are what a head and a plan speak, but comparing two of them by subtracting
    /// bearings lies near the vertical, where a whole turn of bearing is no distance at all.
    /// The one honest measure of how far apart two directions are is the angle between their
    /// vectors, so the fit does its arithmetic here.
    pub fn unit(self) -> Point {
        let bearing = self.bearing_deg.to_radians();
        let above_horizon = self.elevation_deg.to_radians();
        let along = above_horizon.cos();
        Point::new(
            along * bearing.cos(),
            along * bearing.sin(),
            above_horizon.sin(),
        )
    }

    /// How far this direction is from `other`, as the angle between them.
    pub fn angle_to(self, other: Direction) -> f64 {
        let (here, there) = (self.unit(), other.unit());
        let dot = here.x * there.x + here.y * there.y + here.z * there.z;
        dot.clamp(-1.0, 1.0).acos().to_degrees()
    }
}

/// Which way a head's own axis runs against the world.
///
/// Not a signed number, because this is a two-position switch in the fixture's own menu and
/// nothing else: a head with `rPAN` set the other way round tracks DMX perfectly and lands
/// nowhere near where the geometry says, which is a fault that reads as a broken solver. A
/// float able to hold 0.0 or 3.7 would be a third and fourth way for that to happen.
#[derive(Clone, Copy, PartialEq)]
pub enum Sense {
    Forward,
    Reversed,
}

impl Sense {
    fn applied(self, deg: f64) -> f64 {
        match self {
            Self::Forward => deg,
            Self::Reversed => -deg,
        }
    }
}

/// How far a head's axes run, as its own definition declares them.
///
/// Beside a mount rather than inside one: how far a head can turn is a fact about the model,
/// which arrives from its `.qxf` and is the same on every field, while a mount is measured
/// fresh every deployment. A rental changes the one without touching the other.
#[derive(Clone, Copy)]
pub struct Travel {
    pub pan_deg: f64,
    pub tilt_deg: f64,
}

impl Travel {
    pub fn of<F: Position>() -> Self {
        Self {
            pan_deg: F::PAN_RANGE_DEG,
            tilt_deg: F::TILT_RANGE_DEG,
        }
    }
}

/// Where a head stands in the world and which way it looks — the five numbers that turn its
/// unreferenced pan and tilt into a direction over the field.
///
/// The pan axis is taken to be vertical, which is the stand being levelled. Solving the two
/// further angles that would describe a tipped base needs more markers and buys what a bubble
/// level already gives; an off-level stand instead shows up as a fit whose residuals will not
/// come down, which points at the stand rather than at the arithmetic.
///
/// Setup prescribes most of this — a base flat at a known height, turned a known way — but
/// prescribing is not knowing. Ground is not flat and a nominal is not a measurement, so these
/// stay fitted values, and a fit that lands far from what setup asked for is reporting a
/// mounting fault rather than a number worth keeping.
#[derive(Clone, Copy)]
pub struct HeadMount {
    pub position: Point,
    /// Where the beam leaves with both channels at zero.
    pub zero: Direction,
    pub pan: Sense,
    pub tilt: Sense,
}

impl HeadMount {
    /// Which way the beam leaves the head at `pose`.
    ///
    /// Built as a vector and reduced back, rather than by adding the pose to [`Self::zero`]
    /// angle by angle, because 270° of tilt lets the beam travel over the top: past vertical
    /// it comes down the far side, pointing a half-turn away from the bearing it was panned
    /// to. As a vector that reversal is just a negative horizontal component, and the
    /// reduction reports it without being told; as two added angles it is a special case
    /// waiting to be forgotten, in the one function the whole calibration is fitted against.
    pub fn direction(&self, pose: Pose) -> Direction {
        let turned = Direction::new(
            self.zero.bearing_deg + self.pan.applied(pose.pan_deg),
            self.zero.elevation_deg + self.tilt.applied(pose.tilt_deg),
        );
        Point::ORIGIN.toward(turned.unit())
    }

    /// Every pose within `travel` whose beam lands on `target`, in no particular order.
    ///
    /// A head can hit a point more than one way, and this returns all of them and chooses
    /// none — the choice belongs to whatever knows where the head is now and what it is
    /// allowed to sweep on the way, and folding it in here would hide it from both.
    ///
    /// Two things make the answer plural. Tilt can carry the beam over the top, so a head can
    /// pan a half-turn *away* from a target and lean back onto it. And pan runs past a full
    /// revolution on these heads, so a bearing reached at one pan value is reached again 360°
    /// along. Four poses is the most the two together can produce; an unreachable target
    /// yields none, which is not an error and is exactly what a limit on the mechanism looks
    /// like from here.
    pub fn poses_for(&self, travel: Travel, target: Point) -> impl Iterator<Item = Pose> {
        let want = self.position.toward(target);
        let (zero, pan, tilt) = (self.zero, self.pan, self.tilt);

        // Pan to the bearing and tilt to the elevation; or pan a half-turn away and tilt past
        // vertical, which arrives at the same place upside down.
        let ways = [
            (want.bearing_deg, want.elevation_deg),
            (want.bearing_deg - 180.0, 180.0 - want.elevation_deg),
        ];

        ways.into_iter().flat_map(move |(bearing, above_horizon)| {
            // A sense is its own inverse, so the same application that turned the head's
            // degrees into the world's turns them back.
            let pan_deg = pan.applied(bearing - zero.bearing_deg);
            let tilt_deg = tilt.applied(above_horizon - zero.elevation_deg);
            revolutions(pan_deg, travel.pan_deg).flat_map(move |pan_deg| {
                revolutions(tilt_deg, travel.tilt_deg)
                    .map(move |tilt_deg| Pose::new(pan_deg, tilt_deg))
            })
        })
    }
}

/// One head's standing answer to "which of the ways to hit that point are we using".
///
/// A head asked for a target every frame, and re-solving freely, will swing a half-turn and
/// back as the target wanders across the line where two of its poses are equally close. So the
/// answer is anchored: each frame's pose is the one nearest where the head already stands,
/// which holds a branch for exactly as long as it stays continuous — the tracked branch is a
/// few degrees off while the others sit a half-turn or a full turn away, and cannot win by
/// accident.
///
/// Nearest to the head rather than to the last answer, which is why nothing here remembers a
/// pose. The head's position is the truth about which branch it is on, it is already kept one
/// layer down by the rate limiter, and a second copy would be one that goes stale the moment
/// anything else moves the head — a metronome, a scene, a hand on a console — leaving the next
/// choice judged from somewhere the head has not been in minutes.
///
/// A target out of reach changes nothing: the head is told to stay where it is, which is the
/// only honest answer and quieter than snapping to a default.
///
/// Nearness is travel *time*, and the ranking needs no slew rate to measure it: pan and tilt
/// are bounded at one rate per head, so a move takes its larger axis over that rate, and a
/// shared divisor cannot change an ordering. The rate is held anyway, because how long a
/// forced switch takes is a duration rather than an ordering, and the beam has to stay out
/// for it. Should the two axes ever get rates of their own, both uses have to learn about it.
pub struct Chooser {
    mount: HeadMount,
    travel: Travel,
    rate: SlewRate,
    dark_s: f64,
}

/// Where a head should point and whether it may be seen getting there.
#[derive(Clone, Copy)]
pub struct Beam {
    pub pose: Pose,
    /// A multiplier on whatever brightness the show wanted, not a brightness of its own. The
    /// show keeps deciding how bright the beam is; this only decides whether it exists.
    pub lit: f64,
}

/// What counts as changing branch rather than tracking one.
///
/// A tracked branch moves by as much as the target moved and no more, which at frame rate is
/// a fraction of a degree; the alternatives sit a half-turn or a full turn away. Anything in
/// between is not a case that arises, so the threshold sits well clear of both.
const BRANCH_SWITCH_DEG: f64 = 20.0;

impl Chooser {
    pub fn new(mount: HeadMount, travel: Travel, rate: SlewRate) -> Self {
        Self {
            mount,
            travel,
            rate,
            dark_s: 0.0,
        }
    }

    /// Whether any pose in this head's travel lands on `target`.
    ///
    /// For the operator asking a question, not for the frame loop, which learns the same thing
    /// by the beam holding still.
    pub fn reaches(&self, target: Point) -> bool {
        self.mount.poses_for(self.travel, target).next().is_some()
    }

    /// The pose to drive toward for `target`, given where the head stands now.
    ///
    /// A branch change comes out of here dark. The swing it costs is up to a half-turn of pan
    /// at a speed chosen for grace, which is many seconds of a lit beam crossing everything
    /// between the two answers — a move nobody asked for, aimed at nothing, and the one moment
    /// a rig full of intent looks like a rig with a fault. So the beam goes out for as long as
    /// the swing takes and comes back when the head is there.
    pub fn toward(&mut self, at: Pose, target: Point, dt: f64) -> Beam {
        let nearest = self
            .mount
            .poses_for(self.travel, target)
            .min_by(|one, other| travel_between(at, *one).total_cmp(&travel_between(at, *other)));

        let jump = nearest.map_or(0.0, |pose| travel_between(at, pose));
        if jump > BRANCH_SWITCH_DEG {
            self.dark_s = jump / self.rate.deg_s();
        }
        self.dark_s = (self.dark_s - dt).max(0.0);

        Beam {
            pose: nearest.unwrap_or(at),
            lit: if self.dark_s > 0.0 { 0.0 } else { 1.0 },
        }
    }
}

/// How long a head takes to get from one pose to the other, in units of its own slew rate.
///
/// The larger axis rather than the sum or the diagonal: the motors are independent and run at
/// once, so a move costs what its slower half costs and the other axis finishes early.
fn travel_between(from: Pose, to: Pose) -> f64 {
    (to.pan_deg - from.pan_deg)
        .abs()
        .max((to.tilt_deg - from.tilt_deg).abs())
}

/// One recorded calibration point: a head driven onto a marker by hand, and where that marker
/// is.
#[derive(Clone, Copy)]
pub struct Observation {
    pub pose: Pose,
    pub at: Point,
}

/// A mount recovered from observations, and how well it explains them.
pub struct Solution {
    pub mount: HeadMount,
    /// The root mean square of [`residual_deg`] over every observation. It is in the same
    /// degrees the beam misses by, so it can be compared against a beam width rather than
    /// against a previous run.
    pub rms_deg: f64,
}

impl Sense {
    /// Both of them, for a fit that would rather try than be told.
    pub const BOTH: [Self; 2] = [Self::Forward, Self::Reversed];
}

/// How far the beam lands from the marker it was driven onto, under a candidate mount.
pub fn residual_deg(mount: &HeadMount, observation: &Observation) -> f64 {
    mount
        .direction(observation.pose)
        .angle_to(mount.position.toward(observation.at))
}

/// Recovers a head's mount from points it was driven onto by hand.
///
/// `seed` is the mount setup asked for — a nominal height, a base turned the way the plan said
/// to turn it — and starting from it is what makes the answer checkable: a solution far from its seed is
/// reporting that the head is not where it was put, which is a fault to fix on the field
/// rather than a number to accept.
///
/// Both senses of both axes are tried, and the best fit wins. A head whose own menu reverses
/// an axis tracks DMX perfectly and lands nowhere near the geometry, and a fault that reads
/// as a broken solver is worth four times the arithmetic to turn into a line of output.
pub fn solve(seed: HeadMount, observations: &[Observation]) -> Solution {
    Sense::BOTH
        .into_iter()
        .flat_map(|pan| Sense::BOTH.into_iter().map(move |tilt| (pan, tilt)))
        .map(|(pan, tilt)| descend(HeadMount { pan, tilt, ..seed }, observations))
        .min_by(|one, other| one.rms_deg.total_cmp(&other.rms_deg))
        .expect("four sense combinations always yield a solution")
}

/// How far a first probe reaches along each fitted parameter, in the order [`nudged`] takes
/// them: the three of the position in metres, then the two angles of the zero in degrees.
/// Generous enough to cross a badly-placed head or a base turned to the wrong quarter, since
/// the descent halves its reach whenever a pass finds nothing better.
const STRIDE: [f64; 5] = [0.5, 0.5, 0.5, 20.0, 20.0];

/// A backstop on the descent, not a budget. Every pass either improves the fit or halves its
/// reach, so the halving alone ends it — unless improvements arrive forever in ever smaller
/// slivers, which floating point permits and which would hang a tool somebody is waiting on.
const MAX_PASSES: usize = 10_000;

/// Where the halving stops: a thousandth of the first stride, which is under a millimetre and
/// well under a hundredth of a degree.
const SETTLED: f64 = 1e-3;

/// A pattern search: probe each parameter both ways, keep what improves, halve the reach when
/// a whole pass finds nothing.
///
/// No derivatives, because none are needed. This runs once per head on a laptop against a
/// handful of observations, so the budget is enormous and the thing worth buying with it is a
/// method with nothing in it to get wrong — no Jacobian to derive by hand and mistype, no
/// step-size heuristic, and no way to converge confidently on a shape that was mis-copied.
fn descend(seed: HeadMount, observations: &[Observation]) -> Solution {
    let mut mount = seed;
    let mut cost = rms_deg(&mount, observations);
    let mut reach = 1.0;

    for _ in 0..MAX_PASSES {
        if reach <= SETTLED {
            break;
        }
        let mut improved = false;
        for parameter in 0..STRIDE.len() {
            for direction in [1.0, -1.0] {
                let trial = nudged(mount, parameter, direction * reach * STRIDE[parameter]);
                let trial_cost = rms_deg(&trial, observations);
                if trial_cost < cost {
                    (mount, cost) = (trial, trial_cost);
                    improved = true;
                }
            }
        }
        if !improved {
            reach *= 0.5;
        }
    }

    Solution {
        mount,
        rms_deg: cost,
    }
}

fn rms_deg(mount: &HeadMount, observations: &[Observation]) -> f64 {
    if observations.is_empty() {
        return 0.0;
    }
    let squares: f64 = observations
        .iter()
        .map(|observation| residual_deg(mount, observation).powi(2))
        .sum();
    (squares / observations.len() as f64).sqrt()
}

fn nudged(mount: HeadMount, parameter: usize, delta: f64) -> HeadMount {
    let mut nudged = mount;
    match parameter {
        0 => nudged.position.x += delta,
        1 => nudged.position.y += delta,
        2 => nudged.position.z += delta,
        3 => nudged.zero.bearing_deg += delta,
        _ => nudged.zero.elevation_deg += delta,
    }
    nudged
}

/// Every value in `0..=range_deg` that is `base` plus some whole number of turns.
///
/// A head with more than 360° of travel can reach the same place at more than one number,
/// and which of those numbers it is standing on decides how far it has to move to get
/// somewhere else — so they are all offered rather than reduced to a canonical one.
fn revolutions(base: f64, range_deg: f64) -> impl Iterator<Item = f64> {
    let first = base.rem_euclid(360.0);
    (0..).map_while(move |turn| {
        let deg = first + 360.0 * f64::from(turn);
        (deg <= range_deg).then_some(deg)
    })
}
