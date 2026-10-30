//! How many times a second something happens, over a few seconds at a time: for a stage that
//! reports its frame rate.

use std::time::{Duration, Instant};

/// Long enough to average out a frame's jitter, short enough that a stage slowing down shows
/// within a few seconds.
const PERIOD: Duration = Duration::from_secs(5);

pub struct Rate {
    since: Instant,
    count: u32,
}

impl Rate {
    pub fn new() -> Rate {
        Rate {
            since: Instant::now(),
            count: 0,
        }
    }

    /// Counts one, and once a period has passed returns the rate over it and starts the next.
    pub fn tick(&mut self) -> Option<f64> {
        self.count += 1;
        let elapsed = self.since.elapsed();
        if elapsed < PERIOD {
            return None;
        }
        let rate = f64::from(self.count) / elapsed.as_secs_f64();
        *self = Rate::new();
        Some(rate)
    }
}
