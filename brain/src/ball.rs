//! The ball's motor, and what an e-stop release may not do to it.
//!
//! A blackout sends the motor channel to off like every other channel, and to the ball an
//! off is the rig saying the run is over: it clears the ball's own trip. So when the e-stop
//! is released, whatever is driving the rig would get the motor back at once if it was
//! still holding the channel on. Releasing an e-stop must never restart machinery by
//! itself, so the motor stays held off after a release until the source driving the rig
//! asks for a stop of its own — the same rule the ball applies after it loses the universe.

use cortex::qlc_plus::Channel;

/// What the motor channel is held at while latched: the wire's own default, and inside
/// the definition's off band.
const HOLD: u8 = 0;

pub struct MotorInterlock {
    motor: Channel,
    latched: bool,
}

impl MotorInterlock {
    pub fn new(motor: Channel) -> Self {
        Self {
            motor,
            latched: false,
        }
    }

    /// Runs on the final frame, after everything else has decided what it holds. `blackout`
    /// says the frame has already been sent black for the e-stop.
    pub fn apply(&mut self, slots: &mut [u8], blackout: bool) {
        if blackout {
            if !self.latched {
                eprintln!("ball: motor latched off by the e-stop");
                self.latched = true;
            }
            return;
        }
        if !self.latched {
            return;
        }
        if self.is_stop(self.motor.get(slots)) {
            eprintln!("ball: motor latch cleared — the source asked for a stop");
            self.latched = false;
        } else {
            self.motor.set(slots, HOLD);
        }
    }

    /// Whether `value` asks for the motor to be off.
    ///
    /// Read from the definition's bands rather than compared with a number, so a stop means
    /// what the fixture means by one. A definition without bands has nothing to read, and
    /// then only the hold value itself counts: the latch may stay on too long, never clear
    /// too early.
    fn is_stop(&self, value: u8) -> bool {
        match self.motor.band(HOLD) {
            Some(off) => self.motor.band(value) == Some(off),
            None => value == HOLD,
        }
    }
}
