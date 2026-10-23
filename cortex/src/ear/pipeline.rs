//! Per-block feature extraction and the silence ↔ music state machine. Runs on the ear's
//! thread, once per ALSA period.

use super::dsp::{AgcRef, Biquad, EnvFollower, NoiseFloor, OnsetDetector};
use super::AudioFeatures;
use crate::clock::now_us;
use crate::config as cfg;

const EPS: f32 = 1e-6;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Silence,
    Music,
}

pub struct FeaturePipeline {
    rate: f32,
    // filters
    hpf: Biquad,
    lp_low: Biquad,
    hp_high: Biquad,
    // envelope followers (per sample)
    env_broad: EnvFollower,
    env_high: EnvFollower,
    env_fast: EnvFollower, // onset detector input, not blunted by display smoothing
    env_kick: EnvFollower, // the same for the low band: the kick detector's input
    // noise floor and AGC
    floor: NoiseFloor,
    agc: AgcRef,
    // onsets
    onsets: OnsetDetector,
    onset_count: u64,
    last_onset_strength: f32,
    // kicks: onsets in the low band alone, where hats and offbeats do not reach
    kicks: OnsetDetector,
    kick_count: u64,
    // state machine
    state: State,
    above_s: f32,
    below_s: f32,
    music_amount: f32,
    xrun_count: u64,
}

impl FeaturePipeline {
    /// `rate` and `period` are the *negotiated* ALSA values, never the requested ones.
    pub fn new(rate: f32, period: usize) -> Self {
        let block_rate = rate / period as f32;
        Self {
            rate,
            hpf: Biquad::highpass(rate, cfg::HPF_HZ, cfg::FILTER_Q),
            lp_low: Biquad::lowpass(rate, cfg::BAND_LOW_MAX_HZ, cfg::FILTER_Q),
            hp_high: Biquad::highpass(rate, cfg::BAND_HIGH_MIN_HZ, cfg::FILTER_Q),
            env_broad: EnvFollower::new(cfg::ENV_ATTACK_S, cfg::ENV_RELEASE_S, rate),
            env_high: EnvFollower::new(cfg::ENV_ATTACK_S, cfg::ENV_RELEASE_S, rate),
            env_fast: EnvFollower::new(cfg::ONSET_ENV_ATTACK_S, cfg::ONSET_ENV_RELEASE_S, rate),
            env_kick: EnvFollower::new(cfg::ONSET_ENV_ATTACK_S, cfg::ONSET_ENV_RELEASE_S, rate),
            floor: NoiseFloor::new(cfg::FLOOR_WINDOW_S, cfg::FLOOR_SUBWINDOWS, block_rate),
            agc: AgcRef::new(cfg::AGC_RISE_S, cfg::AGC_FALL_S, block_rate, cfg::AGC_MIN_REF),
            onsets: OnsetDetector::new(
                cfg::ONSET_STATS_S,
                cfg::ONSET_K,
                cfg::ONSET_MIN_STRENGTH,
                cfg::ONSET_REFRACTORY_S,
                block_rate,
            ),
            onset_count: 0,
            last_onset_strength: 0.0,
            kicks: OnsetDetector::new(
                cfg::KICK_STATS_S,
                cfg::KICK_K,
                cfg::KICK_MIN_STRENGTH,
                cfg::KICK_REFRACTORY_S,
                block_rate,
            ),
            kick_count: 0,
            state: State::Silence,
            above_s: 0.0,
            below_s: 0.0,
            music_amount: 0.0,
            xrun_count: 0,
        }
    }

    pub fn note_xrun(&mut self) {
        self.xrun_count += 1;
    }

    /// Process one ALSA period of interleaved stereo i16 samples and return
    /// the fresh AudioFeatures to publish.
    pub fn process_block(&mut self, interleaved: &[i16]) -> AudioFeatures {
        let frames = interleaved.len() / cfg::CHANNELS;
        let dt = frames as f32 / self.rate;

        // mono sum + per-sample filter and envelope chain
        for f in 0..frames {
            let l = interleaved[f * cfg::CHANNELS] as f32;
            let r = interleaved[f * cfg::CHANNELS + 1] as f32;
            let x = self.hpf.process((l + r) * 0.5 / 32768.0);
            let low = self.lp_low.process(x);
            let high = self.hp_high.process(x);
            self.env_broad.process(x);
            self.env_high.process(high);
            self.env_fast.process(x);
            self.env_kick.process(low);
        }
        let eb = self.env_broad.value();
        let eh = self.env_high.value();
        let ef = self.env_fast.value();
        let ek = self.env_kick.value();

        // noise floor (frozen in MUSIC) and AGC (frozen outside MUSIC)
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
        let energy = norm(eb);
        let energy_high = norm(eh);

        // onsets: rectified derivative of the fast envelope, adaptive threshold
        let (onset_strength, onset) = self.onsets.process(ef, agc_ref, dt);
        if onset {
            self.onset_count += 1;
            self.last_onset_strength = onset_strength.min(cfg::NORM_HEADROOM);
        }
        // The fast low-band envelope, not a display-smoothed one: that one's release holds it
        // up through a bass line, and the next kick would barely rise above it.
        if self.kicks.process(ek, agc_ref, dt).1 {
            self.kick_count += 1;
        }

        // state machine with hysteresis and hold times
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
                    eprintln!("ear: state SILENCE → MUSIC");
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
                    eprintln!("ear: state MUSIC → SILENCE");
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

        AudioFeatures {
            timestamp_us: now_us(),
            energy,
            energy_high,
            onset_count: self.onset_count,
            last_onset_strength: self.last_onset_strength,
            kick_count: self.kick_count,
            music_amount: self.music_amount,
            state: if self.state == State::Music { AudioFeatures::MUSIC } else { 0 },
            noise_floor: effective_floor,
            agc_ref,
            xrun_count: self.xrun_count,
        }
    }
}
