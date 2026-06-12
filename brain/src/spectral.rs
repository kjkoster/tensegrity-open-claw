//! Slow path: FFT-derived spectral statistics (SOUND.md §5.4) and tempo
//! estimation (§5.5). Runs inline on the audio thread every ~250 ms; nothing
//! here sits on the latency-critical loudness→intensity path.

use crate::config as cfg;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::collections::VecDeque;
use std::sync::Arc;

const EPS: f32 = 1e-9;

#[derive(Clone, Copy, Default)]
pub struct SpectralOut {
    pub centroid: f32,
    pub spread: f32,
    pub flatness: f32,
    pub rolloff: f32,
    pub flux: f32,
}

#[derive(Clone, Copy, Default)]
pub struct TempoOut {
    pub bpm: f32,
    pub confidence: f32,
    pub beat_phase: f32,
}

pub struct SpectralProcessor {
    rate: f32,
    block_rate: f32,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    samples: VecDeque<f32>,
    scratch: Vec<Complex<f32>>,
    mag: Vec<f32>,
    prev_mag: Vec<f32>,
    since_fft: usize,
    interval_samples: usize,
    out: SpectralOut,
    // tempo
    onset_ring: VecDeque<f32>,
    onset_cap: usize,
    tempo: TempoOut,
    pending_bpm: f32,
    pending_s: f32,
}

impl SpectralProcessor {
    pub fn new(rate: f32, block_rate: f32) -> Self {
        let bins = cfg::FFT_SIZE / 2;
        // Hann window
        let window: Vec<f32> = (0..cfg::FFT_SIZE)
            .map(|i| {
                let p = i as f32 / (cfg::FFT_SIZE - 1) as f32;
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * p).cos()
            })
            .collect();
        Self {
            rate,
            block_rate,
            fft: FftPlanner::new().plan_fft_forward(cfg::FFT_SIZE),
            window,
            samples: VecDeque::with_capacity(cfg::FFT_SIZE),
            scratch: vec![Complex::new(0.0, 0.0); cfg::FFT_SIZE],
            mag: vec![0.0; bins],
            prev_mag: vec![0.0; bins],
            since_fft: 0,
            interval_samples: (cfg::FFT_INTERVAL_S * rate) as usize,
            out: SpectralOut::default(),
            onset_ring: VecDeque::new(),
            onset_cap: (cfg::TEMPO_RING_S * block_rate) as usize,
            tempo: TempoOut::default(),
            pending_bpm: 0.0,
            pending_s: 0.0,
        }
    }

    /// Feed one block of HPF'd mono samples plus the block's onset strength.
    /// Returns the current (held) spectral and tempo outputs.
    pub fn update(&mut self, mono: &[f32], onset_strength: f32, dt: f32) -> (SpectralOut, TempoOut) {
        for &s in mono {
            if self.samples.len() == cfg::FFT_SIZE {
                self.samples.pop_front();
            }
            self.samples.push_back(s);
        }
        if self.onset_ring.len() == self.onset_cap {
            self.onset_ring.pop_front();
        }
        self.onset_ring.push_back(onset_strength);

        self.since_fft += mono.len();
        if self.since_fft >= self.interval_samples && self.samples.len() == cfg::FFT_SIZE {
            self.since_fft = 0;
            self.compute_spectrum();
            self.compute_tempo();
        }

        // beat phase advances by wall clock between estimates
        if self.tempo.bpm > 0.0 {
            self.tempo.beat_phase = (self.tempo.beat_phase + dt * self.tempo.bpm / 60.0).fract();
        }

        (self.out, self.tempo)
    }

    fn compute_spectrum(&mut self) {
        for (i, &s) in self.samples.iter().enumerate() {
            self.scratch[i] = Complex::new(s * self.window[i], 0.0);
        }
        self.fft.process(&mut self.scratch);

        let bins = self.mag.len();
        let hz_per_bin = self.rate / cfg::FFT_SIZE as f32;
        for k in 0..bins {
            self.mag[k] = self.scratch[k].norm();
        }
        self.mag[0] = 0.0; // kill DC

        let sum: f32 = self.mag.iter().sum();
        if sum > EPS {
            // centroid + spread
            let mut weighted = 0.0f32;
            for k in 1..bins {
                weighted += k as f32 * hz_per_bin * self.mag[k];
            }
            let centroid_hz = weighted / sum;
            self.out.centroid = log_map_hz(centroid_hz);
            let mut var = 0.0f32;
            for k in 1..bins {
                let d = k as f32 * hz_per_bin - centroid_hz;
                var += self.mag[k] * d * d;
            }
            self.out.spread = ((var / sum).sqrt() / cfg::SPREAD_NORM_HZ).clamp(0.0, 1.0);

            // flatness: geometric / arithmetic mean (skip DC)
            let n = (bins - 1) as f32;
            let log_sum: f32 = self.mag[1..].iter().map(|&m| (m + EPS).ln()).sum();
            let gm = (log_sum / n).exp();
            let am = sum / n;
            self.out.flatness = (gm / (am + EPS)).clamp(0.0, 1.0);

            // rolloff: frequency below which ROLLOFF_FRACTION of energy lies
            let total_energy: f32 = self.mag.iter().map(|&m| m * m).sum();
            let mut cum = 0.0f32;
            let mut rolloff_hz = (bins - 1) as f32 * hz_per_bin;
            for k in 1..bins {
                cum += self.mag[k] * self.mag[k];
                if cum >= cfg::ROLLOFF_FRACTION * total_energy {
                    rolloff_hz = k as f32 * hz_per_bin;
                    break;
                }
            }
            self.out.rolloff = log_map_hz(rolloff_hz);

            // flux: positive bin-magnitude differences vs previous frame
            let mut flux = 0.0f32;
            for k in 0..bins {
                flux += (self.mag[k] - self.prev_mag[k]).max(0.0);
            }
            self.out.flux = (flux / (sum + EPS)).clamp(0.0, 1.0);
        } else {
            // hiss-free digital silence: noise-like flatness, no novelty
            self.out.flatness = 1.0;
            self.out.flux = 0.0;
        }
        self.prev_mag.copy_from_slice(&self.mag);
    }

    fn compute_tempo(&mut self) {
        let len = self.onset_ring.len();
        if (len as f32) < cfg::TEMPO_MIN_RING_S * self.block_rate {
            return;
        }

        let mut x: Vec<f32> = self.onset_ring.iter().copied().collect();
        let mean = x.iter().sum::<f32>() / len as f32;
        for v in &mut x {
            *v -= mean;
        }
        let denom: f32 = x.iter().map(|v| v * v).sum();
        if denom < EPS {
            self.tempo.confidence = 0.0;
            return;
        }

        let r_at = |lag: usize| -> f32 {
            if lag == 0 || lag >= len {
                return 0.0;
            }
            x[lag..]
                .iter()
                .zip(&x[..len - lag])
                .map(|(a, b)| a * b)
                .sum::<f32>()
                / denom
        };
        let lag_of = |bpm: f32| (self.block_rate * 60.0 / bpm).round() as usize;

        let mut best_bpm = 0.0f32;
        let mut best_r = f32::MIN;
        let mut sum_r = 0.0f32;
        let mut n_r = 0u32;
        for bpm in cfg::BPM_MIN..=cfg::BPM_MAX {
            let r = r_at(lag_of(bpm as f32));
            sum_r += r;
            n_r += 1;
            if r > best_r {
                best_r = r;
                best_bpm = bpm as f32;
            }
        }
        let mean_r = sum_r / n_r as f32;
        let confidence = ((best_r - mean_r) / (1.0 - mean_r).max(EPS)).clamp(0.0, 1.0);

        // octave-error correction, preferring BPM_PREF_MIN..BPM_PREF_MAX
        let mut chosen = best_bpm;
        let mut chosen_score = f32::MIN;
        for cand in [best_bpm, best_bpm * 2.0, best_bpm / 2.0] {
            if cand < cfg::BPM_MIN as f32 || cand > cfg::BPM_MAX as f32 {
                continue;
            }
            let bias = if (cfg::BPM_PREF_MIN..=cfg::BPM_PREF_MAX).contains(&cand) {
                cfg::BPM_PREF_BIAS
            } else {
                1.0
            };
            let score = r_at(lag_of(cand)) * bias;
            if score > chosen_score {
                chosen_score = score;
                chosen = cand;
            }
        }

        // hysteresis: accept only nearby refinements, high confidence, or persistence
        if (chosen - self.tempo.bpm).abs() < 3.0 || confidence > 0.8 {
            self.tempo.bpm = chosen;
            self.pending_s = 0.0;
        } else if (chosen - self.pending_bpm).abs() < 3.0 {
            self.pending_s += cfg::FFT_INTERVAL_S;
            if self.pending_s >= cfg::TEMPO_PERSIST_S {
                self.tempo.bpm = chosen;
                self.pending_s = 0.0;
            }
        } else {
            self.pending_bpm = chosen;
            self.pending_s = 0.0;
        }
        self.tempo.confidence = confidence;
    }
}

/// Log-scale mapping of a frequency to 0..1 over the configured centroid range.
fn log_map_hz(hz: f32) -> f32 {
    let lo = cfg::CENTROID_MIN_HZ.ln();
    let hi = cfg::CENTROID_MAX_HZ.ln();
    ((hz.max(1.0).ln() - lo) / (hi - lo)).clamp(0.0, 1.0)
}
