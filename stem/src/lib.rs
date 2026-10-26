//! Stem as the Rust programs see it: the rig's MQTT conventions — the health contract, and the
//! handful of MQTT 3.1.1 packets a publish-only client needs to keep it. The transport is each
//! program's own: the brain's client is `cortex::telemetry`, a firmware's is its own task.
//!
//! The Rust half of `stem/rig_mqtt.py`, and it exists for the same reason that file does: a
//! liveness convention that two processes implement slightly differently is worse than none,
//! because the disagreement only shows up in the telemetry you went looking at during a failure.
//! The brain and a fixture's firmware hold their connections very differently — a std thread on
//! one, an Embassy task on the other — so what is shared is everything that is not the
//! transport, and it is `no_std` so firmware can build it.
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
//! that do this properly bring an async runtime with them, and the brain is rebuilt on a 1.2 GHz
//! Pi on every deploy.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub const CONNECTED: &str = "connected";
pub const DISCONNECTED: &str = "disconnected";
pub const GONE: &str = "gone";

pub const PORT: u16 = 1883;

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

const PROTOCOL_NAME: &str = "MQTT";
const PROTOCOL_LEVEL: u8 = 0x04;

/// No leading slash: `health/brain`, not `/health/brain`. A leading slash is legal MQTT but makes
/// an anonymous empty first level, and `health/#` then matches nothing.
pub fn health_topic(service: &str) -> String {
    format!("health/{service}")
}

/// Where one rig-wide fact about `owner` is published: `sys/brain/board`, `dmx/face/address`.
pub fn rooted_topic(root: &str, owner: &str, leaf: &str) -> String {
    format!("{root}/{owner}/{leaf}")
}

/// A boolean as a payload. Spelled out rather than `{}`-formatted so every client on the rig
/// agrees with `rig_mqtt.flatten`, which has to avoid Python's capitalised `True`.
pub fn boolean(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// The CONNECT for `service`, its will — `gone` on its health topic, retained — included.
///
/// The will goes in the CONNECT packet and nowhere else: it has to be in the broker's hands
/// before the connection can fail, or it protects nothing.
pub fn connect(service: &str) -> Vec<u8> {
    let mut body = Vec::new();
    push_string(&mut body, PROTOCOL_NAME);
    body.push(PROTOCOL_LEVEL);
    body.push(CONNECT_FLAGS);
    body.extend_from_slice(&KEEPALIVE_S.to_be_bytes());

    // The service is the client ID too. Two clients sharing one would take each other's
    // session over, and their health with it.
    push_string(&mut body, service);
    push_string(&mut body, &health_topic(service));
    push_string(&mut body, GONE);

    packet(CONNECT, &body)
}

pub fn publish(topic: &str, payload: &str, retain: bool) -> Vec<u8> {
    let mut body = Vec::new();
    push_string(&mut body, topic);
    // No packet identifier: that field exists only at QoS 1 and above.
    body.extend_from_slice(payload.as_bytes());

    packet(PUBLISH | if retain { PUBLISH_RETAIN } else { 0 }, &body)
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

fn packet(header: u8, body: &[u8]) -> Vec<u8> {
    let mut packet = vec![header];
    push_remaining_length(&mut packet, body.len());
    packet.extend_from_slice(body);
    packet
}

/// The variable-length integer MQTT uses for remaining length: seven bits per byte, the top
/// bit marking that another follows.
fn push_remaining_length(packet: &mut Vec<u8>, mut length: usize) {
    loop {
        let mut byte = (length % 128) as u8;
        length /= 128;
        if length > 0 {
            byte |= 0x80;
        }
        packet.push(byte);
        if length == 0 {
            return;
        }
    }
}

/// Length-prefixed UTF-8, which is how MQTT carries every string.
fn push_string(body: &mut Vec<u8>, text: &str) {
    body.extend_from_slice(&(text.len() as u16).to_be_bytes());
    body.extend_from_slice(text.as_bytes());
}
