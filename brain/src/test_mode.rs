//! Test mode: the cabinet door's buttons take the show over, to step through its scenes by
//! hand without a console.
//!
//! Holding both buttons for two seconds turns it on, starting from the scene the rig is in, and
//! the same again turns it off. While it is on, the left button steps to the next scene and the
//! right button runs the motor once, there and then. The calendar and the sun are ignored; the
//! audio is not, so the strobe still follows the room. The e-stop still wins.
//!
//! The motor runs only when asked, never on its beat, so stepping through the scenes cannot
//! start it by surprise. And test mode ends itself after a quiet spell, because the rig runs
//! unattended, and one left in test mode would ignore the calendar for the rest of the event.

use crate::face::{self, Gestures};
use crate::scene::{Cue, Scene};
use std::time::{Duration, Instant};

/// How long test mode lasts without a gesture.
const TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// One motor run, as long as one on the show's beat.
const MOTOR_RUN: Duration = Duration::from_secs(1);

pub struct TestMode {
    /// The face's counts as last read.
    seen: Gestures,
    on: Option<On>,
}

struct On {
    scene: Scene,
    last_gesture: Instant,
    /// When the run the right button asked for ends.
    run_until: Option<Instant>,
}

impl TestMode {
    pub fn new() -> Self {
        Self {
            seen: face::gestures(),
            on: None,
        }
    }

    pub fn is_on(&self) -> bool {
        self.on.is_some()
    }

    /// The cue while test mode is on, or `None` while the calendar has the rig. `current` is
    /// the scene to start from when a chord turns it on.
    pub fn frame(&mut self, current: Scene, estop: bool) -> Option<Cue> {
        let now = Instant::now();
        let gestures = face::gestures();
        let chords = gestures.chords.wrapping_sub(self.seen.chords);
        let steps = gestures.left.wrapping_sub(self.seen.left);
        let runs = gestures.right.wrapping_sub(self.seen.right);
        self.seen = gestures;

        if chords % 2 == 1 {
            self.on = match self.on {
                None => Some(On {
                    scene: current,
                    last_gesture: now,
                    run_until: None,
                }),
                Some(_) => None,
            };
        }
        let on = self.on.as_mut()?;

        if chords + steps + runs > 0 {
            on.last_gesture = now;
        } else if now - on.last_gesture >= TIMEOUT {
            eprintln!(
                "show: test mode timed out after {} minutes without a press",
                TIMEOUT.as_secs() / 60
            );
            self.on = None;
            return None;
        }
        for _ in 0..steps {
            on.scene = on.scene.next();
        }
        if runs > 0 {
            on.run_until = Some(now + MOTOR_RUN);
        }
        if estop {
            on.run_until = None;
        }
        Some(Cue {
            scene: on.scene,
            motor: on.run_until.is_some_and(|until| now < until),
        })
    }
}
