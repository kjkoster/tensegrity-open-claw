//! All tunables in one place. The festival tuning loop is "edit numbers, rebuild, restart".
//!
//! These describe the *cabinet* rather than the sculpture hanging off it: the Pi, the RS-485
//! HAT, the serial device, the broker. No addresses: those are the rig's, generated from its
//! `clockwork.dot`.

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

// ── Cores and priorities ─────────────────────────────────────────────────────
// The Pi has four cores. The frame loop gets the last to itself; every other thread shares
// the first three. Stem's unit pins it to the same three, so nothing outside the brain lands
// on the frame loop's core either.
pub const FRAME_LOOP_CORES: &[usize] = &[3];
pub const BACKGROUND_CORES: &[usize] = &[0, 1, 2];

// The frame loop's real-time priority. Any FIFO priority puts it ahead of every ordinary
// thread; 50 is the one the brain's unit file set before the threads set their own.
pub const REALTIME_PRIORITY: i32 = 50;

// ── E1.31 receive / external takeover ────────────────────────────────────────
// The E1.31 network-data-loss timeout: an external source that goes quiet this long loses
// the universe and the internal engine resumes. QLC+ does not send the stream-terminated bit,
// so this is what hands the universe back from it; a source that does send it hands back
// inside one frame.
pub const SACN_SOURCE_TIMEOUT_US: u64 = 2_500_000;

// Distinct senders tracked at once. Normally one console; a few more absorbs strays without
// unbounded storage, and an expiring entry frees a slot within the timeout above.
pub const SACN_MAX_SOURCES: usize = 4;

// Rebind backoff cap for the receive socket, while the address it binds is not up yet.
// Retrying rather than dying means the takeover path comes back the moment its interface
// does, and the sculpture keeps breathing on the internal engine meanwhile.
pub const SACN_BIND_RETRY_MAX_S: u64 = 30;

// Fixture addressing, the universe, the sACN port and the address a console is received on
// are not here: they are generated from the rig's QLC+ workspace into the rig's own `patch`
// module, the single source of truth for what is patched where.

// ── The rig state ────────────────────────────────────────────────────────────
// How long the e-stop must be held, without a break, after the brain starts, before its
// release lets the rig run: long enough to be a deliberate press by a person at the cabinet,
// not a bounce or a brush.
pub const ESTOP_ARM_HOLD_US: u64 = 500_000;

// ── Wired DMX-512 output (Zihatec RS-485 HAT) ─────────────────────────────────
// The HAT is clocked with the same slot buffer as the sACN stream, so the wired universe
// mirrors the wireless one — one universe, two transports. Framing (BREAK/MAB timing,
// full-512 padding) lives in the zihatec-rs-485-dmx crate; only the device path is ours.
// Always open the /dev/serial0 alias, never a bare ttyAMA*/ttyS0: it stays stable while
// USB serial adapters renumber around it, and resolves to the PL011 once Bluetooth is
// disabled.
pub const SERIAL_DEVICE: &str = "/dev/serial0";
