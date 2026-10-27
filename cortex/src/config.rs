//! All tunables in one place. The festival tuning loop is "edit numbers, rebuild, restart".
//!
//! These describe the *cabinet* rather than the sculpture hanging off it: the Pi, the RS-485
//! HAT, the serial device, the broker.

use std::net::Ipv4Addr;

// ── Deployment (DMX / sACN) ──────────────────────────────────────────────────
// `FRAME_RATE_HZ`, in a file of its own because the rig crates' build scripts need it too.
include!("../frame_rate.rs");

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
// `health/`. The broker runs on memory-pi, beside the recordings, at the fixed address it sets
// itself on this Pi's access point, so the Pi that drives the show carries none of the
// telemetry. The port and the keepalive are the rig's, in the `stem` crate.
pub const MQTT_ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 0, 10, 2);

// How long a connection attempt may take before it counts as failed and backs off. The broker
// is across the WiFi, and a memory-pi that is off would otherwise leave the attempt to the
// kernel's own SYN retries, the better part of two minutes.
pub const MQTT_CONNECT_TIMEOUT_S: u64 = 5;

// How long to wait for the CONNACK before treating the broker as unusable. Generous: the only
// things that make it slow are a broker still starting up and a WiFi link catching up.
pub const MQTT_CONNACK_TIMEOUT_S: u64 = 5;

// How long a write may block before the broker counts as gone. Writes land in the socket
// buffer and return at once while the broker keeps up; one that blocks this long is writing to
// a peer that is no longer reading.
pub const MQTT_WRITE_TIMEOUT_S: u64 = 5;

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
