//! Brain's own connection to the broker: the health contract, and a place to publish from.
//!
//! Every daemon on the rig holds its own client and announces itself on `health/<service>`,
//! so that a dead process says so without anything else having to notice. This is the brain's
//! end of that, and the reason it cannot be delegated to Stem: only the process itself can
//! hand the broker a will, and only a will distinguishes "stopped" from "died".
//!
//! MQTT 3.1.1 is encoded here rather than taken from a crate, for the same reason E1.31 is.
//! The brain publishes, never subscribes, always at QoS 0 — which is a CONNECT, a PUBLISH, a
//! PINGREQ and a DISCONNECT, and no session state, no inflight window, no subscription table.
//! The crates that do this properly bring an async runtime with them, and a 1.2 GHz Pi that
//! rebuilds on every deploy should not pay for one so a health daemon can move a few hundred
//! bytes a second.
//!
//! Nothing here may touch frame timing. The publish queue is bounded and drops on overflow, a
//! broker that has gone away is a reconnect on a background thread, and the frame loop's only
//! interaction with any of it is a non-blocking send into a channel.
//!
//! One field per topic, never a JSON document. A payload that has to be parsed before it can
//! be read is one no browser, `mosquitto_sub` or dashboard can chart, and the structure a
//! document would carry is structure MQTT already has in the topic path. This end therefore
//! takes a topic and a scalar and does no encoding at all — callers name the leaf.

use crate::config as cfg;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddrV4, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// ── MQTT 3.1.1 control packets (OASIS mqtt-v3.1.1) ───────────────────────────
const CONNECT: u8 = 0x10;
const CONNACK: u8 = 0x20;
const PUBLISH: u8 = 0x30;
const PINGREQ: u8 = 0xC0;
const DISCONNECT: u8 = 0xE0;

const PUBLISH_RETAIN: u8 = 0x01;

/// Connect flags: clean session, a will, and the will retained. The will's QoS stays 0 like
/// everything else here, so its two flag bits are left clear.
const CONNECT_CLEAN_SESSION: u8 = 0x02;
const CONNECT_WILL: u8 = 0x04;
const CONNECT_WILL_RETAIN: u8 = 0x20;

const PROTOCOL_NAME: &str = "MQTT";
const PROTOCOL_LEVEL: u8 = 0x04;

/// Housekeeping reads only. The client never subscribes and has nothing to wait for, so this
/// bounds how long the loop sits in a drain rather than how long it waits for an answer.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(50);

/// The three words on `health/<service>`, published retained.
///
/// The distinction is only meaningful because the first two come from the process and the
/// third comes from the broker: a program cannot announce its own crash, so it hands the
/// announcement over at connect time and the broker makes it when the connection dies without
/// a goodbye. `DISCONNECTED` is somebody running `systemctl stop`; `GONE` is a segfault, an
/// OOM kill, or a Pi that lost power.
const CONNECTED: &str = "connected";
const DISCONNECTED: &str = "disconnected";
const GONE: &str = "gone";

/// One retained-or-not message on its way to the broker.
struct Message {
    topic: String,
    payload: String,
    retain: bool,
}

/// What the telemetry thread can be asked to do. Both arrive on one channel so the goodbye
/// cannot overtake a publish that was already queued.
enum Command {
    Publish(Message),
    Farewell,
}

/// The frame loop's end of the telemetry: a bounded queue and nothing else.
///
/// Cloneable, so any stage can hold one. Every send is non-blocking and a full queue drops the
/// message rather than waiting — a broker that has wedged must cost telemetry and never a
/// frame, and the newest reading is worth more than the backlog behind it anyway.
#[derive(Clone)]
pub struct Publisher {
    outgoing: Option<SyncSender<Command>>,
}

impl Publisher {
    /// A publisher that goes nowhere, for a rig built without telemetry.
    pub fn disabled() -> Self {
        Self { outgoing: None }
    }

    /// Queues one message under this process's own prefix. Never blocks, never fails loudly.
    pub fn publish(&self, topic: impl Into<String>, payload: impl Into<String>, retain: bool) {
        let Some(outgoing) = &self.outgoing else {
            return;
        };
        let message = Command::Publish(Message {
            topic: topic.into(),
            payload: payload.into(),
            retain,
        });
        // A dropped reading is logged nowhere on purpose: the condition that fills this queue
        // is a broker that stopped reading, which the health tree already reports, and a log
        // line per dropped message would be a second failure printed on top of the first.
        match outgoing.try_send(message) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

/// The shutdown end, held by whatever handles SIGTERM.
///
/// Separate from `Publisher` because it is not something a pipeline stage should be able to
/// reach: saying goodbye ends the connection for everybody.
pub struct Farewell {
    outgoing: SyncSender<Command>,
    said: Arc<AtomicBool>,
}

impl Farewell {
    /// Publishes the goodbye and waits, briefly, for it to actually leave.
    ///
    /// Waiting is the point. The caller is about to `exit`, and a goodbye still sitting in a
    /// channel is a goodbye the broker never hears — at which point the will fires and an
    /// orderly stop is recorded as a crash. Bounded, because a wedged broker must not be able
    /// to hold up a shutdown: past the deadline the will fires and `gone` is, at worst,
    /// pessimistic about a rig that really is going away.
    pub fn say(&self) {
        let expires = Instant::now() + Duration::from_millis(cfg::MQTT_FAREWELL_MS);
        let mut pending = Some(Command::Farewell);
        while let Some(command) = pending.take() {
            match self.outgoing.try_send(command) {
                Ok(()) => break,
                // The queue is full because the thread is busy or the broker is slow. Hand the
                // command back and try again until the deadline, rather than dropping the
                // goodbye over a queue that was about to drain anyway.
                Err(TrySendError::Full(returned)) if Instant::now() < expires => {
                    pending = Some(returned);
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return,
            }
        }
        while !self.said.load(Ordering::Acquire) && Instant::now() < expires {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

/// Spawns the telemetry client and returns the handles the rest of the brain speaks through.
///
/// `service` names the health topic and prefixes every other topic this process publishes, so
/// the tree says which process asserted a thing.
pub fn spawn(service: &'static str) -> (JoinHandle<()>, Publisher, Farewell) {
    let (outgoing, incoming) = sync_channel(cfg::MQTT_QUEUE_DEPTH);
    let said = Arc::new(AtomicBool::new(false));

    let thread_said = said.clone();
    let handle = thread::Builder::new()
        .name("telemetry".into())
        .spawn(move || {
            let mut backoff_s = 1u64;
            loop {
                match session(service, &incoming, &thread_said) {
                    // The goodbye went out and the connection was closed cleanly. There is
                    // nothing left to reconnect for; the process is on its way out.
                    Ok(()) => return,
                    Err(e) => eprintln!("telemetry: {e}"),
                }
                thread::sleep(Duration::from_secs(backoff_s));
                backoff_s = (backoff_s * 2).min(cfg::MQTT_RETRY_MAX_S);
            }
        })
        .expect("failed to spawn telemetry thread");

    (
        handle,
        Publisher {
            outgoing: Some(outgoing.clone()),
        },
        Farewell { outgoing, said },
    )
}

/// Connects, announces, and pumps the queue. Returns only once the goodbye has been said.
fn session(
    service: &str,
    incoming: &Receiver<Command>,
    said: &AtomicBool,
) -> std::io::Result<()> {
    let address = SocketAddrV4::new(cfg::MQTT_ADDRESS, cfg::MQTT_PORT);
    let mut stream = TcpStream::connect(address)?;
    // Nagle would hold a small publish back waiting for company that never comes; these are
    // one-datagram-sized messages seconds apart.
    stream.set_nodelay(true)?;

    let health = format!("health/{service}");
    stream.write_all(&encode_connect(service, &health))?;
    read_connack(&mut stream)?;
    eprintln!("telemetry: connected to {address} as {service}");

    stream.write_all(&encode_publish(&health, CONNECTED, true))?;

    // Half the keepalive: the broker drops a client at 1.5× the interval it was given, so
    // pinging twice per interval means one lost ping is not a disconnection.
    let tick = Duration::from_secs(cfg::MQTT_KEEPALIVE_S / 2);
    loop {
        match incoming.recv_timeout(tick) {
            Ok(Command::Publish(message)) => {
                stream.write_all(&encode_publish(
                    &format!("{service}/{}", message.topic),
                    &message.payload,
                    message.retain,
                ))?;
            }
            Ok(Command::Farewell) => {
                // On this connection, and followed by a clean DISCONNECT. A goodbye sent over
                // a second connection would be overwritten moments later by the will firing
                // on this one — the broker publishes it whenever a connection drops without a
                // DISCONNECT, and an exiting process drops all of them.
                stream.write_all(&encode_publish(&health, DISCONNECTED, true))?;
                stream.write_all(&[DISCONNECT, 0x00])?;
                stream.flush()?;
                said.store(true, Ordering::Release);
                return Ok(());
            }
            // Nothing to say. The ping is what keeps the broker from declaring the will on a
            // rig that is simply quiet.
            Err(RecvTimeoutError::Timeout) => stream.write_all(&[PINGREQ, 0x00])?,
            // Every publisher has been dropped, which cannot happen while the brain runs. Keep
            // the connection alive anyway: the health topic is still worth being true.
            Err(RecvTimeoutError::Disconnected) => stream.write_all(&[PINGREQ, 0x00])?,
        }
        drain(&mut stream)?;
    }
}

/// Reads and discards whatever the broker sent — PINGRESPs, mostly — and notices a close.
///
/// Unread bytes are not free: a PINGRESP every fifteen seconds fills the receive buffer over
/// days, the TCP window shuts, and the broker eventually blocks writing to a client that was
/// never listening. Draining is what keeps a publish-only client honest.
fn drain(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(DRAIN_TIMEOUT))?;
    let mut scratch = [0u8; 64];
    loop {
        match stream.read(&mut scratch) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::ConnectionAborted,
                    "broker closed the connection",
                ));
            }
            Ok(_) => continue,
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }
}

fn read_connack(stream: &mut TcpStream) -> std::io::Result<()> {
    // A CONNACK is fixed at four bytes, so this reads exactly that rather than parsing a
    // length it already knows. The connection is unusable until it arrives, which is why this
    // one read waits far longer than the drain does.
    stream.set_read_timeout(Some(Duration::from_secs(cfg::MQTT_CONNACK_TIMEOUT_S)))?;
    let mut packet = [0u8; 4];
    stream.read_exact(&mut packet)?;

    if packet[0] != CONNACK {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("expected CONNACK, got 0x{:02x}", packet[0]),
        ));
    }
    if packet[3] != 0 {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            format!("broker refused the connection, code {}", packet[3]),
        ));
    }
    Ok(())
}

// ── Packet encoding ──────────────────────────────────────────────────────────

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

fn encode_connect(client_id: &str, health: &str) -> Vec<u8> {
    let mut body = Vec::new();
    push_string(&mut body, PROTOCOL_NAME);
    body.push(PROTOCOL_LEVEL);
    body.push(CONNECT_CLEAN_SESSION | CONNECT_WILL | CONNECT_WILL_RETAIN);
    body.extend_from_slice(&(cfg::MQTT_KEEPALIVE_S as u16).to_be_bytes());

    push_string(&mut body, client_id);
    // The will, in the CONNECT packet and nowhere else: it has to be in the broker's hands
    // before the connection can fail, or it protects nothing.
    push_string(&mut body, health);
    push_string(&mut body, GONE);

    let mut packet = vec![CONNECT];
    push_remaining_length(&mut packet, body.len());
    packet.extend_from_slice(&body);
    packet
}

fn encode_publish(topic: &str, payload: &str, retain: bool) -> Vec<u8> {
    let mut body = Vec::new();
    push_string(&mut body, topic);
    // No packet identifier: that field exists only at QoS 1 and above, and everything here is
    // fire-and-forget by design.
    body.extend_from_slice(payload.as_bytes());

    let mut packet = vec![PUBLISH | if retain { PUBLISH_RETAIN } else { 0 }];
    push_remaining_length(&mut packet, body.len());
    packet.extend_from_slice(&body);
    packet
}
