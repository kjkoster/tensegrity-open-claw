//! Orchestrator stage (SPARKLE.md §0.3): the per-frame DMX loop (`noise_task`)
//! and the mapping layer it drives. The mapping (SOUND.md §9) is pure per-frame
//! arithmetic from an `AudioFeatures` snapshot to Perlin/intensity parameters.
//! The breathing baseline is always present underneath; audio modulation is scaled
//! by music_amount. Every output passes through a per-parameter slew limiter as
//! the last defense against flicker.

use crate::audio_features::AudioFeatures;
use crate::clock::now_us;
use crate::config as cfg;
use crate::dmx;
use crate::fixture::Fixture;
use crate::latest::LatestRx;
use crate::perlin::{fbm2, to_dmx};
use embassy_time::{Duration, Ticker};
use std::net::UdpSocket;

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

impl Default for Mapping {
    fn default() -> Self {
        Self::new()
    }
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

    pub fn frame(&mut self, cs: &AudioFeatures, breathing: f64, dt: f64) -> MapOut {
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

/// The per-frame DMX loop: read the latest `AudioFeatures`, map it, fill the slot
/// array for both Ponytail fixtures, and emit one sACN packet at the frame rate.
#[embassy_executor::task]
pub async fn noise_task(socket: UdpSocket, cid: [u8; 16], features: LatestRx<AudioFeatures>) -> ! {
    let ponytail_a = Fixture { start_address: 1 };
    let ponytail_b = Fixture { start_address: 7 };
    let frame_period = Duration::from_micros(1_000_000 / cfg::FRAME_RATE_HZ);
    let mut ticker = Ticker::every(frame_period);
    let mut sequence: u8 = 0;
    let start = std::time::Instant::now();
    let dt = 1.0 / cfg::FRAME_RATE_HZ as f64;
    let mut mapping = Mapping::new();
    // Drift speed varies per frame, so the noise coordinate is an accumulated
    // phase rather than elapsed × speed — otherwise speed changes jump the field.
    let mut t = 0.0_f64;

    loop {
        ticker.next().await;
        let snapshot = features.snapshot();
        let elapsed = start.elapsed().as_secs_f64();

        // Intensity baseline: slow sine breathing between floor and ceiling
        let phase = 2.0 * std::f64::consts::PI * elapsed / cfg::I_SILENCE_PERIOD_S;
        let breathing = cfg::I_SILENCE_FLOOR
            + (cfg::I_SILENCE_CEIL - cfg::I_SILENCE_FLOOR) * 0.5 * (1.0 + phase.sin());

        let out = mapping.frame(&snapshot, breathing, dt);
        t += out.speed * dt;
        // Gamma-correct the dimmer like the colour channels: it lands on the fixture's
        // linear ~100-level native brightness scale, so spending those few levels
        // perceptually (rather than linearly, bunched at the dark end) is what keeps the
        // breathing from banding. Round, not truncate, to match the fixture-side scaling.
        let intensity = (out.intensity.powf(1.0 / cfg::GAMMA) * 255.0 + 0.5) as u8;

        // 12 DMX slots for two 6-channel Ponytail fixtures (IRGBW + Gobo rotation).
        let mut slots = [0u8; 12];

        slots[ponytail_a.slot(0)] = intensity;
        slots[ponytail_a.slot(1)] = to_dmx(fbm2(t, cfg::SEEDS[0], out.octave2), cfg::CONTRAST, cfg::GAMMA, 1.0);
        slots[ponytail_a.slot(2)] = to_dmx(fbm2(t, cfg::SEEDS[1], out.octave2), cfg::CONTRAST, cfg::GAMMA, 1.0);
        slots[ponytail_a.slot(3)] = to_dmx(fbm2(t, cfg::SEEDS[2], out.octave2), cfg::CONTRAST, cfg::GAMMA, 1.0);
        // White (offset 4) is parked at 0: White > 0 hard-cuts RGB on these fixtures.
        slots[ponytail_a.slot(4)] = 0;
        // Gobo rotation (offset 5) is 0 (motor off). Only the BLE bridge personality
        // drives the gobo; the PWM fixtures ignore this slot.
        slots[ponytail_a.slot(5)] = 0;

        slots[ponytail_b.slot(0)] = intensity;
        slots[ponytail_b.slot(1)] = to_dmx(fbm2(t, cfg::SEEDS[4], out.octave2), cfg::CONTRAST, cfg::GAMMA, 1.0);
        slots[ponytail_b.slot(2)] = to_dmx(fbm2(t, cfg::SEEDS[5], out.octave2), cfg::CONTRAST, cfg::GAMMA, 1.0);
        slots[ponytail_b.slot(3)] = to_dmx(fbm2(t, cfg::SEEDS[6], out.octave2), cfg::CONTRAST, cfg::GAMMA, 1.0);
        slots[ponytail_b.slot(4)] = 0;
        slots[ponytail_b.slot(5)] = 0;

        let packet = dmx::encode(cfg::UNIVERSE, sequence, 100, &cid, &slots);
        dmx::send_multicast(&socket, cfg::UNIVERSE, cfg::SACN_PORT, &packet);
        sequence = sequence.wrapping_add(1);
    }
}
