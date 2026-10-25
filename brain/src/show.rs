//! The placeholder show: dark, with test mode on the cabinet door.
//!
//! It stands in while the Halloween show is built, so the brain runs and deploys with nothing
//! on it. It is always in `blackout`. Holding both buttons for two seconds turns test mode on,
//! which lights the door's lamps, and the same again turns it off. Test mode ends itself after
//! fifteen minutes without a chord, because a rig left in it would ignore the calendar
//! unattended.
//!
//! The show state is the brain's own: `brain/show` and `brain/testmode`, retained,
//! published on every change, and logged alongside.

use crate::face::FaceState;
use crate::face_show;
use cortex::telemetry::Publisher;
use std::time::{Duration, Instant};

/// How long test mode lasts without a chord.
const TEST_MODE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub struct Show {
    telemetry: Publisher,
    /// The face's chord count as last read; the face counts from zero, and so does this.
    seen_chords: u32,
    /// When test mode was last turned on, while it is on.
    testing_since: Option<Instant>,
}

impl Show {
    pub fn new(telemetry: Publisher) -> Self {
        eprintln!("show: blackout");
        telemetry.publish("show", "blackout", true);
        eprintln!("show: test mode off");
        telemetry.publish("testmode", stem::boolean(false), true);
        Self {
            telemetry,
            seen_chords: 0,
            testing_since: None,
        }
    }

    /// Fills this frame's slots, given what the door's buttons say.
    pub fn fill(&mut self, slots: &mut [u8], face: &FaceState) {
        let chords = face.chords.wrapping_sub(self.seen_chords);
        self.seen_chords = face.chords;

        if chords % 2 == 1 {
            self.set_testing(self.testing_since.is_none());
        } else if self
            .testing_since
            .is_some_and(|since| since.elapsed() >= TEST_MODE_TIMEOUT)
        {
            eprintln!(
                "show: test mode timed out after {} minutes without a chord",
                TEST_MODE_TIMEOUT.as_secs() / 60
            );
            self.set_testing(false);
        }

        face_show::fill(slots, self.testing_since.is_some());
    }

    fn set_testing(&mut self, on: bool) {
        self.testing_since = on.then(Instant::now);
        eprintln!("show: test mode {}", if on { "on" } else { "off" });
        self.telemetry
            .publish("testmode", stem::boolean(on), true);
    }
}
