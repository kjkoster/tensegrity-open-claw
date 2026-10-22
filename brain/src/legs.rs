//! The three legs, each a wash and a head named for the same colour, and how each kind of
//! fixture is painted a colour.
//!
//! The pairing is a fact about this rig, found only in its fixture names, so it is written
//! here rather than generated. The two paint functions are where "red" becomes a fixture's
//! channels: the wash mixes it, the head turns its wheel to it. Everything above them speaks
//! [`Colour`], so heads that mix rather than step are a change to [`paint_head`] alone.

use crate::patch::{self, Yara, Zq02015};
use crate::presets::Preset;
use cortex::moving_head::Pose;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    White,
    Red,
    Green,
    Blue,
}

pub struct Leg {
    pub wash: Yara,
    pub head: Zq02015,
    /// The leg's own colour, which its wash holds in the base show.
    pub colour: Colour,
    /// This leg's head's pose in a preset.
    pub pose: fn(&Preset) -> Pose,
}

pub static LEGS: [Leg; 3] = [
    Leg {
        wash: patch::WASH_RED_LEG,
        head: patch::HEAD_RED_LEG,
        colour: Colour::Red,
        pose: |preset| preset.head_red_leg,
    },
    Leg {
        wash: patch::WASH_GREEN_LEG,
        head: patch::HEAD_GREEN_LEG,
        colour: Colour::Green,
        pose: |preset| preset.head_green_leg,
    },
    Leg {
        wash: patch::WASH_BLUE_LEG,
        head: patch::HEAD_BLUE_LEG,
        colour: Colour::Blue,
        pose: |preset| preset.head_blue_leg,
    },
];

/// Sets a wash to a mix of its emitters, each 0..1, with every emitter the mix does not name
/// held off rather than left wherever it was.
pub fn paint_wash(wash: &Yara, slots: &mut [u8], mix: &[(Colour, f64)]) {
    for channel in [wash.red, wash.green, wash.blue, wash.white] {
        channel.set(slots, 0);
    }
    for (colour, level) in mix {
        let channel = match colour {
            Colour::White => wash.white,
            Colour::Red => wash.red,
            Colour::Green => wash.green,
            Colour::Blue => wash.blue,
        };
        channel.set_unit(slots, *level);
    }
}

/// Turns a head's colour wheel to `colour`. The wheel steps rather than blends, and on its
/// way between two slots it sweeps through the ones in between, lit.
pub fn paint_head(head: &Zq02015, slots: &mut [u8], colour: Colour) {
    let band = match colour {
        Colour::White => Zq02015::COLOR_WHITE,
        Colour::Red => Zq02015::COLOR_RED,
        Colour::Green => Zq02015::COLOR_GREEN,
        Colour::Blue => Zq02015::COLOR_BLUE,
    };
    head.color.set(slots, band.center());
}
