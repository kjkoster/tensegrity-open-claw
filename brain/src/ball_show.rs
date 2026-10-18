//! The ball's channels for a cue: its motor, and its power LEDs' pattern.
//!
//! The ball renders the pulse and the strobe itself. A frame here is 25 ms, longer than the
//! pulse holds at its peak, and multicast over WiFi arrives bunched, so drawing either from
//! the frame loop would come out as a staircase with a stutter. What the brain sends is which
//! pattern, how bright, and — for the strobe — how long its flashes and gaps are, which is how
//! the music still reaches it.

use crate::patch::{BALL_5, Ball};
use crate::scene::{Cue, Scene};
use ball_dmx::STROBE_STEP_MS;
use cortex::audio_features::AudioFeatures;

/// The power LEDs' peak in each pattern, as a fraction of full.
const PULSE_LEVEL: f64 = 1.0;
const STROBE_LEVEL: f64 = 0.8;

// TODO: settle a cap on the flash rate. At their extremes these give 80 ms cycles, 12.5 Hz,
// inside the 3–30 Hz band that can set off photosensitive seizures; a floor under the gap
// would keep the strobe below 3 Hz.
/// The strobe's flash, from a quiet high band to a loud one.
const STROBE_ON_MS: (f64, f64) = (20.0, 80.0);
/// The strobe's gap, from a quiet room to a loud one: louder is faster.
const STROBE_OFF_MS: (f64, f64) = (600.0, 60.0);

pub fn fill(slots: &mut [u8], cue: Cue, features: &AudioFeatures) {
    let motor = if cue.motor { Ball::MOTOR_ON } else { Ball::MOTOR_OFF };
    BALL_5.motor.set(slots, motor.center());

    let (pattern, level, on_ms, off_ms) = match cue.scene {
        Scene::Estop | Scene::Blackout | Scene::Daylight => (Ball::PATTERN_DIRECT, 0.0, 0.0, 0.0),
        Scene::Pulse => (Ball::PATTERN_PULSE, PULSE_LEVEL, 0.0, 0.0),
        Scene::Strobe => (
            Ball::PATTERN_STROBE,
            STROBE_LEVEL,
            mix(STROBE_ON_MS, features.energy_high),
            mix(STROBE_OFF_MS, features.energy),
        ),
    };
    BALL_5.pattern.set(slots, pattern.center());
    BALL_5.white.set_unit(slots, level);
    BALL_5.blue.set_unit(slots, level);
    BALL_5.strobe_on.set(slots, strobe_steps(on_ms));
    BALL_5.strobe_off.set(slots, strobe_steps(off_ms));
}

/// `range.0` at `amount` 0, `range.1` at 1.
fn mix(range: (f64, f64), amount: f32) -> f64 {
    let amount = f64::from(amount).clamp(0.0, 1.0);
    range.0 + (range.1 - range.0) * amount
}

fn strobe_steps(ms: f64) -> u8 {
    (ms / f64::from(STROBE_STEP_MS)).round().clamp(0.0, 255.0) as u8
}
