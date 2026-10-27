//! The mouth's settings: which interface it plays through, and how it drives it.

// The interface is whichever class-compliant USB audio interface is in the cabinet — today an
// Alesis io|2, the one the ear used. Swapping it is these three lines: its ALSA card name, and
// its USB identity for the probe. `plughw` rather than `hw`, so ALSA converts the 16-bit
// samples to whatever width the interface takes natively.
pub const ALSA_DEVICE: &str = "plughw:CARD=io2,DEV=0"; // confirm with `aplay -L`
pub const USB_VENDOR: u16 = 0x13b2; // confirm with `lsusb`
pub const USB_PRODUCT: u16 = 0x0008;

// Small, because the cut is the point: a mute lands in the next period the mouth fills, and a
// sound starts at the next period after its cue. 256 frames is 5.3 ms at 48 kHz, and two of
// them in the buffer put a cue about 11 ms behind the frame that asked for it. A buffer this
// short underruns if the thread is kept from running for 5 ms; `mouth/xruns` says when it
// does, and the answer is a bigger buffer.
pub const PERIOD_FRAMES: usize = 256;
pub const PERIODS_PER_BUFFER: usize = 2;

// How many sounds can play at once. A cue past this takes over the voice that has played
// longest, which is the one nearest its end. Allocated once, so playing never allocates.
pub const VOICES: usize = 8;

// The cap on the backoff between attempts to reopen a failed interface.
pub const DEVICE_RETRY_MAX_S: u64 = 30;
