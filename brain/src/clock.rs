//! The shared monotonic clock used across every pipeline stage (SPARKLE.md §0.2):
//! `now_us` stamps each `AudioFeatures` snapshot and every consumer's staleness
//! check reads against the same epoch.

use std::sync::OnceLock;
use std::time::Instant;

static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Monotonic microseconds since process start; the shared clock used for
/// `timestamp_us` and the consumers' staleness checks.
pub fn now_us() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}
