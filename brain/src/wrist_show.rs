//! Wrist's channels: Hold or Pluck, and the tug a Pluck makes.
//!
//! Only the show's side of Wrist. What it hears from Wrist is in `wrist`; what Wrist does with
//! its channels is its firmware's, in `CONTROL_LOOP.md`.

use crate::patch::{WRIST, Wrist};
use cortex::qlc_plus::Capability;

/// The tug a Pluck makes: `size` as a fraction of Wrist's largest, `A_MAX` in its firmware;
/// `speed` as a fraction of its fastest; which way, facing the shaft.
pub struct Tug {
    pub size: f64,
    pub speed: f64,
    pub clockwise: bool,
}

/// Sends Pluck, or Hold, with the tug Pluck reads as it starts.
pub fn fill(slots: &mut [u8], pluck: bool, tug: &Tug) {
    let mode = if pluck {
        Wrist::MODE_PLUCK
    } else {
        Wrist::MODE_HOLD
    };
    WRIST.mode.set(slots, middle(mode));
    WRIST.amplitude.set_unit(slots, tug.size);
    let direction = if tug.clockwise {
        Wrist::DIRECTION_CLOCKWISE
    } else {
        Wrist::DIRECTION_COUNTER_CLOCKWISE
    };
    WRIST.direction.set(slots, middle(direction));
    let speed = Wrist::SPEED_SLOW_TO_THE_MAXIMUM;
    let span = f64::from(speed.max - speed.min);
    WRIST.speed.set(slots, speed.min + (span * tug.speed.clamp(0.0, 1.0)).round() as u8);
}

/// The level in the middle of a band, as far from its neighbours as it can be.
fn middle(band: Capability) -> u8 {
    band.min + (band.max - band.min) / 2
}
