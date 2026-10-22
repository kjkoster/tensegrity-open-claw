//! Heart's beat: a lub-dub on the kick drum, never slower than 60 BPM.
//!
//! A beat comes on the first kick at least a refractory after the last beat, or at the longest
//! gap if no kick has come by then — so the heart is at 60 BPM in silence, between kicks, and
//! when the ear is gone, and locks to the kick when one falls between the two. That window
//! holds every kick from 60 to 75 BPM and every second kick from 120 to 150; between 75 and
//! 120 the kicks mostly miss it, and the heart runs at 60, catching a kick when one lands
//! inside. Only the gap between beats moves; the lub-dub itself has one fixed shape, soft at
//! both ends.
//!
//! Stepped every frame, like the sparkle, so Heart enters mid-rhythm rather than on a beat of
//! its own.

use cortex::ear::AudioFeatures;
use std::f64::consts::PI;

/// No beat sooner than this after the last.
const REFRACTORY_S: f64 = 0.8;
/// A beat no later than this after the last, kick or no kick.
const LONGEST_GAP_S: f64 = 1.0;
/// The lub's peak after the beat, the dub's after the lub, and how far either spreads to each
/// side of its peak.
const LUB_AT_S: f64 = 0.08;
const DUB_AFTER_S: f64 = 0.12;
const SPREAD_S: f64 = 0.08;
/// The dub's height against the lub's.
const DUB_HEIGHT: f64 = 0.8;
/// Where the light rests between beats, as a fraction of full.
const RESTING_LEVEL: f64 = 0.2;

pub struct Heart {
    since_beat_s: f64,
    seen_kicks: u64,
}

impl Heart {
    pub fn new() -> Self {
        Self {
            since_beat_s: LONGEST_GAP_S,
            seen_kicks: 0,
        }
    }

    /// The lub-dub's height this frame, `dt` seconds after the last: 0 between beats, 1 at the
    /// lub's peak.
    pub fn frame(&mut self, features: &AudioFeatures, dt: f64) -> f64 {
        let kicked = features.kick_count != self.seen_kicks;
        self.seen_kicks = features.kick_count;
        self.since_beat_s += dt;
        if (kicked && self.since_beat_s >= REFRACTORY_S) || self.since_beat_s >= LONGEST_GAP_S {
            self.since_beat_s = 0.0;
        }
        let lub = bump(self.since_beat_s - LUB_AT_S);
        let dub = DUB_HEIGHT * bump(self.since_beat_s - LUB_AT_S - DUB_AFTER_S);
        lub.max(dub)
    }
}

/// The light for a height of the beat: the resting level between beats, full at the lub.
pub fn level(height: f64) -> f64 {
    RESTING_LEVEL + (1.0 - RESTING_LEVEL) * height
}

/// A raised cosine of half-width [`SPREAD_S`], 1 at `offset` 0.
fn bump(offset: f64) -> f64 {
    let x = offset / SPREAD_S;
    if x.abs() >= 1.0 { 0.0 } else { 0.5 * (1.0 + (PI * x).cos()) }
}
