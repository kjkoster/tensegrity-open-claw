//! The ear: the rig's microphone input, listening on its own thread and handing the show
//! what it hears as [`AudioFeatures`].
//!
//! Like the face, the ear is a part of the rig rather than a patched fixture: it has no DMX
//! channels, but it has its own state, its own thread and its own client on the broker —
//! `health/ear`, and `ear/state`, `ear/xruns`, `ear/clipping`, `ear/error`,
//! `ear/usb/address`, `ear/usb/device` and `ear/usb/rate_hz`, retained.
//!
//! Capture reads blocks off the USB interface (`capture`), and per-block feature extraction
//! (`pipeline`, with its DSP building blocks) turns each into a fresh `AudioFeatures`. That snapshot is the contract between the ear and the show: a single Copy
//! struct carried over a latest-value seam. The ear never waits; the show never blocks.

mod capture;
mod dsp;
mod pipeline;

use crate::clock::now_us;
use crate::config as cfg;
use crate::latest::{LatestRx, latest};
use crate::telemetry::Publisher;
use std::thread::JoinHandle;

/// Starts the ear on its own thread, reporting to `telemetry`, and returns the reader of
/// what it hears. Until the first block arrives the reader holds a default snapshot, which
/// reads as silence.
pub fn spawn(telemetry: Publisher) -> (JoinHandle<()>, LatestRx<AudioFeatures>) {
    let (features, reader) = latest(AudioFeatures::default());
    (capture::spawn(features, telemetry), reader)
}

/// Written by the ear's thread at block rate, read by the DMX loop at the frame
/// rate. Levels are nominally 0..1 (post-AGC, post-floor).
///
/// Only what the show reads, plus the diagnostics the ear reports: everything here is
/// computed on every block, so a field nobody reads is work done for nothing.
#[derive(Clone, Copy, Default)]
pub struct AudioFeatures {
    pub timestamp_us: u64,
    /// Broadband loudness: the ball's strobe gap, and the music state machine's input.
    pub energy: f32,
    /// Loudness above the mid band: the ball's strobe flash length.
    pub energy_high: f32,
    /// Onsets so far, and how strong the last one was: the washes' glints. Counters survive
    /// latest-value snapshots where a flag would be missed between two frames.
    pub onset_count: u64,
    pub last_onset_strength: f32,
    /// Onsets of the low band alone, counted the same way: Heart's beat.
    pub kick_count: u64,
    /// 0..1, fading between silence and music: how much the glints count.
    pub music_amount: f32,
    /// 0 = SILENCE, 1 = MUSIC: pulse or strobe.
    pub state: u8,
    // diagnostics, for the ear's status line and `ear/xruns`
    pub noise_floor: f32,
    pub agc_ref: f32,
    pub xrun_count: u64,
}

impl AudioFeatures {
    /// The `state` the pipeline reports while it hears music.
    pub const MUSIC: u8 = 1;

    /// Whether this snapshot is too old to believe: a capture that has stalled must read as
    /// silence, not as the last thing it heard.
    pub fn is_stale(&self) -> bool {
        now_us().saturating_sub(self.timestamp_us) > cfg::STALE_US
    }

    /// Whether the room has music in it, by the state machine's discrete call.
    pub fn hears_music(&self) -> bool {
        !self.is_stale() && self.state == Self::MUSIC
    }
}
