//! Mapping layer (SOUND.md §9): pure per-frame arithmetic from a ControlState
//! snapshot to Perlin/intensity parameters. The breathing baseline is always
//! present underneath; audio modulation is scaled by music_amount. Every
//! output passes through a per-parameter slew limiter as the last defense
//! against flicker.

use crate::config as cfg;
use crate::control::{now_us, ControlState};

pub struct MapOut {
    pub intensity: f64, // 0..1
    pub speed: f64,     // Perlin cells per second
    pub octave2: f64,   // second-octave amplitude, 0..OCTAVE2_MAX
    pub w_gain: f64,    // W channel trim
}

/// One-pole slew limiter with a per-parameter time constant.
struct Slew {
    tau_s: f64,
    y: f64,
}

impl Slew {
    fn new(tau_s: f64, initial: f64) -> Self {
        Self { tau_s, y: initial }
    }

    fn step(&mut self, target: f64, dt: f64) -> f64 {
        self.y += (target - self.y) * (1.0 - (-dt / self.tau_s).exp());
        self.y
    }
}

pub struct Mapping {
    eff_music: f64,
    last_onset_count: u64,
    accent_strength: f64,
    since_onset_s: f64,
    intensity: Slew,
    speed: Slew,
    octave2: Slew,
    w_gain: Slew,
}

impl Mapping {
    pub fn new() -> Self {
        Self {
            eff_music: 0.0,
            last_onset_count: 0,
            accent_strength: 0.0,
            since_onset_s: f64::MAX / 2.0,
            intensity: Slew::new(cfg::SLEW_INTENSITY_S, 0.0),
            speed: Slew::new(cfg::SLEW_SPEED_S, cfg::SPEED_MIN),
            octave2: Slew::new(cfg::SLEW_OCTAVE2_S, 0.0),
            w_gain: Slew::new(cfg::SLEW_W_GAIN_S, cfg::W_GAIN_SILENCE),
        }
    }

    pub fn frame(&mut self, cs: &ControlState, breathing: f64, dt: f64) -> MapOut {
        // A stalled or absent producer reads as silence (§6): decay our own
        // effective music_amount so the sculpture falls back to breathing.
        let stale = now_us().saturating_sub(cs.timestamp_us) > cfg::STALE_US;
        let target = if stale { 0.0 } else { cs.music_amount as f64 };
        let rate = if target > self.eff_music {
            dt / cfg::FADE_UP_S as f64
        } else {
            dt / cfg::FADE_DOWN_S as f64
        };
        self.eff_music += (target - self.eff_music).clamp(-rate, rate);

        // §9.1 onset accent: a decaying kick on every counted onset.
        if cs.onset_count != self.last_onset_count {
            self.last_onset_count = cs.onset_count;
            self.accent_strength = (cs.last_onset_strength as f64).min(1.0);
            self.since_onset_s = 0.0;
        }
        self.since_onset_s += dt;
        let accent = cfg::ACCENT_GAIN
            * self.accent_strength
            * (-self.since_onset_s / cfg::ACCENT_DECAY_S).exp()
            * self.eff_music;

        // §9.1 intensity: breathing baseline crossfaded with shaped energy
        let shaped = (cs.energy as f64).clamp(0.0, 1.0).powf(cfg::SHAPE_EXP);
        let musical = cfg::MUSIC_INTENSITY_FLOOR + (1.0 - cfg::MUSIC_INTENSITY_FLOOR) * shaped;
        let intensity = (mix(breathing, musical, self.eff_music) + accent).clamp(0.0, 1.0);

        // §9.2 drift speed from energy_slow — never raw energy
        let speed = cfg::SPEED_MIN
            + self.eff_music
                * (cs.energy_slow as f64).clamp(0.0, 1.0)
                * (cfg::SPEED_MAX - cfg::SPEED_MIN);

        // §9.3 turbulence: second Perlin octave from onset density
        let octave2 = cfg::OCTAVE2_MAX * self.eff_music * (cs.onset_density as f64).clamp(0.0, 1.0);

        // §9.4 warmth: centroid → W gain, drifting slowly
        let w_target = mix(
            cfg::W_GAIN_SILENCE,
            cfg::W_GAIN_MIN
                + (cs.centroid as f64).clamp(0.0, 1.0) * (cfg::W_GAIN_MAX - cfg::W_GAIN_MIN),
            self.eff_music,
        );

        MapOut {
            intensity: self.intensity.step(intensity, dt),
            speed: self.speed.step(speed, dt),
            octave2: self.octave2.step(octave2, dt),
            w_gain: self.w_gain.step(w_target, dt),
        }
    }
}

fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
