//! The four heads, and the channel pair that says where one of them is pointing.
//!
//! There is no world frame here, no solve, and nothing measured. The arms drive pan and tilt
//! directly, so nothing ever has to know where a head stands, which way the field runs, or how
//! far away the mage is — a rig that moves gets carried out, set down and switched on. What is
//! left below is the roster: four names bound to four patched fixtures.
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

/// One head: a name and the fixture it is patched to.
///
/// The name is the workspace's, so a line of log and a line of patch name the same head.
pub struct Head {
    pub name: &'static str,
    pub fixture: &'static patch::Zq02015,
}

/// The arc as it stands, from the mage's left round to their right.
///
/// Snakes 1 and 2 are the mage's left and 3 and 4 their right, which is the pairing an arm
/// drives: each arm flies the pair on its own side of the body.
pub static SNAKE_1: Head = Head {
    name: "snake_1",
    fixture: &patch::SNAKE_1,
};

pub static SNAKE_2: Head = Head {
    name: "snake_2",
    fixture: &patch::SNAKE_2,
};

pub static SNAKE_3: Head = Head {
    name: "snake_3",
    fixture: &patch::SNAKE_3,
};

pub static SNAKE_4: Head = Head {
    name: "snake_4",
    fixture: &patch::SNAKE_4,
};
