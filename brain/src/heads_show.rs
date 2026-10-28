//! The moving heads' channels: their wheels and their dimmers.
//!
//! Pan and tilt are not written until the presets are aimed, so every head stays where it
//! is in every state.

use crate::patch::{LEFT_STROBE_HEAD, MIDDLE_WASH_HEAD, RIGHT_STROBE_HEAD, ROSE_HEAD, Zq02015};
use crate::show::ROSE_COLOUR;
use cortex::qlc_plus::Capability;

/// What the heads' wheels show.
#[derive(Clone, Copy)]
pub enum Wheels {
    /// The rose head on the rose gobo in `ROSE_COLOUR`; the others white and open.
    Rose,
    /// Every head white and open.
    White,
}

pub struct Heads {
    pub wheels: Wheels,
    /// The rose head's dimmer, 0..1.
    pub rose: f64,
    /// The other three heads' dimmer, 0..1.
    pub others: f64,
}

const OTHERS: [&Zq02015; 3] = [&LEFT_STROBE_HEAD, &MIDDLE_WASH_HEAD, &RIGHT_STROBE_HEAD];

pub fn fill(slots: &mut [u8], heads: &Heads) {
    let (colour, gobo) = match heads.wheels {
        Wheels::Rose => (ROSE_COLOUR, Zq02015::GOBO_ROSE),
        Wheels::White => (Zq02015::COLOR_WHITE, Zq02015::GOBO_OPEN),
    };
    fill_head(slots, &ROSE_HEAD, colour, gobo, heads.rose);
    for head in OTHERS {
        fill_head(
            slots,
            head,
            Zq02015::COLOR_WHITE,
            Zq02015::GOBO_OPEN,
            heads.others,
        );
    }
}

fn fill_head(slots: &mut [u8], head: &Zq02015, colour: Capability, gobo: Capability, level: f64) {
    head.color.set(slots, colour.center());
    head.gobo.set(slots, gobo.center());
    head.strobe.set(slots, Zq02015::STROBE_SHUTTER_OPEN.center());
    head.dimmer.set_unit(slots, level);
    // These make a head run itself, reset, or light its body; none of that is the show.
    head.automatic_mode.park(slots);
    head.reset.park(slots);
    head.light_strips.park(slots);
}
