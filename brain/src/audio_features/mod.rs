//! The audio-features pipeline stage (SPARKLE.md §0.1, §0.3): per-block feature
//! extraction (`pipeline`, with its DSP and spectral building blocks) and the
//! `AudioFeatures` snapshot it produces.
//!
//! `AudioFeatures` (formerly `ControlState`) is the published contract between the
//! capture/feature thread and its consumers (SOUND.md §6, §8.1): a single Copy
//! struct carried over a latest-value `Latest` seam. The producer never waits;
//! consumers never block.

mod dsp;
mod pipeline;
mod spectral;

pub use pipeline::FeaturePipeline;

/// Written by the capture thread at block rate, read by the DMX loop at 44 Hz
/// and by the recorder at 10 Hz. All features are nominally 0..1 (post-AGC,
/// post-floor) unless noted.
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
    pub music_amount: f32, // 0..1 crossfade, see §7
    pub state: u8,         // 0 = SILENCE, 1 = MUSIC (informational)
    // diagnostics
    pub noise_floor: f32,
    pub agc_ref: f32,
    pub xrun_count: u64,
}
