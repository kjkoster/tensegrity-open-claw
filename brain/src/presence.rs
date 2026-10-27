//! Presence: the distance from the person who has come furthest into the yard to the trigger
//! line, in paths — 1 at the yard entry, 0 at the line, negative past it — or the tombstone when
//! nobody is in the yard.
//!
//! The eyeball is not built yet, so outside test mode the yard is empty. In test mode the left
//! button steps through a few distances in its place, so the heart can be heard quickening.
//!
//! Debouncing is the show's, here: the scare needs the distance at or past the line for
//! `SCARE_CONFIRM_SECS`, and the yard counts as empty once it has been for `LEAVE_SECS`, so a
//! detector that loses someone for a frame neither scares nor resets the show.
//!
//! Published on `brain/presence` when it changes, as the distance rounded to a hundredth, or
//! `tombstone`.

use crate::show::{LEAVE_SECS, SCARE_CONFIRM_SECS, frames};
use cortex::telemetry::Publisher;

/// Nobody in the yard. Infinitely far is what an empty yard means, so every comparison the show
/// makes falls the safe way without a special case.
pub const TOMBSTONE: f64 = f64::INFINITY;

/// What test mode puts in the yard, a step per press of the left button: nobody, someone
/// halfway up the path, and someone just short of the trigger line — near enough to race the
/// heart, never past the line, so only the right button scares.
const TEST_STEPS: [f64; 3] = [TOMBSTONE, 0.5, 0.1];

/// What presence says this frame.
pub struct Reading {
    pub distance: f64,
    /// The scare is due: the distance crossed the line and has stayed at or past it for
    /// `SCARE_CONFIRM_SECS`. True on that one frame only.
    pub scare: bool,
    /// The yard has been empty for `LEAVE_SECS`.
    pub vacated: bool,
}

pub struct Presence {
    telemetry: Publisher,
    /// Which of `TEST_STEPS` test mode has chosen.
    test_step: usize,
    previous: f64,
    /// Frames the distance has been at or past the line since it crossed it. `None` while it is
    /// short of the line, and when it got there from the tombstone: someone appearing past the
    /// line, coming back from the door, has not walked up the path and is not scared.
    past_line: Option<u32>,
    /// Frames the yard has been empty.
    empty: u32,
    published: String,
}

impl Presence {
    pub fn new(telemetry: Publisher) -> Self {
        let published = word(TOMBSTONE);
        telemetry.publish("presence", published.clone(), true);
        Self {
            telemetry,
            test_step: 0,
            previous: TOMBSTONE,
            past_line: None,
            empty: 0,
            published,
        }
    }

    /// Steps test mode's distance on to the next one, back to nobody after the last.
    pub fn cycle(&mut self) {
        self.test_step = (self.test_step + 1) % TEST_STEPS.len();
    }

    /// Empties the yard test mode filled, for when it ends.
    pub fn reset(&mut self) {
        self.test_step = 0;
    }

    pub fn step(&mut self, testing: bool) -> Reading {
        let distance = if testing {
            TEST_STEPS[self.test_step]
        } else {
            TOMBSTONE
        };

        self.past_line = match self.past_line {
            _ if distance > 0.0 => None,
            Some(held) => Some(held.saturating_add(1)),
            None if self.previous > 0.0 && self.previous.is_finite() => Some(1),
            None => None,
        };
        self.previous = distance;
        self.empty = if distance == TOMBSTONE {
            self.empty.saturating_add(1)
        } else {
            0
        };

        let published = word(distance);
        if published != self.published {
            self.telemetry.publish("presence", published.clone(), true);
            self.published = published;
        }

        Reading {
            distance,
            scare: self.past_line == Some(frames(SCARE_CONFIRM_SECS)),
            vacated: self.empty >= frames(LEAVE_SECS),
        }
    }
}

fn word(distance: f64) -> String {
    if distance == TOMBSTONE {
        "tombstone".to_string()
    } else {
        format!("{distance:.2}")
    }
}
