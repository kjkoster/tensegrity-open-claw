//! Where a head is pointing, and how fast it is allowed to get somewhere else.
//!
//! Two clocks meet here. Whatever decides where a head should point updates slowly — a pose
//! estimate at a handful of hertz, a metronome on a timer — while the wire runs at frame
//! rate, and a head handed the slow signal directly steps between its values. So the target
//! is held and walked toward every frame, and what reaches the fixture is a smooth pursuit
//! of a coarsely-updating goal rather than the goal itself.

use crate::qlc_plus::Position;

/// Where a head points, in the unreferenced degrees [`Position`] describes.
#[derive(Clone, Copy, PartialEq)]
pub struct Pose {
    pub pan_deg: f64,
    pub tilt_deg: f64,
}

impl Pose {
    pub const fn new(pan_deg: f64, tilt_deg: f64) -> Self {
        Self { pan_deg, tilt_deg }
    }
}

/// The slowest a head can be driven and still read as moving rather than stepping.
///
/// Below this the fixture renders a smooth stream of positions as a series of visible jumps.
/// It is a property of the mechanism: measured with the commanded ramp verified exact to four
/// significant figures, with the head's own motor ramp making no difference at either end of
/// its channel, and with the stutter following the commanded rate rather than the unit or the
/// stretch of travel it crossed. Nothing above the fixture can dither its way past a
/// mechanism that will not place itself any finer.
///
/// This is a fact about a fixture, which normally means it belongs in that fixture's
/// definition and not in Rust. The definition format has no field for it — it describes
/// channels and travel, not how the travel behaves — so it lives here, as the one class of
/// fixture fact the source of truth cannot hold. It was measured on the moving heads both
/// rigs own; a rental is a different mechanism and wants measuring again.
pub const MIN_SLEW_DEG_S: f64 = 7.0;

/// An angular speed that a head can actually render.
///
/// The floor is enforced by the type rather than checked at the point of use, because the
/// numbers that reach it will not always be written by hand: a personality's slew constant, a
/// noise field's rate, an eased approach to a target all end up here, and any of them can
/// compute their way below the floor without anyone choosing to. Making the value
/// unrepresentable is what keeps that from reaching the wire.
///
/// Too-slow rates are raised rather than refused. A show that cannot render the motion it
/// asked for should give the slowest honest version of it, not stop.
#[derive(Clone, Copy)]
pub struct SlewRate(f64);

impl SlewRate {
    /// The slowest rate there is: anything below the floor becomes this.
    pub const SLOWEST: Self = Self(MIN_SLEW_DEG_S);

    pub const fn new(deg_s: f64) -> Self {
        if deg_s < MIN_SLEW_DEG_S {
            Self::SLOWEST
        } else {
            Self(deg_s)
        }
    }

    pub const fn deg_s(self) -> f64 {
        self.0
    }
}

/// A per-head ceiling on angular speed, and the position that respects it.
///
/// A rate limit rather than an exponential ease toward the target: the quantity worth
/// bounding is what the motor actually does, and an ease makes that a function of how far
/// away the target happens to be — the same head crawls through small moves and slams
/// through large ones. The limit is per head because heads differ in what they can do
/// gracefully, and because a personality is exactly this number with a name on it.
///
/// Pan and tilt are limited independently, since the motors are independent. A diagonal move
/// therefore finishes its short axis first and straightens out, which is what the mechanism
/// does anyway.
pub struct Slew {
    pose: Pose,
    rate: SlewRate,
}

impl Slew {
    pub fn new(start: Pose, rate: SlewRate) -> Self {
        Self { pose: start, rate }
    }

    /// Where the head points now.
    ///
    /// The one copy of it. Anything choosing where to send the head next has to judge from
    /// where it actually is, and a second copy kept by the chooser would go stale the moment
    /// something else moved the head.
    pub fn pose(&self) -> Pose {
        self.pose
    }

    /// Advances one frame toward `target` and returns where the head now points.
    pub fn step(&mut self, target: Pose, dt: f64) -> Pose {
        let step = self.rate.deg_s() * dt;
        self.pose.pan_deg = approach(self.pose.pan_deg, target.pan_deg, step);
        self.pose.tilt_deg = approach(self.pose.tilt_deg, target.tilt_deg, step);
        self.pose
    }
}

/// Moves `from` toward `to` by at most `step`, landing exactly on the target rather than
/// oscillating around it once the remaining distance is smaller than one frame's travel.
fn approach(from: f64, to: f64, step: f64) -> f64 {
    let delta = to - from;
    if delta.abs() <= step {
        to
    } else {
        from + delta.signum() * step
    }
}

/// Points a fixture at a pose, converting degrees to the pair's 16-bit range.
///
/// The ranges come from the fixture's own definition through [`Position`], so this is the
/// one place degrees become DMX and no show above it holds a number that belongs to a model.
pub fn aim<F: Position>(fixture: &F, slots: &mut [u8], pose: Pose) {
    fixture
        .pan()
        .set_unit(slots, pose.pan_deg / F::PAN_RANGE_DEG);
    fixture
        .tilt()
        .set_unit(slots, pose.tilt_deg / F::TILT_RANGE_DEG);
}

/// Where a fixture's slots say it is pointing — the exact inverse of [`aim`].
///
/// Calibration is a look driven by hand from a console and saved, so the recorded pose has to
/// come back out through the same travel constants it would have gone in through. Written as
/// the inverse of `aim` rather than as its own conversion for that reason: a wrong range is
/// then wrong in both directions and shows up as a fit that will not close, instead of
/// cancelling itself out on the way round and leaving the error to appear on the field.
pub fn pose_of<F: Position>(fixture: &F, slots: &[u8]) -> Pose {
    Pose::new(
        fixture.pan().get_unit(slots) * F::PAN_RANGE_DEG,
        fixture.tilt().get_unit(slots) * F::TILT_RANGE_DEG,
    )
}
