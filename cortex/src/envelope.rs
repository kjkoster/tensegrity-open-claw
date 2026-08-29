//! A follower that rises fast and falls slowly, stepped once per frame.
//!
//! The shape a show reaches for whenever something in the room is the input and a channel is the
//! output: the input arrives in bursts, at whatever rate the thing measuring it manages, and the
//! channel has to move smoothly at frame rate regardless. A value written straight through jumps
//! at the input's rate and holds flat between, which reads as a fault; this rises to meet it and
//! then lets go over a time somebody chose.
//!
//! Asymmetric on purpose. Attack and release are different numbers because they answer different
//! questions — how quickly the rig notices, and how long it takes to forget — and one time
//! constant for both can only be wrong at one end.
//!
//! Stepped with the frame's own `dt` rather than built against a fixed rate, so a frame that
//! arrives late decays by what it actually cost rather than by what a nominal rate says it
//! should have. Same shape as [`crate::moving_head::Slew`], which limits a head's travel the
//! same way: hold one, step it every frame, read what comes back.

/// A value that chases a target, quickly one way and slowly the other.
pub struct Envelope {
    attack_s: f64,
    release_s: f64,
    value: f64,
}

impl Envelope {
    /// Times in seconds: how long to rise toward a target, and how long to fall away from one.
    ///
    /// Both are the one-pole time constant rather than a time-to-arrive, so the value covers
    /// about two thirds of the remaining distance in one of them and never quite lands. That is
    /// the point of it — an envelope that arrives exactly has a corner in it, and a corner on a
    /// dimmer is visible.
    pub fn new(attack_s: f64, release_s: f64) -> Self {
        Self {
            attack_s,
            release_s,
            value: 0.0,
        }
    }

    /// Where the envelope is now, without moving it.
    pub fn value(&self) -> f64 {
        self.value
    }

    /// Puts the envelope back where it started, for a state that is beginning rather than
    /// continuing.
    pub fn reset(&mut self) {
        self.value = 0.0;
    }

    /// Advances one frame toward `target` and returns the new value.
    ///
    /// Which time constant applies is decided per frame by which way the value is going, so a
    /// target that drops mid-rise switches to the release on that same frame rather than
    /// finishing a climb nothing is asking for any more.
    pub fn step(&mut self, target: f64, dt: f64) -> f64 {
        let tau = if target > self.value {
            self.attack_s
        } else {
            self.release_s
        };
        // A zero or negative time constant is "no smoothing at all" rather than a division by
        // zero, and a frame that took no time moves nothing.
        let at = if tau > 0.0 && dt > 0.0 {
            1.0 - (-dt / tau).exp()
        } else {
            1.0
        };
        self.value += (target - self.value) * at;
        self.value
    }
}
