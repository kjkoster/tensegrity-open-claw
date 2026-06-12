//! Fast path: per-block feature extraction (SOUND.md §4–§5) and the
//! silence ↔ music state machine (§7). Runs on the audio capture thread,
//! once per ALSA period. FFT-free; the slow path lives in spectral.rs.

use crate::config as cfg;
use crate::control::{now_us, ControlState};
use crate::dsp::{AgcRef, Biquad, EnvFollower, NoiseFloor, OnePole};
use crate::spectral::SpectralProcessor;
use std::collections::VecDeque;

const EPS: f32 = 1e-6;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Silence,
    Music,
}

pub struct FeaturePipeline {
    rate: f32,
    // §4.2–§4.3 filters
    hpf: Biquad,
    lp_low: Biquad,
    hp_mid: Biquad,
    lp_mid: Biquad,
    hp_high: Biquad,
    // §4.4 envelope followers (per sample)
    env_broad: EnvFollower,
    env_low: EnvFollower,
    env_mid: EnvFollower,
    env_high: EnvFollower,
    env_fast: EnvFollower, // onset detector input, not blunted by display smoothing
    // §4.5 / §4.6
    floor: NoiseFloor,
    agc: AgcRef,
    // §5.1 block-rate smoothers
    energy_slow: OnePole,
    bass_ratio: OnePole,
    tilt: OnePole,
    // §5.2 dynamics
    peak_smooth: OnePole,
    rms_smooth: OnePole,
    rms_mean: OnePole,
    rms_sq: OnePole,
    // §5.3 onsets
    prev_env_fast: f32,
    onset_mean: OnePole,
    onset_sq: OnePole,
    since_onset_s: f32,
    onset_count: u64,
    last_onset_strength: f32,
    onset_times: VecDeque<f64>,
    // §5.6 long horizon
    energy_3min: OnePole,
    quiet_seconds: f32,
    // §7 state machine
    state: State,
    above_s: f32,
    below_s: f32,
    music_amount: f32,
    // slow path
    spectral: SpectralProcessor,
    // bookkeeping
    clock_s: f64,
    seq: u64,
    xrun_count: u64,
    mono: Vec<f32>,
}

impl FeaturePipeline {
    /// `rate` and `period` are the *negotiated* ALSA values, never the requested ones.
    pub fn new(rate: f32, period: usize) -> Self {
        let block_rate = rate / period as f32;
        Self {
            rate,
            hpf: Biquad::highpass(rate, cfg::HPF_HZ, cfg::FILTER_Q),
            lp_low: Biquad::lowpass(rate, cfg::BAND_LOW_MAX_HZ, cfg::FILTER_Q),
            hp_mid: Biquad::highpass(rate, cfg::BAND_LOW_MAX_HZ, cfg::FILTER_Q),
            lp_mid: Biquad::lowpass(rate, cfg::BAND_MID_MAX_HZ, cfg::FILTER_Q),
            hp_high: Biquad::highpass(rate, cfg::BAND_MID_MAX_HZ, cfg::FILTER_Q),
            env_broad: EnvFollower::new(cfg::ENV_ATTACK_S, cfg::ENV_RELEASE_S, rate),
            env_low: EnvFollower::new(cfg::ENV_ATTACK_S, cfg::ENV_RELEASE_S, rate),
            env_mid: EnvFollower::new(cfg::ENV_ATTACK_S, cfg::ENV_RELEASE_S, rate),
            env_high: EnvFollower::new(cfg::ENV_ATTACK_S, cfg::ENV_RELEASE_S, rate),
            env_fast: EnvFollower::new(cfg::ONSET_ENV_ATTACK_S, cfg::ONSET_ENV_RELEASE_S, rate),
            floor: NoiseFloor::new(cfg::FLOOR_WINDOW_S, cfg::FLOOR_SUBWINDOWS, block_rate),
            agc: AgcRef::new(cfg::AGC_RISE_S, cfg::AGC_FALL_S, block_rate, cfg::AGC_MIN_REF),
            energy_slow: OnePole::new(cfg::ENERGY_SLOW_S, block_rate),
            bass_ratio: OnePole::new(cfg::BASS_RATIO_SMOOTH_S, block_rate),
            tilt: OnePole::new(cfg::TILT_SMOOTH_S, block_rate),
            peak_smooth: OnePole::new(cfg::CREST_SMOOTH_S, block_rate),
            rms_smooth: OnePole::new(cfg::CREST_SMOOTH_S, block_rate),
            rms_mean: OnePole::new(cfg::RMS_VAR_WINDOW_S, block_rate),
            rms_sq: OnePole::new(cfg::RMS_VAR_WINDOW_S, block_rate),
            prev_env_fast: 0.0,
            onset_mean: OnePole::new(cfg::ONSET_STATS_S, block_rate),
            onset_sq: OnePole::new(cfg::ONSET_STATS_S, block_rate),
            since_onset_s: f32::MAX,
            onset_count: 0,
            last_onset_strength: 0.0,
            onset_times: VecDeque::new(),
            energy_3min: OnePole::new(cfg::ENERGY_3MIN_S, block_rate),
            quiet_seconds: 0.0,
            state: State::Silence,
            above_s: 0.0,
            below_s: 0.0,
            music_amount: 0.0,
            spectral: SpectralProcessor::new(rate, block_rate),
            clock_s: 0.0,
            seq: 0,
            xrun_count: 0,
            mono: Vec::with_capacity(period),
        }
    }

    pub fn note_xrun(&mut self) {
        self.xrun_count += 1;
    }

    /// Process one ALSA period of interleaved stereo i16 samples and return
    /// the fresh ControlState to publish.
    pub fn process_block(&mut self, interleaved: &[i16]) -> ControlState {
        let frames = interleaved.len() / cfg::CHANNELS;
        let dt = frames as f32 / self.rate;
        self.clock_s += dt as f64;

        // §4.1 mono sum + per-sample filter and envelope chain
        let mut sum_sq = 0.0f32;
        let mut block_peak = 0.0f32;
        self.mono.clear();
        for f in 0..frames {
            let l = interleaved[f * cfg::CHANNELS] as f32;
            let r = interleaved[f * cfg::CHANNELS + 1] as f32;
            let x = self.hpf.process((l + r) * 0.5 / 32768.0);
            sum_sq += x * x;
            block_peak = block_peak.max(x.abs());
            let low = self.lp_low.process(x);
            let mid = self.lp_mid.process(self.hp_mid.process(x));
            let high = self.hp_high.process(x);
            self.env_broad.process(x);
            self.env_low.process(low);
            self.env_mid.process(mid);
            self.env_high.process(high);
            self.env_fast.process(x);
            self.mono.push(x);
        }
        let block_rms = (sum_sq / frames.max(1) as f32).sqrt();
        let eb = self.env_broad.value();
        let el = self.env_low.value();
        let em = self.env_mid.value();
        let eh = self.env_high.value();
        let ef = self.env_fast.value();

        // §4.5 noise floor (frozen in MUSIC) and §4.6 AGC (frozen outside MUSIC)
        self.floor.frozen = self.state == State::Music;
        self.floor.update(eb);
        let effective_floor = self.floor.floor() * cfg::FLOOR_MARGIN;
        self.agc.frozen = self.state != State::Music;
        self.agc.update(eb);
        let agc_ref = self.agc.reference();

        let norm = |e: f32| {
            ((e - effective_floor).max(0.0) / (agc_ref - effective_floor).max(EPS))
                .clamp(0.0, cfg::NORM_HEADROOM)
        };

        // §5.1 energy family
        let energy = norm(eb);
        let energy_low = norm(el);
        let energy_mid = norm(em);
        let energy_high = norm(eh);
        let energy_slow = self.energy_slow.process(energy.min(1.0));
        let bass_ratio = self.bass_ratio.process(el / (eb + EPS));
        let tilt = self.tilt.process((eh - el) / (eh + el + EPS));

        // §5.2 dynamics
        let peak = self.peak_smooth.process(block_peak);
        let rms = self.rms_smooth.process(block_rms);
        let crest_db = 20.0 * ((peak + EPS) / (rms + EPS)).log10();
        let crest = ((crest_db - 3.0) / 17.0).clamp(0.0, 1.0);
        let m = self.rms_mean.process(block_rms);
        let m2 = self.rms_sq.process(block_rms * block_rms);
        let rms_var = ((m2 - m * m).max(0.0).sqrt() / (m + EPS)).clamp(0.0, 1.0);

        // §5.3 onsets: rectified derivative of the fast envelope, adaptive threshold
        let onset_strength = ((ef - self.prev_env_fast).max(0.0) / (agc_ref + EPS)).min(4.0);
        self.prev_env_fast = ef;
        let omean = self.onset_mean.process(onset_strength);
        let osq = self.onset_sq.process(onset_strength * onset_strength);
        let ostd = (osq - omean * omean).max(0.0).sqrt();
        self.since_onset_s = (self.since_onset_s + dt).min(f32::MAX / 2.0);
        if onset_strength > omean + cfg::ONSET_K * ostd
            && onset_strength > cfg::ONSET_MIN_STRENGTH
            && self.since_onset_s >= cfg::ONSET_REFRACTORY_S
        {
            self.onset_count += 1;
            self.last_onset_strength = onset_strength.min(cfg::NORM_HEADROOM);
            self.since_onset_s = 0.0;
            self.onset_times.push_back(self.clock_s);
        }
        let horizon = self.clock_s - cfg::ONSET_DENSITY_WINDOW_S as f64;
        while self.onset_times.front().is_some_and(|&t| t < horizon) {
            self.onset_times.pop_front();
        }
        let onset_density = ((self.onset_times.len() as f32 / cfg::ONSET_DENSITY_WINDOW_S)
            / cfg::ONSET_DENSITY_FULL_HZ)
            .clamp(0.0, 1.0);

        // §5.6 long horizon
        let energy_3min = self.energy_3min.process(energy.min(1.0));
        if energy > cfg::UP_THRESHOLD {
            self.quiet_seconds = 0.0;
        } else {
            self.quiet_seconds += dt;
        }

        // §5.4 / §5.5 slow path (FFT + tempo), inline every Nth block
        let (spec, tempo) = self.spectral.update(&self.mono, onset_strength, dt);

        // §7 state machine with hysteresis and hold times
        match self.state {
            State::Silence => {
                self.above_s = if energy > cfg::UP_THRESHOLD {
                    self.above_s + dt
                } else {
                    0.0
                };
                if self.above_s > cfg::UP_HOLD_S {
                    self.state = State::Music;
                    self.below_s = 0.0;
                    eprintln!("audio: state SILENCE → MUSIC");
                }
            }
            State::Music => {
                self.below_s = if energy < cfg::DOWN_THRESHOLD {
                    self.below_s + dt
                } else {
                    0.0
                };
                if self.below_s > cfg::DOWN_HOLD_S {
                    self.state = State::Silence;
                    self.above_s = 0.0;
                    eprintln!("audio: state MUSIC → SILENCE");
                }
            }
        }
        let target = if self.state == State::Music { 1.0 } else { 0.0 };
        let slew = if target > self.music_amount {
            dt / cfg::FADE_UP_S
        } else {
            dt / cfg::FADE_DOWN_S
        };
        self.music_amount += (target - self.music_amount).clamp(-slew, slew);

        self.seq += 1;
        ControlState {
            seq: self.seq,
            timestamp_us: now_us(),
            energy,
            energy_low,
            energy_mid,
            energy_high,
            energy_slow,
            bass_ratio,
            tilt,
            crest,
            rms_var,
            onset_strength,
            onset_count: self.onset_count,
            last_onset_strength: self.last_onset_strength,
            onset_density,
            centroid: spec.centroid,
            spread: spec.spread,
            flatness: spec.flatness,
            rolloff: spec.rolloff,
            spectral_flux: spec.flux,
            bpm: tempo.bpm,
            tempo_confidence: tempo.confidence,
            beat_phase: tempo.beat_phase,
            energy_3min,
            quiet_seconds: self.quiet_seconds,
            music_amount: self.music_amount,
            state: if self.state == State::Music { 1 } else { 0 },
            noise_floor: effective_floor,
            agc_ref,
            xrun_count: self.xrun_count,
        }
    }
}
