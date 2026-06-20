//! All tunables in one place, grouped by the section numbers of SOUND.md.
//! The festival tuning loop is "edit numbers, rebuild, restart".

// ── Deployment (DMX / sACN) ──────────────────────────────────────────────────
pub const UNIVERSE: u16 = 1;
pub const SACN_PORT: u16 = 5568;
pub const FRAME_RATE_HZ: u64 = 44;

// ── Noise engine ─────────────────────────────────────────────────────────────
pub const CONTRAST: f64 = 1.6;
pub const GAMMA: f64 = 2.2;

// Distinct 64-bit seeds give independent noise fields sharing one drift speed.
// Order: Fixture A [R, G, B, W], Fixture B [R, G, B, W]
pub const SEEDS: [u64; 8] = [
    0xcafe_babe_dead_beef,
    0x1234_5678_9abc_def0,
    0xfedc_ba98_7654_3210,
    0xa5a5_a5a5_5a5a_5a5a,
    0x0f0f_0f0f_f0f0_f0f0,
    0x5555_aaaa_5555_aaaa,
    0x3c3c_3c3c_c3c3_c3c3,
    0x6969_6969_9696_9696,
];

// ── §3 Capture ───────────────────────────────────────────────────────────────
pub const ALSA_DEVICE: &str = "plughw:CARD=io2,DEV=0"; // confirm with `arecord -L`
pub const REQUESTED_RATE_HZ: u32 = 48_000; // always use the negotiated rate, never this
pub const CHANNELS: usize = 2;
pub const PERIOD_FRAMES: usize = 512; // ≈10.7 ms @ 48 kHz; do not chase smaller
pub const PERIODS_PER_BUFFER: usize = 4;
pub const DEVICE_RETRY_MAX_S: u64 = 30;

// ── §4.2 High-pass filter ────────────────────────────────────────────────────
pub const HPF_HZ: f32 = 90.0;
pub const FILTER_Q: f32 = 0.707;

// ── §4.3 Band split ──────────────────────────────────────────────────────────
pub const BAND_LOW_MAX_HZ: f32 = 250.0;
pub const BAND_MID_MAX_HZ: f32 = 4_000.0;

// ── §4.4 Envelope followers ──────────────────────────────────────────────────
pub const ENV_ATTACK_S: f32 = 0.010; // ≤ 15 ms keeps lights tight on hits
pub const ENV_RELEASE_S: f32 = 0.200;
pub const ONSET_ENV_ATTACK_S: f32 = 0.001;
pub const ONSET_ENV_RELEASE_S: f32 = 0.050;

// ── §4.5 Noise-floor tracking ────────────────────────────────────────────────
pub const FLOOR_WINDOW_S: f32 = 8.0;
pub const FLOOR_SUBWINDOWS: usize = 16;
pub const FLOOR_MARGIN: f32 = 1.75;

// ── §4.6 AGC ─────────────────────────────────────────────────────────────────
pub const AGC_RISE_S: f32 = 1.0;
pub const AGC_FALL_S: f32 = 30.0; // must stay slow or the AGC becomes a compressor
pub const AGC_MIN_REF: f32 = 1e-3;
pub const NORM_HEADROOM: f32 = 1.5; // genuine peaks may read > 1.0

// ── §5.1 Energy family ───────────────────────────────────────────────────────
pub const ENERGY_SLOW_S: f32 = 1.5;
pub const BASS_RATIO_SMOOTH_S: f32 = 0.5;
pub const TILT_SMOOTH_S: f32 = 0.5;

// ── §5.2 Dynamics family ─────────────────────────────────────────────────────
pub const CREST_SMOOTH_S: f32 = 1.0;
pub const RMS_VAR_WINDOW_S: f32 = 2.0;

// ── §5.3 Onsets ──────────────────────────────────────────────────────────────
pub const ONSET_STATS_S: f32 = 1.5; // history window for the adaptive threshold
pub const ONSET_K: f32 = 2.0; // fire above mean + k·std
pub const ONSET_MIN_STRENGTH: f32 = 0.02; // absolute veto against digital silence
pub const ONSET_REFRACTORY_S: f32 = 0.09;
pub const ONSET_DENSITY_WINDOW_S: f32 = 3.0;
pub const ONSET_DENSITY_FULL_HZ: f32 = 8.0; // onsets/s that read as density 1.0

// ── §5.4 Spectral (slow path) ────────────────────────────────────────────────
pub const FFT_SIZE: usize = 2048;
pub const FFT_INTERVAL_S: f32 = 0.25;
pub const CENTROID_MIN_HZ: f32 = 100.0; // log-mapping range for centroid/rolloff
pub const CENTROID_MAX_HZ: f32 = 8_000.0;
pub const SPREAD_NORM_HZ: f32 = 4_000.0;
pub const ROLLOFF_FRACTION: f32 = 0.85;

// ── §5.5 Tempo ───────────────────────────────────────────────────────────────
pub const TEMPO_RING_S: f32 = 6.0;
pub const TEMPO_MIN_RING_S: f32 = 3.0; // no estimates until this much history
pub const BPM_MIN: u32 = 60;
pub const BPM_MAX: u32 = 180;
pub const BPM_PREF_MIN: f32 = 90.0; // octave-error correction prefers this range
pub const BPM_PREF_MAX: f32 = 150.0;
pub const BPM_PREF_BIAS: f32 = 1.15;
pub const TEMPO_PERSIST_S: f32 = 2.0; // hysteresis before accepting a jump

// ── §5.6 Long horizon ────────────────────────────────────────────────────────
pub const ENERGY_3MIN_S: f32 = 180.0;

// ── §6 Published contract ────────────────────────────────────────────────────
pub const STALE_US: u64 = 250_000; // older snapshots read as silence

// ── §7 Silence ↔ music state machine ─────────────────────────────────────────
pub const UP_THRESHOLD: f32 = 0.15;
pub const DOWN_THRESHOLD: f32 = 0.05; // must be < UP_THRESHOLD
pub const UP_HOLD_S: f32 = 0.5;
pub const DOWN_HOLD_S: f32 = 6.0; // a quiet bridge must not flip the state
pub const FADE_UP_S: f32 = 1.5;
pub const FADE_DOWN_S: f32 = 3.0;

// ── §9 Mapping layer (DMX side) ──────────────────────────────────────────────
pub const MUSIC_INTENSITY_FLOOR: f64 = 0.0; // steady field in music; 0 = black, glints are all the light
pub const SPEED_MIN: f64 = 1.5; // music colour-drift speed band, cells per second
pub const SPEED_MAX: f64 = 7.5;
pub const SLEW_INTENSITY_S: f64 = 0.03;
pub const SLEW_OCTAVE2_S: f64 = 0.3; // also the Ponytail RGB colour-channel slew

// ── §Ponytail — silence breathing (SPARKLE.md §2) ────────────────────────────
pub const PONYTAIL_BREATH_PERIOD_S: f64 = 18.0; // own slow breath, far longer than the PWM 3.5 s
pub const PONYTAIL_BREATH_FLOOR: f64 = 0.08; // never fully dark in silence
pub const PONYTAIL_BREATH_CEIL: f64 = 0.60; // gentle ceiling; calm, not bright
pub const PONYTAIL_SILENCE_DRIFT: f64 = 0.30; // RGB Perlin drift speed, ≪ SPEED_MIN

// ── §Ponytail — gobo (slow spatial reshuffle, NOT twinkle rate; SPARKLE.md §3.3) ─
pub const GOBO_DRIFT_MUSIC_MIN: f64 = 0.30; // floor: strip always moving during music (~speed 4/10)
pub const GOBO_DRIFT_MAX: f64 = 1.00; // ceiling: full motor speed (10/10) at the busiest
pub const GOBO_SLEW_UP_S: f64 = 0.80;
pub const GOBO_SLEW_DOWN_S: f64 = 4.00; // slower → pattern settles after music
pub const GOBO_TEMPO_FACTOR: f64 = 0.30; // weight of bpm coupling (0 = disable)
pub const GOBO_TEMPO_CONF_GATE: f64 = 0.50;

// ── §Ponytail — glints (LED flashes; the sparkle; SPARKLE.md §3.1–§3.2) ───────
pub const SPARKLE_FLASH_FRAMES: u32 = 1; // on-time floor in frames (sub-frame impossible)
pub const SPARKLE_ACCENT_GAIN: f64 = 0.85; // glint brightness pop per onset
pub const SPARKLE_AFTERGLOW_DECAY_S: f64 = 0.025; // ~25 ms tail (≈1 frame); lower → sharper on-off / square strobe

// ── §Ponytail — colour (RGB; SPARKLE.md §3.4) ────────────────────────────────
pub const PONYTAIL_HUE_WARM_CENTROID: f64 = 0.00; // centroid → warm end (red/amber)
pub const PONYTAIL_HUE_COOL_CENTROID: f64 = 1.00; // centroid → cool end (blue/white-ish)
pub const PONYTAIL_SHIMMER_OCT2_MAX: f64 = 0.50; // onset_density → second-octave turbulence

// ── §Ponytail — white-sparkle mode (modal hard switch; SPARKLE.md §4) ─────────
pub const WHITE_MODE_PERLIN_SEED: u64 = 0x5eed_dead_beef_cafe; // own seed; slow gate
pub const WHITE_MODE_PERLIN_SPEED: f64 = 0.02; // very slow → sparse, aperiodic eligibility
pub const WHITE_MODE_GATE: f64 = 0.35; // Perlin threshold for an eligible window
pub const WHITE_MODE_COMMIT_DENSITY: f64 = 0.70; // musical event required to actually enter
pub const WHITE_MODE_MIN_DWELL_S: f64 = 4.0;
pub const WHITE_MODE_MAX_DWELL_S: f64 = 25.0; // exit on next dark frame after dwell

// ── §10 Observability ────────────────────────────────────────────────────────
pub const STATUS_INTERVAL_S: f32 = 5.0;

// ── §14 Sound-profile recorder ───────────────────────────────────────────────
pub const RECORDER_DIR: &str = "/home/kjkoster/ear";
pub const RECORDER_RATE_HZ: u64 = 10;
pub const RECORDER_ROTATE_S: u64 = 600;
pub const RECORDER_BATCH_ROWS: usize = 100; // one row group + flush per ~10 s
pub const RECORDER_RETRY_S: u64 = 10;
