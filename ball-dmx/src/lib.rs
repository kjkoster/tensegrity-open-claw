//! What a ball's DMX values mean where its fixture definition cannot say it.
//!
//! A `.qxf` describes a channel in bands of values, and has no way to say that a value is a
//! length of time. The ball renders its strobe from two such lengths, and the brain chooses
//! them, so the unit they are carried in lives here, where both sides read the one copy.

#![no_std]

/// What one step of the `Strobe on` and `Strobe off` channels is worth: 0–1020 ms in all,
/// which spans the shortest flash the eye still catches and a trough long enough to read as
/// a pause.
pub const STROBE_STEP_MS: u32 = 4;
