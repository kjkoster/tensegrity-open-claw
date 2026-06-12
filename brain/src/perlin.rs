fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn gradient(cell: i64, seed: u64) -> f64 {
    if splitmix64(cell as u64 ^ seed) & 1 == 0 {
        1.0
    } else {
        -1.0
    }
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// 1-D gradient Perlin noise using splitmix64 instead of a permutation table.
/// Returns a value in roughly [−0.5, 0.5].
pub fn noise1d(t: f64, seed: u64) -> f64 {
    let i = t.floor() as i64;
    let f = t - t.floor();
    let u = fade(f);
    let a = gradient(i, seed) * f;
    let b = gradient(i + 1, seed) * (f - 1.0);
    a + u * (b - a)
}

/// Two-octave fBm: the base field plus a second octave at double frequency
/// whose amplitude (0..~0.5) is the turbulence driver. Calm music keeps the
/// field smooth; busy music makes it roil.
pub fn fbm2(t: f64, seed: u64, octave2_amp: f64) -> f64 {
    noise1d(t, seed) + octave2_amp * noise1d(t * 2.0, seed ^ 0x9e37_79b9_7f4a_7c15)
}

/// Maps a noise value to a DMX byte via contrast gain, clamp, gamma, and per-channel gain.
pub fn to_dmx(n: f64, contrast: f64, gamma: f64, gain: f64) -> u8 {
    let v = (n * contrast + 0.5).clamp(0.0, 1.0).powf(1.0 / gamma) * gain;
    (v * 255.0) as u8
}
