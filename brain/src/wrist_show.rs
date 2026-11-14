//! Wrist's channels: its mode, the tug a Pluck makes, and Run's sine.
//!
//! Only the show's side of Wrist. What it hears from Wrist is in `wrist`; what Wrist does with
//! its channels is its firmware's, in `wrist/`.

use crate::patch::{WRIST, Wrist};
use cortex::qlc_plus::Capability;

/// The tug a Pluck makes: `size` as a fraction of Wrist's largest, `A_MAX` in its firmware;
/// `speed` as a fraction of its fastest; which way, facing the shaft.
pub struct Tug {
    pub size: f64,
    pub speed: f64,
    pub clockwise: bool,
}

/// Run's sine: `amplitude` and `frequency` as fractions of Wrist's largest; which way the first
/// swing goes, facing the shaft.
pub struct Sine {
    pub amplitude: f64,
    pub frequency: f64,
    pub clockwise: bool,
}

/// The mode Wrist is sent.
#[derive(Clone, Copy)]
pub enum Ask {
    Blackout,
    Hold,
    Pluck,
    Run,
}

/// Sends `ask`, with the tug Pluck reads as it starts, or, in Run, the sine.
pub fn fill(slots: &mut [u8], ask: Ask, tug: &Tug, sine: &Sine) {
    let mode = match ask {
        Ask::Blackout => Wrist::MODE_BLACKOUT,
        Ask::Hold => Wrist::MODE_HOLD,
        Ask::Pluck => Wrist::MODE_PLUCK,
        Ask::Run => Wrist::MODE_RUN,
    };
    WRIST.mode.set(slots, middle(mode));
    let (amplitude, clockwise) = match ask {
        Ask::Run => (sine.amplitude, sine.clockwise),
        _ => (tug.size, tug.clockwise),
    };
    WRIST.amplitude.set_unit(slots, amplitude);
    let direction = if clockwise {
        Wrist::DIRECTION_CLOCKWISE
    } else {
        Wrist::DIRECTION_COUNTER_CLOCKWISE
    };
    WRIST.direction.set(slots, middle(direction));
    let speed = Wrist::SPEED_SLOW_TO_THE_MAXIMUM;
    let span = f64::from(speed.max - speed.min);
    WRIST.speed.set(slots, speed.min + (span * tug.speed.clamp(0.0, 1.0)).round() as u8);
    // Frequency is 16 bits over two channels, coarse then fine.
    let frequency = (f64::from(u16::MAX) * sine.frequency.clamp(0.0, 1.0)).round() as u16;
    let [coarse, fine] = frequency.to_be_bytes();
    WRIST.frequency.set(slots, coarse);
    WRIST.frequency_fine.set(slots, fine);
}

/// The level in the middle of a band, as far from its neighbours as it can be.
fn middle(band: Capability) -> u8 {
    band.min + (band.max - band.min) / 2
}
