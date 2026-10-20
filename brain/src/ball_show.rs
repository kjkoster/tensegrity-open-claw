//! The ball's channels for a cue: its motor, its power LEDs' pattern, its status LED, and
//! whether it may sleep.
//!
//! The ball renders the pulse and the strobe itself. A frame here is 25 ms, longer than the
//! pulse holds at its peak, and multicast over WiFi arrives bunched, so drawing either from
//! the frame loop would come out as a staircase with a stutter. What the brain sends is which
//! pattern, how bright, and — for the strobe — how long its flashes and gaps are, which is how
//! the music still reaches it.

use crate::patch::{BALL, Ball};
use crate::show_state::{Cue, ShowState};
use ball_dmx::STROBE_STEP_MS;
use cortex::ear::AudioFeatures;
use std::f64::consts::TAU;

/// How fast the status LED fades up and down.
const STATUS_LED_HZ: f64 = 1.0;

/// The power LEDs' peak in each pattern, as a fraction of full.
const PULSE_LEVEL: f64 = 1.0;
const STROBE_LEVEL: f64 = 0.8;

// TODO: settle a cap on the flash rate. At their extremes these give 80 ms cycles, 12.5 Hz,
// inside the 3–30 Hz band that can set off photosensitive seizures; a floor under the gap
// would keep the strobe below 3 Hz.
/// The strobe's flash, from a quiet high band to a loud one.
const STROBE_FLASH_MS: (f64, f64) = (20.0, 80.0);
/// The strobe's gap, from a quiet room to a loud one: louder is faster.
const STROBE_GAP_MS: (f64, f64) = (600.0, 60.0);

pub struct BallShow {
    /// The status LED's sine, kept within one turn so it never grows past the precision of
    /// an f64.
    status_led_phase: f64,
}

impl BallShow {
    pub fn new() -> Self {
        Self { status_led_phase: 0.0 }
    }

    /// Fills the ball's channels for this frame, `dt` seconds after the last.
    pub fn fill(&mut self, slots: &mut [u8], cue: Cue, features: &AudioFeatures, dt: f64) {
        fill_look(slots, cue, features);

        let sleep = if cue.sleep { Ball::SLEEP_SLEEP } else { Ball::SLEEP_AWAKE };
        BALL.sleep.set(slots, sleep.center());

        // A sine on the status LED, so that a ball following the rig is told apart at a
        // glance from one that is not — which is why it breathes in blackout as well.
        self.status_led_phase = (self.status_led_phase + TAU * STATUS_LED_HZ * dt) % TAU;
        BALL.status_led.set_unit(slots, (1.0 + self.status_led_phase.sin()) / 2.0);
    }
}

/// The motor and the power LEDs, as the show state asks for them.
fn fill_look(slots: &mut [u8], cue: Cue, features: &AudioFeatures) {
    let motor = if cue.motor { Ball::MOTOR_ON } else { Ball::MOTOR_OFF };
    BALL.motor.set(slots, motor.center());

    let (pattern, level, flash_ms, gap_ms) = match cue.state {
        ShowState::Blackout | ShowState::Daylight => (Ball::PATTERN_DIRECT, 0.0, 0.0, 0.0),
        ShowState::Pulse => (Ball::PATTERN_PULSE, PULSE_LEVEL, 0.0, 0.0),
        ShowState::Strobe => (
            Ball::PATTERN_STROBE,
            STROBE_LEVEL,
            mix(STROBE_FLASH_MS, features.energy_high),
            mix(STROBE_GAP_MS, features.energy),
        ),
    };
    BALL.pattern.set(slots, pattern.center());
    BALL.white.set_unit(slots, level);
    BALL.blue.set_unit(slots, level);
    BALL.strobe_flash.set(slots, strobe_steps(flash_ms));
    BALL.strobe_gap.set(slots, strobe_steps(gap_ms));
}

/// `range.0` at `amount` 0, `range.1` at 1.
fn mix(range: (f64, f64), amount: f32) -> f64 {
    let amount = f64::from(amount).clamp(0.0, 1.0);
    range.0 + (range.1 - range.0) * amount
}

fn strobe_steps(ms: f64) -> u8 {
    (ms / f64::from(STROBE_STEP_MS)).round().clamp(0.0, 255.0) as u8
}
