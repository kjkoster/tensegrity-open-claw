//! The heart: one oscillator that the rose and the heartbeat both follow, so the two never
//! drift apart.
//!
//! Each beat cues the heartbeat recording once, and the rose plays the recording's own
//! envelope from that beat, between `ROSE_FLOOR` and full: the lub and the dub land where the
//! recording puts them, and a different recording brings its own pulse with it.
//!
//! The pace is the distance. The target is `REST_BPM` at `HEART_RANGE` and beyond, `SCARE_BPM`
//! at the trigger line, linear between, and the rate glides toward it with `RATE_TAU`, because
//! detection jitter must not make the heart stutter. The rate is taken once per beat, at the
//! beat, so a lub-dub is never cut short by a change of pace.

use crate::show::{BUILDUP_SECS, HEART_RANGE, RATE_TAU, REST_BPM, ROSE_FLOOR, SCARE_BPM, frames};
use cortex::config::FRAME_RATE_HZ;
use cortex::envelope::Envelope;

pub struct Heart {
    /// The heartbeat's loudness per frame.
    envelope: &'static [u8],
    /// Beats a minute above `REST_BPM`. Above rest rather than the rate itself, so a heart
    /// started afresh starts at rest.
    quickening: Envelope,
    /// Frames until the next beat. Fractional, so a period that is not a whole number of
    /// frames still keeps its pace.
    until_beat: f64,
    /// Frames since the latest beat and the one before it. Two, because at a fast pace a beat
    /// comes before the last one's envelope has played out, and the rose takes the louder.
    since: [Option<usize>; 2],
}

impl Heart {
    pub fn new(envelope: &'static [u8]) -> Self {
        Self {
            envelope,
            quickening: Envelope::new(RATE_TAU, RATE_TAU),
            until_beat: 0.0,
            since: [None; 2],
        }
    }

    /// Starts the heart afresh, at rest: the rose dark for `BUILDUP_SECS` while the heads travel
    /// to it, then the first beat.
    pub fn restart(&mut self) {
        self.quickening.reset();
        self.until_beat = f64::from(frames(BUILDUP_SECS));
        self.since = [None; 2];
    }

    /// Steps one frame with the furthest person at `distance`. Returns whether the heart beat on
    /// this frame, and the rose's level, 0..1.
    pub fn step(&mut self, distance: f64) -> (bool, f64) {
        let frame_rate = FRAME_RATE_HZ as f64;
        self.quickening
            .step(target_bpm(distance) - REST_BPM, 1.0 / frame_rate);

        let beat = self.until_beat <= 0.0;
        if beat {
            let bpm = REST_BPM + self.quickening.value();
            self.until_beat += 60.0 / bpm * frame_rate;
            self.since = [Some(0), self.since[0]];
        }
        self.until_beat -= 1.0;

        let rose = match self.since[0] {
            None => 0.0,
            Some(_) => {
                let loudest = self
                    .since
                    .iter()
                    .flatten()
                    .filter_map(|&frame| self.envelope.get(frame))
                    .copied()
                    .max()
                    .unwrap_or(0);
                ROSE_FLOOR + (1.0 - ROSE_FLOOR) * f64::from(loudest) / f64::from(u8::MAX)
            }
        };
        for frame in self.since.iter_mut().flatten() {
            *frame += 1;
        }
        (beat, rose)
    }
}

/// The rate the heart glides toward. The tombstone, `+∞`, clamps to rest like any distance
/// beyond the range.
fn target_bpm(distance: f64) -> f64 {
    let far = (distance / HEART_RANGE).clamp(0.0, 1.0);
    SCARE_BPM + (REST_BPM - SCARE_BPM) * far
}
