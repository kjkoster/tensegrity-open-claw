//! The ball's channels for a cue: its motor, its power LEDs' pattern, its status LED, and
//! whether it may sleep.
//!
//! The ball renders the pulse itself. A frame here is 25 ms, longer than the pulse holds at its
//! peak, and multicast over WiFi arrives bunched, so drawing it from the frame loop would come
//! out as a staircase with a stutter. What the brain sends is which pattern, and how bright.
//!
//! Ball Lighting is the one exception: its fade is drawn here, frame by frame, in Direct. At
//! five seconds a crossfade the bunching steps it about a percent at a time, which reads as
//! smooth.

use crate::ball_lighting;
use crate::patch::{BALL, Ball};
use crate::show_state::{Base, Cue, Look};
use std::f64::consts::TAU;

/// How fast the status LED fades up and down.
const STATUS_LED_HZ: f64 = 1.0;

/// The power LEDs' peak in the pulse, as a fraction of full.
const PULSE_LEVEL: f64 = 1.0;

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
    pub fn fill(&mut self, slots: &mut [u8], cue: Cue, dt: f64) {
        fill_look(slots, cue);

        let sleep = if cue.sleep { Ball::SLEEP_SLEEP } else { Ball::SLEEP_AWAKE };
        BALL.sleep.set(slots, sleep.center());

        // A sine on the status LED, so that a ball following the rig is told apart at a
        // glance from one that is not — which is why it breathes in blackout as well.
        self.status_led_phase = (self.status_led_phase + TAU * STATUS_LED_HZ * dt) % TAU;
        BALL.status_led.set_unit(slots, (1.0 + self.status_led_phase.sin()) / 2.0);
    }
}

/// The motor and the power LEDs, as the show state asks for them. The motor keeps its beat
/// whatever the power LEDs do.
fn fill_look(slots: &mut [u8], cue: Cue) {
    let motor = if cue.motor { Ball::MOTOR_ON } else { Ball::MOTOR_OFF };
    BALL.motor.set(slots, motor.center());

    let (pattern, white, blue) = match cue.look() {
        Look::Base(Base::Blackout | Base::Daylight) => (Ball::PATTERN_DIRECT, 0.0, 0.0),
        Look::Base(Base::Dusk) => (Ball::PATTERN_PULSE, PULSE_LEVEL, PULSE_LEVEL),
        // White at the fade's white end, blue at its colour end: the ball has no other colour.
        Look::BallLighting => {
            let colour = ball_lighting::phase(&cue);
            (Ball::PATTERN_DIRECT, 1.0 - colour, colour)
        }
    };
    BALL.pattern.set(slots, pattern.center());
    BALL.white.set_unit(slots, white);
    BALL.blue.set_unit(slots, blue);
    // The ball's strobe pattern is never asked for, so its timing channels stay at zero.
    BALL.strobe_flash.set(slots, 0);
    BALL.strobe_gap.set(slots, 0);
}
