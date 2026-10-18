//! The ear: the rig's microphone input, listening on its own thread and handing the show
//! what it hears as [`AudioFeatures`].
//!
//! Like the face, the ear is a part of the rig rather than a patched fixture: it has no DMX
//! channels, but it has its own state, its own thread and its own client on the broker —
//! `health/ear`, and `ear/state`, `ear/xruns`, `ear/clipping`, `ear/error`,
//! `ear/usb/address`, `ear/usb/device` and `ear/usb/rate_hz`, retained.
//!
//! Capture reads blocks off the USB interface (`capture`), and per-block feature extraction
//! (`pipeline`, with its DSP and spectral building blocks) turns each into a fresh
//! `AudioFeatures`. That snapshot is the contract between the ear and the show: a single Copy
//! struct carried over a latest-value seam. The ear never waits; the show never blocks.

mod capture;
mod dsp;
mod pipeline;
mod spectral;

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
/// rate. All features are nominally 0..1 (post-AGC, post-floor) unless noted.
#[derive(Clone, Copy, Default)]
pub struct AudioFeatures {
    pub seq: u64,
    pub timestamp_us: u64,
    // energy family
    pub energy: f32,
    pub energy_low: f32,
    pub energy_mid: f32,
    pub energy_high: f32,
    pub energy_slow: f32,
    pub bass_ratio: f32,
    pub tilt: f32, // −1..+1
    // dynamics
    pub crest: f32,
    pub rms_var: f32,
    // onsets
    pub onset_strength: f32,
    pub onset_count: u64, // counters survive latest-value snapshots; flags would not
    pub last_onset_strength: f32,
    pub onset_density: f32,
    // spectral (slow path; holds last value between updates)
    pub centroid: f32,
    pub spread: f32,
    pub flatness: f32,
    pub rolloff: f32,
    pub spectral_flux: f32,
    // tempo (never load-bearing; gate on tempo_confidence)
    pub bpm: f32,
    pub tempo_confidence: f32,
    pub beat_phase: f32,
    // long horizon
    pub energy_3min: f32,
    pub quiet_seconds: f32,
    // state machine
    pub music_amount: f32, // 0..1 crossfade between silence and music
    pub state: u8,         // 0 = SILENCE, 1 = MUSIC (informational)
    // diagnostics
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
