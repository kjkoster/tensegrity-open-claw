//! The Ponytail sparkle / breathing mapping (SPARKLE.md §1–§5): pure per-frame
//! arithmetic from an `AudioFeatures` snapshot to the Ponytail's six channels —
//! intensity, R, G, B, white, gobo — each in 0..1, converted to DMX at slot-fill.
//!
//! The physical model (§1) splits the work by timescale: the LED carries the fast
//! sparkle (per-onset glints over an intensity floor, plus colour), while the slowly
//! rotating gobo is a spatial mask whose drift only reshuffles which fibres are lit —
//! onsets never touch it. White and RGB are modal: a hard cut masked to a dark frame,
//! never a crossfade (§4, §5).

use crate::audio_features::AudioFeatures;
use crate::clock::now_us;
use crate::config as cfg;
use crate::perlin::{fbm2, noise1d};

/// A glint contribution below this is "off" for the dark-frame test, and the level
/// the steady intensity must sit under, that together mask the hard RGB↔White cut
/// (SPARKLE.md §4): between glints, with the bundle near its floor, the next flash
/// simply appears in the new mode with no visible colour jump.
const GLINT_DARK_EPS: f64 = 0.02;

/// `eff_music` above which the piece counts as "in music" — gates white-mode entry
/// and forces its exit when music leaves, so white sparkle is a music-only phrase.
const MUSIC_PRESENT: f64 = 0.5;

/// Warm and cool endpoints of the centroid→hue ramp (§3.4): amber at the warm end,
/// an icy blue-white at the cool end. Display-space 0..1, scaled to a byte at fill.
const WARM_RGB: [f64; 3] = [1.00, 0.45, 0.10];
const COOL_RGB: [f64; 3] = [0.60, 0.75, 1.00];

pub struct PonytailOut {
    pub intensity: f64,
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub white: f64,
    pub gobo: f64,
}

/// Symmetric one-pole slew limiter — the last-line flicker defense (SPARKLE.md §5).
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

/// Asymmetric slew for the gobo (§3.3): ease into motion, coast slowly to rest so
/// the sparkle pattern settles after music. The slowest slew of all the channels.
struct GoboSlew {
    y: f64,
}

impl GoboSlew {
    fn new() -> Self {
        Self { y: 0.0 }
    }

    fn step(&mut self, target: f64, dt: f64) -> f64 {
        let tau = if target > self.y {
            cfg::GOBO_SLEW_UP_S
        } else {
            cfg::GOBO_SLEW_DOWN_S
        };
        self.y += (target - self.y) * (1.0 - (-dt / tau).exp());
        self.y
    }
}

#[derive(Clone, Copy, PartialEq)]
enum WhiteMode {
    Rgb,
    White,
}

pub struct PonytailMapping {
    // crossfade + glint state
    eff_music: f64,
    last_onset_count: u64,
    accent_strength: f64,
    since_onset_s: f64,
    // own breath clock (§2, §6) and colour-drift phase
    breath_s: f64,
    colour_t: f64,
    // per-fixture noise seeds, so two Ponytails never share a colour field
    seed_r: u64,
    seed_g: u64,
    seed_b: u64,
    // white-mode state machine (§4)
    white_mode: WhiteMode,
    white_dwell_s: f64,
    white_gate_phase: f64,
    white_seed: u64,
    // slews (§5): white is deliberately un-slewed (the modal switch)
    intensity: Slew,
    r: Slew,
    g: Slew,
    b: Slew,
    gobo: GoboSlew,
}

impl PonytailMapping {
    pub fn new(seed_r: u64, seed_g: u64, seed_b: u64, white_seed: u64) -> Self {
        Self {
            eff_music: 0.0,
            last_onset_count: 0,
            accent_strength: 0.0,
            since_onset_s: f64::MAX / 2.0,
            breath_s: 0.0,
            colour_t: 0.0,
            seed_r,
            seed_g,
            seed_b,
            white_mode: WhiteMode::Rgb,
            white_dwell_s: 0.0,
            white_gate_phase: 0.0,
            white_seed,
            intensity: Slew::new(cfg::SLEW_INTENSITY_S, 0.0),
            r: Slew::new(cfg::SLEW_OCTAVE2_S, 0.0),
            g: Slew::new(cfg::SLEW_OCTAVE2_S, 0.0),
            b: Slew::new(cfg::SLEW_OCTAVE2_S, 0.0),
            gobo: GoboSlew::new(),
        }
    }

    pub fn frame(&mut self, af: &AudioFeatures, dt: f64) -> PonytailOut {
        // Crossfade: a stalled or absent producer reads as silence (§5), decaying our
        // own effective music_amount so the sculpture falls back to breathing.
        let stale = now_us().saturating_sub(af.timestamp_us) > cfg::STALE_US;
        let target = if stale { 0.0 } else { af.music_amount as f64 };
        let rate = if target > self.eff_music {
            dt / cfg::FADE_UP_S as f64
        } else {
            dt / cfg::FADE_DOWN_S as f64
        };
        self.eff_music += (target - self.eff_music).clamp(-rate, rate);
        let m = self.eff_music;

        // Glint (§3.1): a flash held for the on-time floor, then an instant-attack
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

        // Intensity (§2, §3.2): silence breath crossfaded with the music glow floor,
        // each slewed; the glint is added on top un-slewed for a sharp attack.
        self.breath_s += dt;
        let phase = 2.0 * std::f64::consts::PI * self.breath_s / cfg::PONYTAIL_BREATH_PERIOD_S;
        let breath = cfg::PONYTAIL_BREATH_FLOOR
            + (cfg::PONYTAIL_BREATH_CEIL - cfg::PONYTAIL_BREATH_FLOOR) * 0.5 * (1.0 + phase.sin());
        // In music the steady field is dark (MUSIC_INTENSITY_FLOOR, default 0) and the
        // glints are the only light, flashing against black (§3.1). There is no
        // energy-driven glow: that tracked sustained loudness and kept the bundle lit
        // straight through loud passages, which read as "always on".
        let base = self.intensity.step(mix(breath, cfg::MUSIC_INTENSITY_FLOOR, m), dt);
        let intensity = (base + glint).clamp(0.0, 1.0);

        // Colour (§3.4): a slow Perlin wander in silence, crossfading to centroid
        // warmth with an onset-driven second-octave shimmer in music.
        let drift_speed = mix(cfg::PONYTAIL_SILENCE_DRIFT, music_colour_speed(af), m);
        self.colour_t += drift_speed * dt;
        let oct2 = cfg::PONYTAIL_SHIMMER_OCT2_MAX * m * (af.onset_density as f64).clamp(0.0, 1.0);
        let silence_rgb = [
            perlin_unit(self.colour_t, self.seed_r, oct2),
            perlin_unit(self.colour_t, self.seed_g, oct2),
            perlin_unit(self.colour_t, self.seed_b, oct2),
        ];
        let warmth = ((af.centroid as f64 - cfg::PONYTAIL_HUE_WARM_CENTROID)
            / (cfg::PONYTAIL_HUE_COOL_CENTROID - cfg::PONYTAIL_HUE_WARM_CENTROID))
            .clamp(0.0, 1.0);
        let music_rgb = warmth_to_rgb(warmth, self.colour_t, self.seed_r, oct2);
        let r = self.r.step(mix(silence_rgb[0], music_rgb[0], m), dt);
        let g = self.g.step(mix(silence_rgb[1], music_rgb[1], m), dt);
        let b = self.b.step(mix(silence_rgb[2], music_rgb[2], m), dt);

        // Gobo (§3.3): slow reshuffle inside a slow band, nudged by energy_slow blended
        // with onset_density (optional tempo scaling, gated on confidence), eased into
        // motion by music, then asym-slewed. Onsets never touch this channel.
        let busy = 0.5 * (af.energy_slow as f64).clamp(0.0, 1.0)
            + 0.5 * (af.onset_density as f64).clamp(0.0, 1.0);
        let mut drift =
            cfg::GOBO_DRIFT_MUSIC_MIN + (cfg::GOBO_DRIFT_MAX - cfg::GOBO_DRIFT_MUSIC_MIN) * busy;
        if (af.tempo_confidence as f64) > cfg::GOBO_TEMPO_CONF_GATE {
            let tempo_norm = ((af.bpm as f64 - cfg::BPM_MIN as f64)
                / (cfg::BPM_MAX as f64 - cfg::BPM_MIN as f64))
                .clamp(0.0, 1.0);
            drift += cfg::GOBO_TEMPO_FACTOR * tempo_norm * (cfg::GOBO_DRIFT_MAX - drift);
        }
        let gobo = self.gobo.step(mix(0.0, drift, m), dt);

        // White-sparkle mode (§4): a slow Perlin gate makes a window eligible; a
        // sustained-density musical event commits the entry; both the entry and the
        // exit flip only on a dark frame so the hard cut is invisible.
        self.white_gate_phase += cfg::WHITE_MODE_PERLIN_SPEED * dt;
        let eligible =
            (0.5 + noise1d(self.white_gate_phase, self.white_seed)) > cfg::WHITE_MODE_GATE;
        let committed = (af.onset_density as f64) > cfg::WHITE_MODE_COMMIT_DENSITY && m > MUSIC_PRESENT;
        let dark =
            glint < GLINT_DARK_EPS && intensity < cfg::MUSIC_INTENSITY_FLOOR + GLINT_DARK_EPS;
        match self.white_mode {
            WhiteMode::Rgb => {
                if eligible && committed && dark {
                    self.white_mode = WhiteMode::White;
                    self.white_dwell_s = 0.0;
                }
            }
            WhiteMode::White => {
                self.white_dwell_s += dt;
                let want_exit = self.white_dwell_s >= cfg::WHITE_MODE_MAX_DWELL_S
                    || !eligible
                    || m < MUSIC_PRESENT;
                if self.white_dwell_s >= cfg::WHITE_MODE_MIN_DWELL_S && want_exit && dark {
                    self.white_mode = WhiteMode::Rgb;
                }
            }
        }

        // White is modal: in white mode the LED is white and the RGB emitters are dark.
        match self.white_mode {
            WhiteMode::White => PonytailOut {
                intensity,
                r: 0.0,
                g: 0.0,
                b: 0.0,
                white: 1.0,
                gobo,
            },
            WhiteMode::Rgb => PonytailOut {
                intensity,
                r,
                g,
                b,
                white: 0.0,
                gobo,
            },
        }
    }
}

fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Music colour-drift speed: reuse the wash's energy_slow→speed band so the field
/// wanders (and the shimmer phase advances) faster in louder passages.
fn music_colour_speed(af: &AudioFeatures) -> f64 {
    cfg::SPEED_MIN + (af.energy_slow as f64).clamp(0.0, 1.0) * (cfg::SPEED_MAX - cfg::SPEED_MIN)
}

/// One Perlin colour channel mapped to a perceptual 0..1, via the same contrast /
/// gamma curve as `perlin::to_dmx` once was, but kept as a float so it can be blended
/// and slewed before the slot-fill conversion to a DMX byte.
fn perlin_unit(t: f64, seed: u64, oct2: f64) -> f64 {
    (fbm2(t, seed, oct2) * cfg::CONTRAST + 0.5)
        .clamp(0.0, 1.0)
        .powf(1.0 / cfg::GAMMA)
}

/// Centroid warmth → RGB along the amber↔blue-white ramp, with a small onset-driven
/// Perlin shimmer (`oct2`) riding on top so busy music roils and calm music stays
/// smooth (§3.4).
fn warmth_to_rgb(warmth: f64, t: f64, seed: u64, oct2: f64) -> [f64; 3] {
    let shimmer = oct2 * noise1d(t * 2.0, seed ^ 0x9e37_79b9_7f4a_7c15);
    [
        (mix(WARM_RGB[0], COOL_RGB[0], warmth) + shimmer).clamp(0.0, 1.0),
        (mix(WARM_RGB[1], COOL_RGB[1], warmth) + shimmer).clamp(0.0, 1.0),
        (mix(WARM_RGB[2], COOL_RGB[2], warmth) + shimmer).clamp(0.0, 1.0),
    ]
}
