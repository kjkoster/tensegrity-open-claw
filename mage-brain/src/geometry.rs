//! The four heads, and the channel pair that says where one of them is pointing.
//!
//! There is no world frame here, no solve, and nothing measured. Tilt is driven straight off an
//! arm, and pan off three pan values per head that were *recorded* by driving that head onto
//! somebody standing at each end of the camera's crop. Between them nothing ever has to know
//! where a head stands, which way the field runs, or how far away the mage is — a rig that moves
//! gets carried out, set down and switched on.
//!
//! Sixteen bit, because pan and tilt are sixteen-bit channels and the coarse byte alone is
//! two degrees of pan — which is visible as a step in a move slow enough to watch.

use crate::patch;
use cortex::moving_head::Pose;
use cortex::qlc_plus::Position;

/// A head position as the console shows it: the pan and tilt channels, coarse and fine
/// together.
#[derive(Clone, Copy)]
pub struct Aim {
    pub pan: u16,
    pub tilt: u16,
}

impl Aim {
    pub const fn new(pan: u16, tilt: u16) -> Self {
        Self { pan, tilt }
    }

    /// The same position in the degrees the slew limiter and `aim` speak.
    ///
    /// Through the fixture's own travel, so a channel value becomes an angle in the one place
    /// that already knows how far these heads turn, rather than against a range retyped here
    /// to go stale.
    pub fn pose(self) -> Pose {
        Pose::new(
            degrees(self.pan, <patch::Zq02015 as Position>::PAN_RANGE_DEG),
            degrees(self.tilt, <patch::Zq02015 as Position>::TILT_RANGE_DEG),
        )
    }
}

fn degrees(dmx: u16, range_deg: f64) -> f64 {
    f64::from(dmx) / f64::from(u16::MAX) * range_deg
}

/// Walks from one channel value to another, `at` running 0 to 1 and clamped at both ends.
///
/// In the channel's own units rather than in degrees, because every end of every one of these
/// walks was read off a slider, and the arithmetic between them should not have to make a trip
/// through a travel constant to get back to where it started.
pub fn lerp(from: u16, to: u16, at: f64) -> u16 {
    let at = at.clamp(0.0, 1.0);
    let span = f64::from(to) - f64::from(from);
    (f64::from(from) + span * at)
        .round()
        .clamp(0.0, f64::from(u16::MAX)) as u16
}

/// The three pan values that keep one head on a mage walking across the field: where that head
/// has to point for a mage at the far end of their own right, in the middle, and at the far end
/// of their own left.
///
/// Recorded on the rig rather than computed — drive the head onto somebody standing at each end
/// of the camera's crop and write down the pan — which is what buys the convergence without
/// buying a world frame with it. Per head and never shared, because four heads standing in an
/// arc need four different swings to stay on one child, and a single shared swing is four heads
/// agreeing to be wrong in three places.
#[derive(Clone, Copy)]
pub struct PanTrack {
    pub mage_right: u16,
    pub centre: u16,
    pub mage_left: u16,
}

impl PanTrack {
    /// Where this head points for a mage at `across`, which runs −1 at the far end of the mage's
    /// own right to +1 at the far end of their left.
    ///
    /// The two halves are interpolated separately, because the recorded ends are not the same
    /// distance from the recorded centre and one straight line through all three would put the
    /// middle somewhere nobody wrote down. It is also what keeps the error small between the
    /// samples: pan against a position across a field is a tangent rather than a line, and a
    /// sample in the middle takes most of the bend out of it.
    pub fn at(self, across: f64) -> u16 {
        if across >= 0.0 {
            lerp(self.centre, self.mage_left, across)
        } else {
            lerp(self.centre, self.mage_right, -across)
        }
    }
}

// Unrecorded, and the same for all four heads until they are. The centre is where hanging arms
// used to park every head, and the two ends are a swing either side of it that is known to be
// within travel — so an unrecorded rig sweeps together and is visibly a rig nobody has recorded,
// rather than one that looks calibrated and quietly misses the child by a metre.
const UNRECORDED: PanTrack = PanTrack {
    mage_right: 0x7000,
    centre: 0xaa00,
    mage_left: 0xe000,
};

/// One head: a name, the fixture it is patched to, and the pan it takes to follow a mage.
///
/// The name is the workspace's, so a line of log and a line of patch name the same head.
pub struct Head {
    pub name: &'static str,
    pub fixture: &'static patch::Zq02015,
    pub pan: PanTrack,
}

/// The arc as it stands, from the mage's left round to their right.
///
/// Snakes 1 and 2 are the mage's left and 3 and 4 their right, which is the pairing an arm
/// drives: each arm flies the pair on its own side of the body.
pub static SNAKE_1: Head = Head {
    name: "snake_1",
    fixture: &patch::SNAKE_1,
    pan: UNRECORDED,
};

pub static SNAKE_2: Head = Head {
    name: "snake_2",
    fixture: &patch::SNAKE_2,
    pan: UNRECORDED,
};

pub static SNAKE_3: Head = Head {
    name: "snake_3",
    fixture: &patch::SNAKE_3,
    pan: UNRECORDED,
};

pub static SNAKE_4: Head = Head {
    name: "snake_4",
    fixture: &patch::SNAKE_4,
    pan: UNRECORDED,
};
