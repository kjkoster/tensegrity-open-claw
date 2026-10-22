//! Test mode: the cabinet door's buttons take the show over, to step through its show states
//! by hand without a console.
//!
//! Holding both buttons for two seconds turns it on, starting from the show state the show is
//! in, and the same again turns it off. While it is on, the left button steps to the next show
//! state — the base states, then the incidents — and the motor runs for as long as the right
//! button is held. The calendar, the sun and the incident schedule are ignored, so an incident
//! can be seen before it is enabled; the audio is not, so the strobe and Heart still follow the
//! room.
//!
//! The motor runs only when asked, never on its beat, so stepping through the show states cannot
//! start it by surprise. And test mode ends itself after a quiet spell, because the rig runs
//! unattended, and one left in test mode would ignore the calendar for the rest of the event.

use crate::face::FaceState;
use crate::show_state::ShowState;
use std::time::{Duration, Instant};

/// How long test mode lasts without a gesture.
const TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// What test mode asks for this frame.
pub struct Manual {
    pub state: ShowState,
    pub motor: bool,
}

pub struct TestMode {
    /// The face's counts as last read; the face counts from zero, and so does this.
    seen_chords: u32,
    seen_left_presses: u32,
    on: Option<On>,
}

struct On {
    state: ShowState,
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

    /// What test mode asks for while it is on, or `None` while the calendar has the show.
    /// `current` is the show state to start from when a chord turns it on.
    pub fn frame(&mut self, current: ShowState, face: &FaceState) -> Option<Manual> {
        let now = Instant::now();
        let chords = face.chords.wrapping_sub(self.seen_chords);
        let steps = face.left_presses.wrapping_sub(self.seen_left_presses);
        self.seen_chords = face.chords;
        self.seen_left_presses = face.left_presses;
        let running = face.right_held;

        if chords % 2 == 1 {
            self.on = match self.on {
                None => Some(On {
                    state: current,
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
            on.state = on.state.next();
        }
        Some(Manual {
            state: on.state,
            motor: running,
        })
    }
}
