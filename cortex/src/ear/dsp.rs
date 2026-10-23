//! DSP building blocks for the audio fast path.

/// RBJ biquad, Direct Form 1 (the safe choice for streaming use).
/// One instance per filter per signal path.
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    pub fn highpass(fs: f32, f0: f32, q: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * f0 / fs;
        let (sw, cw) = (w0.sin(), w0.cos());
        let alpha = sw / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 + cw) / 2.0) / a0,
            b1: (-(1.0 + cw)) / a0,
            b2: ((1.0 + cw) / 2.0) / a0,
            a1: (-2.0 * cw) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Rectifying envelope follower with asymmetric one-pole smoothing:
/// fast attack, slow release. Runs per sample.
pub struct EnvFollower {
    attack_coef: f32,
    release_coef: f32,
    env: f32,
}

impl EnvFollower {
    /// Times in seconds at sample rate `fs`.
    pub fn new(attack_s: f32, release_s: f32, fs: f32) -> Self {
        Self {
            attack_coef: (-1.0 / (attack_s * fs)).exp(),
            release_coef: (-1.0 / (release_s * fs)).exp(),
            env: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let r = x.abs();
        let coef = if r > self.env {
            self.attack_coef
        } else {
            self.release_coef
        };
        self.env = coef * self.env + (1.0 - coef) * r;
        self.env
    }

    pub fn value(&self) -> f32 {
        self.env
    }
}

/// One-pole low-pass smoother, for block-rate or sample-rate signals.
pub struct OnePole {
    coef: f32,
    y: f32,
}

impl OnePole {
    pub fn new(tau_s: f32, rate_hz: f32) -> Self {
        Self {
            coef: (-1.0 / (tau_s * rate_hz)).exp(),
            y: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.y = self.coef * self.y + (1.0 - self.coef) * x;
        self.y
    }
}

/// Picks onsets out of an envelope: its rise from one block to the next, against an adaptive
/// threshold of that rise's own recent mean and spread, so the bar moves with the music.
pub struct OnsetDetector {
    k: f32,
    min_strength: f32,
    refractory_s: f32,
    prev: f32,
    mean: OnePole,
    sq: OnePole,
    since_s: f32,
}

impl OnsetDetector {
    /// Fires above the mean plus `k` standard deviations over the last `stats_s`, and above
    /// `min_strength` regardless, which vetoes digital silence; and no sooner than
    /// `refractory_s` after the last.
    pub fn new(stats_s: f32, k: f32, min_strength: f32, refractory_s: f32, block_rate_hz: f32) -> Self {
        Self {
            k,
            min_strength,
            refractory_s,
            prev: 0.0,
            mean: OnePole::new(stats_s, block_rate_hz),
            sq: OnePole::new(stats_s, block_rate_hz),
            since_s: f32::MAX,
        }
    }

    /// This block's onset strength — the envelope's rise relative to `reference`, the AGC's
    /// level — and whether it is an onset.
    pub fn process(&mut self, env: f32, reference: f32, dt: f32) -> (f32, bool) {
        let strength = ((env - self.prev).max(0.0) / (reference + 1e-6)).min(4.0);
        self.prev = env;
        let mean = self.mean.process(strength);
        let sq = self.sq.process(strength * strength);
        let std = (sq - mean * mean).max(0.0).sqrt();
        self.since_s = (self.since_s + dt).min(f32::MAX / 2.0);
        let fired = strength > mean + self.k * std
            && strength > self.min_strength
            && self.since_s >= self.refractory_s;
        if fired {
            self.since_s = 0.0;
        }
        (strength, fired)
    }
}

/// Running-minimum noise floor over a sliding window, implemented as
/// sub-window minima so it is O(1) per block.
/// Adaptation is frozen while in MUSIC state.
pub struct NoiseFloor {
    minima: Vec<f32>,
    idx: usize,
    current_min: f32,
    blocks_per_sub: u32,
    count: u32,
    initialized: bool,
    pub frozen: bool,
}

impl NoiseFloor {
    pub fn new(window_s: f32, subwindows: usize, block_rate_hz: f32) -> Self {
        let blocks_per_sub = ((window_s / subwindows as f32) * block_rate_hz).max(1.0) as u32;
        Self {
            minima: vec![0.0; subwindows],
            idx: 0,
            current_min: 0.0,
            blocks_per_sub,
            count: 0,
            initialized: false,
            frozen: false,
        }
    }

    pub fn update(&mut self, env: f32) {
        if !self.initialized {
            self.minima.fill(env);
            self.current_min = env;
            self.initialized = true;
            return;
        }
        if self.frozen {
            return;
        }
        self.current_min = self.current_min.min(env);
        self.count += 1;
        if self.count >= self.blocks_per_sub {
            self.minima[self.idx] = self.current_min;
            self.idx = (self.idx + 1) % self.minima.len();
            self.current_min = env;
            self.count = 0;
        }
    }

    pub fn floor(&self) -> f32 {
        self.minima
            .iter()
            .copied()
            .fold(self.current_min, f32::min)
    }
}

/// Slow loudness reference: rises quickly toward peaks, decays very slowly —
/// a streaming approximation of a high percentile.
/// Frozen whenever the state machine is not in MUSIC.
pub struct AgcRef {
    reference: f32,
    rise_coef: f32,
    fall_coef: f32,
    min_ref: f32,
    pub frozen: bool,
}

impl AgcRef {
    pub fn new(rise_s: f32, fall_s: f32, block_rate_hz: f32, min_ref: f32) -> Self {
        Self {
            reference: min_ref,
            rise_coef: (-1.0 / (rise_s * block_rate_hz)).exp(),
            fall_coef: (-1.0 / (fall_s * block_rate_hz)).exp(),
            min_ref,
            frozen: false,
        }
    }

    pub fn update(&mut self, env: f32) {
        if self.frozen {
            return;
        }
        let coef = if env > self.reference {
            self.rise_coef
        } else {
            self.fall_coef
        };
        self.reference = coef * self.reference + (1.0 - coef) * env;
        self.reference = self.reference.max(self.min_ref); // never divide by ~0
    }

    pub fn reference(&self) -> f32 {
        self.reference
    }
}
