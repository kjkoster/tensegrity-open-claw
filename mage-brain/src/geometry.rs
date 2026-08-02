//! Where this rig's pieces stand on the field: the stool at the origin, four heads around it,
//! and the markers the calibration is recorded against.
//!
//! **The frame is anchored here.** `cortex::geometry` fixes the conventions every rig shares —
//! right-handed, metres, degrees, +Z up, bearings counter-clockwise from +X — and leaves the
//! two choices that are a rig's own. This rig puts the origin at the **stool base on the
//! ground** and points **+X at the centre of the audience**, which makes +Y the mage's left as
//! they stand facing out.
//!
//! Audience-relative rather than compass-relative, because every rule that consumes it is
//! already stated that way: beams never sweep eye level over the audience, the target surface
//! hangs above them, a throw travels outward. A frame those have to be re-derived into before
//! they can be checked is a frame that gets got wrong on a field at four in the afternoon. The
//! stool base is the origin because it is the one point the show is about, it is physically
//! markable, and it is what a tape measure is held against while the markers are recorded.
//! Turning the rig around on the field re-measures markers; it never re-derives the frame.
//!
//! Numbers rather than machinery. The arithmetic that turns any of this into a pose is shared,
//! because the other rig gets the same workflow the moment it wants aimed heads; what is here
//! is only true of a stool on a field, and would be dead weight in the shared half.
//!
//! **Everything below is the lighting plan's, not a solve's.** Setup imposes most of this
//! geometry rather than discovering it — a base flat at a known height with its connectors
//! turned toward the stool — and these constants are that intent written down. They are good
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

/// A kid stands on this, and the show is about what happens on top of it.
///
/// Chair height rather than the knee height the plan first asked for, and the reason is
/// [`HEAD_Z`]: the heads cannot aim below their own lens, so a stool shorter than that is a
/// stool whose occupant stands in a place no beam can be pointed at. The assertion below is
/// what keeps that from being rediscovered on a field.
pub const STOOL_HEIGHT_M: f64 = 0.55;

/// The mage's feet, which is where the attentive pool lands.
pub const STOOL_TOP: Point = Point::new(0.0, 0.0, STOOL_HEIGHT_M);

/// A child's face above the surface they are standing on. Only used to place a marker, and
/// only ever approximately: the fit wants a point off the ground far more than it wants that
/// point to be any particular height.
const FACE_ABOVE_STOOL_M: f64 = 1.20;

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
/// of the world that stops being lightable — including the stool the whole show is about.
const HEAD_Z: f64 = 0.45;

/// The show needs a beam it can point at the mage's feet, and a head cannot point below its
/// own lens. Everything about the rig can be retuned on a field except how tall the stool is,
/// which is a thing somebody has to have brought, so the conflict is worth catching at the
/// build rather than at the first show.
const _: () = assert!(
    STOOL_HEIGHT_M > HEAD_Z,
    "the stool must stand taller than the heads' lenses, or nothing can light the mage's feet"
);

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

/// A square around the stool, every base turned inward so its connectors face the mage.
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
/// exactly the direction a head's own height lives in. The two over the stool carry the
/// height spread, the three on poles carry the bearing and distance spread, and no three of
/// them lie on one line.
pub static MARKERS: [Marker; 5] = [
    Marker {
        scene: "cal_attention_spot",
        at: STOOL_TOP,
    },
    Marker {
        scene: "cal_mage_face",
        at: Point::new(0.0, 0.0, STOOL_HEIGHT_M + FACE_ABOVE_STOOL_M),
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
