//! All tunables in one place. The festival tuning loop is "edit numbers, rebuild, restart".
//!
//! These describe the *cabinet* rather than the sculpture hanging off it: the Pi, the RS-485
//! HAT, the audio interface, the serial device.

use std::net::Ipv4Addr;

// ── Deployment (DMX / sACN) ──────────────────────────────────────────────────
// Fixed 40 Hz, just under the ~43 Hz wire ceiling of a full 512-slot wired universe.
pub const FRAME_RATE_HZ: u64 = 40;

// E1.31 source priority — one number with two jobs, deliberately not split. It is the
// priority the brain transmits at, and it is the bar an external console must clear to take
// the universe: takeover requires STRICTLY greater (see sacn_in.rs). Strict is load-bearing.
// Every sACN source ships defaulted to 100, so any laptop that joins the network with a live
// universe would otherwise seize the rig; taking over has to be a deliberate act. The QLC+
// workspace is configured at 200. Equal priority is not a tie — the brain keeps driving.
// Two constants here would drift apart and break that silently.
pub const SACN_PRIORITY: u8 = 100;
// Stream-terminated frames emitted on shutdown. UDP is lossy, so repeat: any one
// arriving makes the fixtures drop the brain's source at once instead of waiting out
// the 2.5 s data-loss timeout.
pub const SACN_RELEASE_FRAMES: u8 = 3;

// ── E1.31 receive / external takeover ────────────────────────────────────────
// The E1.31 network-data-loss timeout: an external source that goes quiet this long loses
// the universe and the internal engine resumes. This is the backstop only — a clean QLC+
// shutdown sets the stream-terminated bit and hands back inside one frame.
pub const SACN_SOURCE_TIMEOUT_US: u64 = 2_500_000;

// Distinct senders tracked at once. Normally one console; a few more absorbs strays without
// unbounded storage, and an expiring entry frees a slot within the timeout above.
pub const SACN_MAX_SOURCES: usize = 4;

// Rebind backoff cap for the receive socket. Retrying rather than dying means the takeover
// path comes back the moment whatever held the port lets go, and the sculpture keeps
// breathing on the internal engine meanwhile.
pub const SACN_BIND_RETRY_MAX_S: u64 = 30;

// Fixture addressing, the universe, the sACN port and the address a console is received on
// are not here: they are generated from the rig's QLC+ workspace into the rig's own `patch`
// module, the single source of truth for what is patched where.

// ── Telemetry (the broker) ───────────────────────────────────────────────────
// Stem owns the broker; every daemon holds its own client and announces itself under
// `health/`. Loopback, because the broker is on this Pi — what reaches the tunnel and the AP
// is the broker's own listener, which is Stem's configuration to make, not this process's to
// decide. The port and the keepalive are the rig's, in `rig_mqtt`.
pub const MQTT_ADDRESS: Ipv4Addr = Ipv4Addr::LOCALHOST;

// How long to wait for the CONNACK before treating the broker as unusable. Generous: it is a
// loopback handshake, and the only thing that makes it slow is a broker still starting up.
pub const MQTT_CONNACK_TIMEOUT_S: u64 = 5;

// Messages that may sit queued for the telemetry thread. Deliberately shallow — the queue
// exists so the frame loop never waits on a socket, not so telemetry survives an outage, and
// past this depth the newest reading is worth more than the backlog behind it.
pub const MQTT_QUEUE_DEPTH: usize = 64;

// Reconnect backoff, matching the other receivers': a broker that comes back is picked up
// without anything being restarted.
pub const MQTT_RETRY_MAX_S: u64 = 30;

// How long a shutdown waits for the goodbye to actually leave. The alternative to waiting is
// the will firing and an orderly stop being recorded as a crash, so it is worth a moment — but
// only a moment, because a wedged broker must never be able to hold up a shutdown.
pub const MQTT_FAREWELL_MS: u64 = 500;

// ── Wired DMX-512 output (Zihatec RS-485 HAT) ─────────────────────────────────
// The HAT is clocked with the same slot buffer as the sACN stream, so the wired universe
// mirrors the wireless one — one universe, two transports. Framing (BREAK/MAB timing,
// full-512 padding) lives in the zihatec-rs-485-dmx crate; only the device path is ours.
// Always open the /dev/serial0 alias, never a bare ttyAMA*/ttyS0: it stays stable while
// USB serial adapters renumber around it, and resolves to the PL011 once Bluetooth is
// disabled.
pub const SERIAL_DEVICE: &str = "/dev/serial0";

// ── Perceived brightness ─────────────────────────────────────────────────────
pub const GAMMA: f64 = 2.2;

// ── Ear — capture ────────────────────────────────────────────────────────────
// The ear's interface is whichever class-compliant USB audio interface is in the cabinet —
// today an Alesis io|2, chosen as the cheapest used one available, not for anything it does.
// Swapping it is these three lines: its ALSA card name, and its USB identity for the probe.
pub const ALSA_DEVICE: &str = "plughw:CARD=io2,DEV=0"; // confirm with `arecord -L`
pub const EAR_USB_VENDOR: u16 = 0x13b2; // confirm with `lsusb`
pub const EAR_USB_PRODUCT: u16 = 0x0008;
pub const REQUESTED_RATE_HZ: u32 = 48_000; // always use the negotiated rate, never this
pub const CHANNELS: usize = 2;
pub const PERIOD_FRAMES: usize = 512; // ≈10.7 ms @ 48 kHz; do not chase smaller
pub const PERIODS_PER_BUFFER: usize = 4;
pub const DEVICE_RETRY_MAX_S: u64 = 30;

// ── Ear — high-pass filter ───────────────────────────────────────────────────
pub const HPF_HZ: f32 = 90.0;
pub const FILTER_Q: f32 = 0.707;

// ── Ear — envelope followers ─────────────────────────────────────────────────
pub const ENV_ATTACK_S: f32 = 0.010; // ≤ 15 ms keeps lights tight on hits
pub const ENV_RELEASE_S: f32 = 0.200;
pub const ONSET_ENV_ATTACK_S: f32 = 0.001;
pub const ONSET_ENV_RELEASE_S: f32 = 0.050;

// ── Ear — noise-floor tracking ───────────────────────────────────────────────
pub const FLOOR_WINDOW_S: f32 = 8.0;
pub const FLOOR_SUBWINDOWS: usize = 16;
pub const FLOOR_MARGIN: f32 = 1.75;

// ── Ear — AGC ────────────────────────────────────────────────────────────────
pub const AGC_RISE_S: f32 = 1.0;
pub const AGC_FALL_S: f32 = 30.0; // must stay slow or the AGC becomes a compressor
pub const AGC_MIN_REF: f32 = 1e-3;
pub const NORM_HEADROOM: f32 = 1.5; // genuine peaks may read > 1.0

// ── Ear — onsets ─────────────────────────────────────────────────────────────
pub const ONSET_STATS_S: f32 = 1.5; // history window for the adaptive threshold
pub const ONSET_K: f32 = 2.0; // fire above mean + k·std
pub const ONSET_MIN_STRENGTH: f32 = 0.02; // absolute veto against digital silence
pub const ONSET_REFRACTORY_S: f32 = 0.09;

// ── Ear — published features ─────────────────────────────────────────────────
pub const STALE_US: u64 = 250_000; // older snapshots read as silence

// ── Ear — silence ↔ music state machine ──────────────────────────────────────
pub const UP_THRESHOLD: f32 = 0.15;
pub const DOWN_THRESHOLD: f32 = 0.05; // must be < UP_THRESHOLD
pub const UP_HOLD_S: f32 = 0.5;
pub const DOWN_HOLD_S: f32 = 6.0; // a quiet bridge must not flip the state
pub const FADE_UP_S: f32 = 1.5;
pub const FADE_DOWN_S: f32 = 3.0;

// ── Mapping layer (DMX side) ─────────────────────────────────────────────────
pub const MUSIC_INTENSITY_FLOOR: f64 = 0.0; // steady field in music; 0 = black, glints are all the light
pub const SLEW_INTENSITY_S: f64 = 0.03;

// ── Sparkle — silence breathing ──────────────────────────────────────────────
pub const SPARKLE_BREATH_PERIOD_S: f64 = 18.0; // own slow breath, far longer than the PWM 3.5 s
pub const SPARKLE_BREATH_FLOOR: f64 = 0.08; // never fully dark in silence
pub const SPARKLE_BREATH_CEIL: f64 = 0.60; // gentle ceiling; calm, not bright

// ── Sparkle — glints (LED flashes; the sparkle) ──────────────────────────────
pub const SPARKLE_FLASH_FRAMES: u32 = 1; // on-time floor in frames (sub-frame impossible)
pub const SPARKLE_ACCENT_GAIN: f64 = 0.85; // glint brightness pop per onset
pub const SPARKLE_AFTERGLOW_DECAY_S: f64 = 0.025; // ~25 ms tail (≈1 frame); lower → sharper on-off / square strobe

// ── Observability ────────────────────────────────────────────────────────────
pub const STATUS_INTERVAL_S: f32 = 5.0;
