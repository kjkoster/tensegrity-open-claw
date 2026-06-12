//! The published contract between the audio thread and its consumers
//! (SOUND.md §6, §8.1): a single Copy struct behind an arc_swap slot with
//! latest-value semantics. The producer never waits; consumers never block.

use arc_swap::ArcSwap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// Written by the audio thread at block rate, read by the DMX loop at 44 Hz
/// and by the recorder at 10 Hz. All features are nominally 0..1 (post-AGC,
/// post-floor) unless noted.
#[derive(Clone, Copy, Default)]
pub struct ControlState {
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

pub fn control_pair() -> (ControlPublisher, ControlReader) {
    let slot = Arc::new(ArcSwap::from_pointee(ControlState::default()));
    (ControlPublisher(slot.clone()), ControlReader(slot))
}

/// Single-writer side, owned by the audio thread.
pub struct ControlPublisher(Arc<ArcSwap<ControlState>>);

impl ControlPublisher {
    pub fn publish(&self, state: ControlState) {
        self.0.store(Arc::new(state));
    }
}

/// Reader side; cheap to clone, lock-free to read.
#[derive(Clone)]
pub struct ControlReader(Arc<ArcSwap<ControlState>>);

impl ControlReader {
    pub fn snapshot(&self) -> ControlState {
        **self.0.load()
    }
}

static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Monotonic microseconds since process start; the shared clock used for
/// `timestamp_us` and the consumers' staleness checks.
pub fn now_us() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}
