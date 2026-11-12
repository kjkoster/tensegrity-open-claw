//! Stem as the Rust programs see it: the rig's MQTT conventions — the health contract, and the
//! handful of MQTT 3.1.1 packets a publish-only client needs to keep it. The transport is each
//! program's own: the brain's client is `cortex::telemetry`, a firmware's is its own task.
//!
//! The Rust half of `stem/rig_mqtt.py`, and it exists for the same reason that file does: a
//! liveness convention that two processes implement slightly differently is worse than none,
//! because the disagreement only shows up in the telemetry you went looking at during a failure.
//! The brain and a fixture's firmware hold their connections very differently — a std thread on
//! one, an Embassy task on the other — so what is shared is everything that is not the
//! transport, and it is `no_std` with no allocator so any firmware can build it: every packet is
//! written into a buffer its caller owns.
//!
//! ## health/<service>
//!
//! Every client publishes its own liveness, retained, as one of three words. `connected` and
//! `disconnected` come from the client; `gone` comes from the *broker*, out of the will the
//! client handed it in the CONNECT packet. A program cannot announce its own crash, so it hands
//! the announcement over in advance, and the broker makes it when the connection dies without a
//! DISCONNECT. That is what makes `disconnected` (a stop, a restart, a deep sleep) and `gone` (a
//! crash, a lost link, a lost supply) a real distinction.
//!
//! ## sys/<owner> and dmx/<owner>
//!
//! Facts the whole rig is read by are published under a rig-wide root, with the owner beneath
//! it, rather than under the owner's own name: what a client runs on — board, cores, memory,
//! build — under `sys/`, and where a client or a part sits on the universe — universe,
//! address, frame rate — under `dmx/`. Both retained. `sys/#` is then the hardware of the whole
//! rig in one subscription, and `dmx/#` its patch, the way `health/#` is its liveness.
//!
//! Every other topic a client publishes sits under its own service name, one scalar per topic.
//!
//! ## Why encoded here and not taken from a crate
//!
//! A client that publishes at QoS 0 and never subscribes needs a CONNECT, a PUBLISH, a PINGREQ
//! and a DISCONNECT, and no session state, inflight window or subscription table. The crates
//! that do this properly bring an async runtime and an allocator with them, neither of which a
//! firmware should have to carry for four packet types.

#![no_std]

pub const CONNECTED: &str = "connected";
pub const DISCONNECTED: &str = "disconnected";
pub const GONE: &str = "gone";

/// The broker publishes a client's will once it has heard nothing for 1.5× this, so a dead
/// client is declared gone inside 15 seconds. The same value as `rig_mqtt.KEEPALIVE_S`, and
/// every client pings at half of it, so one lost ping is not a disconnection.
pub const KEEPALIVE_S: u16 = 10;

/// The packets that are nothing but a fixed header. A PINGRESP is the only thing a broker
/// sends a publish-only client once it is connected, so it is also the whole of what one
/// reads back.
pub const PINGREQ: [u8; 2] = [0xC0, 0x00];
pub const PINGRESP: [u8; 2] = [0xD0, 0x00];
pub const DISCONNECT: [u8; 2] = [0xE0, 0x00];

/// A CONNACK is fixed at four bytes, so a client reads exactly this many after its CONNECT.
pub const CONNACK_LEN: usize = 4;

const CONNECT: u8 = 0x10;
const CONNACK: u8 = 0x20;
const PUBLISH: u8 = 0x30;
const PUBLISH_RETAIN: u8 = 0x01;

/// Clean session, a will, and the will retained. The will's QoS stays 0 like everything else,
/// so its two flag bits are left clear.
const CONNECT_FLAGS: u8 = 0x02 | 0x04 | 0x20;

/// The largest remaining length four bytes of MQTT's variable-length integer can say.
const MAX_REMAINING: usize = 268_435_455;

const PROTOCOL_NAME: &str = "MQTT";
const PROTOCOL_LEVEL: u8 = 0x04;

/// The roots of the rig-wide trees: `health/<service>`, `sys/<owner>/<leaf>` and
/// `dmx/<owner>/<leaf>`.
pub const HEALTH: &str = "health";
pub const SYS: &str = "sys";
pub const DMX: &str = "dmx";

/// A packet did not fit the buffer it was being written into.
#[derive(Debug)]
pub struct TooLong;

/// A boolean as a payload. Spelled out rather than `{}`-formatted so every client on the rig
/// agrees with `rig_mqtt.flatten`, which has to avoid Python's capitalised `True`.
pub fn boolean(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// The CONNECT for `service`, its will — `gone` on its health topic, retained — included,
/// written into `out`. Returns the packet's length.
///
/// The will goes in the CONNECT packet and nowhere else: it has to be in the broker's hands
/// before the connection can fail, or it protects nothing.
pub fn connect(out: &mut [u8], service: &str) -> Result<usize, TooLong> {
    let will = [HEALTH, service];
    let body = string_len(&[PROTOCOL_NAME])
        + 1
        + 1
        + 2
        + string_len(&[service])
        + string_len(&will)
        + string_len(&[GONE]);

    let mut packet = Writer::new(out);
    packet.header(CONNECT, body)?;
    packet.string(&[PROTOCOL_NAME])?;
    packet.byte(PROTOCOL_LEVEL)?;
    packet.byte(CONNECT_FLAGS)?;
    packet.bytes(&KEEPALIVE_S.to_be_bytes())?;
    // The service is the client ID too. Two clients sharing one would take each other's
    // session over, and their health with it.
    packet.string(&[service])?;
    packet.string(&will)?;
    packet.string(&[GONE])?;
    Ok(packet.len)
}

/// A PUBLISH at QoS 0 written into `out`, its topic the `levels` joined by `/`. Returns the
/// packet's length.
///
/// No leading slash: `health/brain`, not `/health/brain`. A leading slash is legal MQTT but
/// makes an anonymous empty first level, and `health/#` then matches nothing.
pub fn publish(
    out: &mut [u8],
    levels: &[&str],
    payload: &str,
    retain: bool,
) -> Result<usize, TooLong> {
    // No packet identifier: that field exists only at QoS 1 and above.
    let body = string_len(levels) + payload.len();

    let mut packet = Writer::new(out);
    packet.header(PUBLISH | if retain { PUBLISH_RETAIN } else { 0 }, body)?;
    packet.string(levels)?;
    packet.bytes(payload.as_bytes())?;
    Ok(packet.len)
}

/// Why a broker would not have us.
#[derive(Debug)]
pub enum Refused {
    /// Something other than a CONNACK arrived first.
    NotConnack(u8),
    /// The broker's return code: a bad protocol level, a rejected client ID, and so on.
    Code(u8),
}

pub fn read_connack(packet: &[u8; CONNACK_LEN]) -> Result<(), Refused> {
    if packet[0] != CONNACK {
        return Err(Refused::NotConnack(packet[0]));
    }
    match packet[3] {
        0 => Ok(()),
        code => Err(Refused::Code(code)),
    }
}

/// `levels` joined by `/`, as MQTT carries it: a two-byte length, then the UTF-8.
fn string_len(levels: &[&str]) -> usize {
    2 + levels.iter().map(|level| level.len()).sum::<usize>() + levels.len().saturating_sub(1)
}

/// A cursor over a caller's buffer that refuses to run past its end.
struct Writer<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl<'a> Writer<'a> {
    fn new(out: &'a mut [u8]) -> Self {
        Self { out, len: 0 }
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), TooLong> {
        let end = self.len + bytes.len();
        self.out
            .get_mut(self.len..end)
            .ok_or(TooLong)?
            .copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }

    fn byte(&mut self, byte: u8) -> Result<(), TooLong> {
        self.bytes(&[byte])
    }

    /// The fixed header: the packet type, then the remaining length as the variable-length
    /// integer MQTT uses, seven bits per byte with the top bit marking that another follows.
    /// It has at most four bytes, so a packet longer than that can carry is too long whatever
    /// the buffer.
    fn header(&mut self, kind: u8, mut remaining: usize) -> Result<(), TooLong> {
        if remaining > MAX_REMAINING {
            return Err(TooLong);
        }
        self.byte(kind)?;
        loop {
            let mut byte = (remaining % 128) as u8;
            remaining /= 128;
            if remaining > 0 {
                byte |= 0x80;
            }
            self.byte(byte)?;
            if remaining == 0 {
                return Ok(());
            }
        }
    }

    /// Length-prefixed UTF-8, which is how MQTT carries every string: here `levels` joined
    /// by `/`.
    fn string(&mut self, levels: &[&str]) -> Result<(), TooLong> {
        let len = u16::try_from(string_len(levels) - 2).map_err(|_| TooLong)?;
        self.bytes(&len.to_be_bytes())?;
        for (index, level) in levels.iter().enumerate() {
            if index > 0 {
                self.byte(b'/')?;
            }
            self.bytes(level.as_bytes())?;
        }
        Ok(())
    }
}
