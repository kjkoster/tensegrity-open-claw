//! Test mode: the cabinet door's buttons take the show over, to step through its scenes by
//! hand without a console.
//!
//! Holding both buttons for two seconds turns it on, starting from the scene the rig is in, and
//! the same again turns it off. While it is on, the left button steps to the next scene and the
//! motor runs for as long as the right button is held. The calendar and the sun are ignored; the
//! audio is not, so the strobe still follows the room. The e-stop still wins.
//!
//! The motor runs only when asked, never on its beat, so stepping through the scenes cannot
//! start it by surprise. And test mode ends itself after a quiet spell, because the rig runs
//! unattended, and one left in test mode would ignore the calendar for the rest of the event.

use crate::face::FaceState;
use crate::scene::{Cue, Scene};
use std::time::{Duration, Instant};

/// How long test mode lasts without a gesture.
const TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub struct TestMode {
    /// The face's counts as last read; the face counts from zero, and so does this.
    seen_chords: u32,
    seen_left_presses: u32,
    on: Option<On>,
}

struct On {
    scene: Scene,
    last_gesture: Instant,
}

impl TestMode {
    pub fn new() -> Self {
        Self {
            seen_chords: 0,
            seen_left_presses: 0,
            on: None,
        }
    }

    pub fn is_on(&self) -> bool {
        self.on.is_some()
    }

    /// The cue while test mode is on, or `None` while the calendar has the rig. `current` is
    /// the scene to start from when a chord turns it on.
    pub fn frame(&mut self, current: Scene, face: &FaceState) -> Option<Cue> {
        let now = Instant::now();
        let chords = face.chords.wrapping_sub(self.seen_chords);
        let steps = face.left_presses.wrapping_sub(self.seen_left_presses);
        self.seen_chords = face.chords;
        self.seen_left_presses = face.left_presses;
        let running = face.right_held;

        if chords % 2 == 1 {
            self.on = match self.on {
                None => Some(On {
                    scene: current,
                    last_gesture: now,
                }),
                Some(_) => None,
            };
        }
        let on = self.on.as_mut()?;

        if chords + steps > 0 || running {
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
        Some(Cue {
            scene: on.scene,
            motor: running,
            testing: true,
        })
    }
}
