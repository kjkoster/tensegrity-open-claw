//! Where this rig's pieces stand on the field: the mage at the origin, four heads around them,
//! and the markers the calibration is recorded against.
//!
//! **The frame is anchored here.** `cortex::geometry` fixes the conventions every rig shares —
//! right-handed, metres, degrees, +Z up, bearings counter-clockwise from +X — and leaves the
//! two choices that are a rig's own. This rig puts the origin **where the mage stands, on the
//! ground** and points **+X at the centre of the audience**, which makes +Y the mage's left as
//! they stand facing out.
//!
//! Audience-relative rather than compass-relative, because every rule that consumes it is
//! already stated that way: beams never sweep eye level over the audience, the target surface
//! hangs above them, a throw travels outward. A frame those have to be re-derived into before
//! they can be checked is a frame that gets got wrong on a field at four in the afternoon. The
//! mage's own footprint is the origin because it is the one point the show is about, it is
//! physically markable, and it is what a tape measure is held against while the markers are
//! recorded.
//! Turning the rig around on the field re-measures markers; it never re-derives the frame.
//!
//! Numbers rather than machinery. The arithmetic that turns any of this into a pose is shared,
//! because the other rig gets the same workflow the moment it wants aimed heads; what is here
//! is only true of this rig on a field, and would be dead weight in the shared half.
//!
//! **Everything below is the lighting plan's, not a solve's.** Setup imposes most of this
//! geometry rather than discovering it — a base flat at a known height with its connectors
//! turned toward the mage — and these constants are that intent written down. They are good
//! enough to aim with, badly, before a marker has been recorded, and that is their job: they
//! seed the fit and they are what the fit is checked against. The solver prints their
//! replacements, which are pasted over them.
//!
//! Both files that need them read them from here. A head paired with another head's mount
//! aims into the crowd while every number involved looks reasonable, so there is one table and
//! no second place to keep it in step.

// Two binaries read this table and neither reads all of it: the show never opens the marker
// list, the solver never asks where the beams converge. Unused here means "used by the other
// one", which is the point of there being a single table.
#![allow(dead_code)]

use crate::patch;
use cortex::geometry::{Direction, HeadMount, Point, Sense, Travel};

/// Chest height on a standing child, and where the attentive beams land.
///
/// The show wanted the mage's feet and cannot have them: the ground at the origin is below
/// [`HEAD_Z`] and no beam reaches it. This is the better target anyway — a beam on a torso is a
/// beam the audience sees land on a person rather than on grass.
pub const MAGE_CHEST_M: f64 = 1.10;

/// A standing child's face. Only used to place a marker, and only ever approximately: the fit
/// wants a second point well above the first far more than it wants either at a particular
/// height, and the gap between this and [`MAGE_CHEST_M`] is the whole of what it wants.
const MAGE_FACE_M: f64 = 1.45;

/// Where the beams meet in magic mode: a plane at this height over the audience side.
///
/// Recorded here because it is rig geometry and gets measured with the same tape as the rest.
/// Nothing consumes it until the show can point at things.
pub const TARGET_SURFACE_HEIGHT_M: f64 = 3.50;

/// How high the beam leaves a head: floor to the centre of the lens, with the base standing on
/// the field itself.
///
/// Measured with a tape against the head rather than taken from the declared dimensions, which
/// describe a shipping box. That the base stands on nothing is load-bearing rather than lazy:
/// these heads cannot aim below their own lens, so every centimetre of plinth is a centimetre
/// of the world that stops being lightable — starting with the ground the mage stands on.
const HEAD_Z: f64 = 0.45;

/// How high a calibration target rides on its pole.
///
/// Markers are not painted on the ground, because a head that cannot look below its own
/// optical centre cannot be driven onto one. A metre clears the heads with room to spare and
/// is a height a person can stand a cone or a stick at without a spirit level.
const MARKER_POLE_HEIGHT_M: f64 = 1.00;

/// Elevation of the beam at tilt zero, with the head level.
///
/// Level, measured on the bench: tilt zero folds the head back over its own base and the beam
/// leaves horizontally, pointing the way the connectors face. Tilt then rotates it up, through
/// vertical at the middle of the channel, and back down to horizontal at the far end — which
/// is 180° of beam, not the 270° the definition used to claim.
///
/// The consequence is a hard one and belongs next to the number: **these heads cannot aim
/// below their own optical centre.** Anything the show wants lit has to sit at or above that
/// height, and anything below it is not a tuning problem but a mounting one.
const TILT_ZERO_ELEVATION_DEG: f64 = 0.0;

/// One head as the plan places it, and the fixture it drives.
///
/// The name is the workspace's, so a line of solver output and a line of startup log name the
/// same head.
pub struct Head {
    pub name: &'static str,
    pub fixture: &'static patch::Zq02015,
    pub mount: HeadMount,
}

impl Head {
    pub fn travel(&self) -> Travel {
        Travel::of::<patch::Zq02015>()
    }
}

/// A square around the mage, every base turned inward so its connectors face them.
///
/// The bearings below are that rule already worked out: a head on a corner looks back at the
/// origin. They are written as numbers rather than computed from the positions because this
/// file is what the solver overwrites, and a solved mount has no rule left in it to compute
/// from — the head is wherever the field let it stand.
pub static SNAKE_1: Head = Head {
    name: "snake_1",
    fixture: &patch::SNAKE_1,
    mount: HeadMount {
        position: Point::new(1.5, 1.5, HEAD_Z),
        zero: Direction::new(-135.0, TILT_ZERO_ELEVATION_DEG),
        pan: Sense::Forward,
        tilt: Sense::Forward,
    },
};

pub static SNAKE_2: Head = Head {
    name: "snake_2",
    fixture: &patch::SNAKE_2,
    mount: HeadMount {
        position: Point::new(1.5, -1.5, HEAD_Z),
        zero: Direction::new(135.0, TILT_ZERO_ELEVATION_DEG),
        pan: Sense::Forward,
        tilt: Sense::Forward,
    },
};

pub static SNAKE_3: Head = Head {
    name: "snake_3",
    fixture: &patch::SNAKE_3,
    mount: HeadMount {
        position: Point::new(-1.5, -1.5, HEAD_Z),
        zero: Direction::new(45.0, TILT_ZERO_ELEVATION_DEG),
        pan: Sense::Forward,
        tilt: Sense::Forward,
    },
};

pub static SNAKE_4: Head = Head {
    name: "snake_4",
    fixture: &patch::SNAKE_4,
    mount: HeadMount {
        position: Point::new(-1.5, 1.5, HEAD_Z),
        zero: Direction::new(-45.0, TILT_ZERO_ELEVATION_DEG),
        pan: Sense::Forward,
        tilt: Sense::Forward,
    },
};

pub static HEADS: [&Head; 4] = [&SNAKE_1, &SNAKE_2, &SNAKE_3, &SNAKE_4];

/// A place on the field the heads are driven onto by hand, and the QLC+ scene holding where
/// they were pointing when they got there.
pub struct Marker {
    /// The scene's name in the workspace. One scene per marker, carrying all four heads.
    pub scene: &'static str,
    pub at: Point,
}

/// The calibration set.
///
/// Named for what they are rather than lettered, which buys two things. A marker named for a
/// show point is one the show asks for again — the attention spot is the attentive home, and
/// one save serves both — and "the mage's face" is a place the next deployment can put back,
/// where "marker C" is not.
///
/// Not one of them is on the ground, because not one of them could be driven onto there. What
/// the heads cannot reach cost the set nothing, as it turns out: the fit wants points spread
/// in height as much as in bearing, and a plane of ground markers would have held it weakly in
/// exactly the direction a head's own height lives in. The two over the mage carry the
/// height spread, the three on poles carry the bearing and distance spread, and no three of
/// them lie on one line.
pub static MARKERS: [Marker; 5] = [
    Marker {
        scene: "cal_attention_spot",
        at: Point::new(0.0, 0.0, MAGE_CHEST_M),
    },
    Marker {
        scene: "cal_mage_face",
        at: Point::new(0.0, 0.0, MAGE_FACE_M),
    },
    Marker {
        scene: "cal_field_left",
        at: Point::new(1.0, 3.0, MARKER_POLE_HEIGHT_M),
    },
    Marker {
        scene: "cal_field_right",
        at: Point::new(1.0, -3.0, MARKER_POLE_HEIGHT_M),
    },
    Marker {
        scene: "cal_audience_near",
        at: Point::new(4.0, 0.0, MARKER_POLE_HEIGHT_M),
    },
];
