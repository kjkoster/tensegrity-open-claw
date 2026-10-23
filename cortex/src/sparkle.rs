//! The sparkle / breathing mapping: pure per-frame arithmetic from an `AudioFeatures`
//! snapshot to one intensity in 0..1.
//!
//! In silence the rig breathes on its own slow clock. In music the steady field goes dark and
//! each onset fires a glint: a flash with a short afterglow, flashing against black. The two
//! crossfade on the ear's own sense of how much music there is.

use crate::ear::AudioFeatures;
use crate::config as cfg;

/// Symmetric one-pole slew limiter — the last-line flicker defense.
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

pub struct SparkleMapping {
    // crossfade + glint state
    eff_music: f64,
    last_onset_count: u64,
    accent_strength: f64,
    since_onset_s: f64,
    // own breath clock
    breath_s: f64,
    intensity: Slew,
}

impl SparkleMapping {
    pub fn new() -> Self {
        Self {
            eff_music: 0.0,
            last_onset_count: 0,
            accent_strength: 0.0,
            since_onset_s: f64::MAX / 2.0,
            breath_s: 0.0,
            intensity: Slew::new(cfg::SLEW_INTENSITY_S, 0.0),
        }
    }

    /// This frame's intensity, 0..1, `dt` seconds after the last.
    pub fn frame(&mut self, af: &AudioFeatures, dt: f64) -> f64 {
        // Crossfade: a stalled or absent producer reads as silence, decaying our
        // own effective music_amount so the sculpture falls back to breathing.
        let target = if af.is_stale() { 0.0 } else { af.music_amount as f64 };
        let rate = if target > self.eff_music {
            dt / cfg::FADE_UP_S as f64
        } else {
            dt / cfg::FADE_DOWN_S as f64
        };
        self.eff_music += (target - self.eff_music).clamp(-rate, rate);
        let m = self.eff_music;

        // Glint: a flash held for the on-time floor, then an instant-attack
        // exponential afterglow. The brevity lives in the decay because a sub-frame
        // on-time is not commandable at the frame rate.
        if af.onset_count != self.last_onset_count {
            self.last_onset_count = af.onset_count;
            self.accent_strength = (af.last_onset_strength as f64).min(1.0);
            self.since_onset_s = 0.0;
        }
        self.since_onset_s += dt;
        let flash_hold_s = cfg::SPARKLE_FLASH_FRAMES as f64 * dt;
        let env = if self.since_onset_s <= flash_hold_s {
            1.0
        } else {
            (-(self.since_onset_s - flash_hold_s) / cfg::SPARKLE_AFTERGLOW_DECAY_S).exp()
        };
        let glint = cfg::SPARKLE_ACCENT_GAIN * self.accent_strength * env * m;

        // Intensity: silence breath crossfaded with the music floor, slewed; the glint is
        // added on top un-slewed for a sharp attack.
        self.breath_s += dt;
        let phase = 2.0 * std::f64::consts::PI * self.breath_s / cfg::SPARKLE_BREATH_PERIOD_S;
        let breath = cfg::SPARKLE_BREATH_FLOOR
            + (cfg::SPARKLE_BREATH_CEIL - cfg::SPARKLE_BREATH_FLOOR) * 0.5 * (1.0 + phase.sin());
        // In music the steady field is dark (MUSIC_INTENSITY_FLOOR, default 0) and the
        // glints are the only light, flashing against black. There is no
        // energy-driven glow: that tracked sustained loudness and kept the bundle lit
        // straight through loud passages, which read as "always on".
        let base = self.intensity.step(mix(breath, cfg::MUSIC_INTENSITY_FLOOR, m), dt);
        (base + glint).clamp(0.0, 1.0)
    }
}

fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
