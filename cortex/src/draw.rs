//! Random draws for decisions taken once, rather than fields sampled every frame.
//!
//! A draw is a pure function of its key: the same seed and the same key components give the
//! same values, in the same order, every time they are asked for. So whoever draws stores
//! nothing — a schedule is recomputed from its key each frame and comes out the same — and a
//! restart reproduces every decision made before it.

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// The values one key gives, in order.
pub struct Draw {
    state: u64,
}

impl Draw {
    /// The draws for `key` under `seed`. Each component of the key is folded in turn, so
    /// `[stretch, 3]` and `[stretch, 4]` are unrelated streams.
    pub fn new(seed: u64, key: &[i64]) -> Self {
        let state = key
            .iter()
            .fold(splitmix64(seed), |state, part| splitmix64(state ^ *part as u64));
        Self { state }
    }

    /// A value uniform over [0, 1).
    pub fn unit(&mut self) -> f64 {
        // SplitMix64's own stream: a counter stepped by its golden-ratio increment, each step
        // mixed. `splitmix64` adds the increment itself, so this is the counter's next value.
        let bits = splitmix64(self.state);
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        // The top 53 bits, which is every bit an f64 in [0, 1) can hold.
        (bits >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A value uniform over [`low`, `high`).
    pub fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    /// An index uniform over `0..len`. `len` must not be zero.
    pub fn index(&mut self, len: usize) -> usize {
        ((self.unit() * len as f64) as usize).min(len - 1)
    }
}
