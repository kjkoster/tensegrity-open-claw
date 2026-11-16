//! Memory's connection to the broker: the relay of every device's OSC onto MQTT, the bridge for
//! troubleshooting from the Mac, and the health contract that comes with a client.
//!
//! The client announces itself on `health/<service>`, and hands the broker a will, so a dead
//! Memory says so without anything else having to notice.
//!
//! The packets and the health words are the `stem` crate's, shared with any firmware. The
//! client publishes, never subscribes, always at QoS 0, and every relayed value retained, so a
//! subscriber arriving later sees the last of each. The broker keeps nothing across a restart,
//! so the client replays every retained value on each connect.
//!
//! Nothing here may hold up its caller. The publish queue is bounded and drops on overflow, and
//! a broker that has gone away is a reconnect on a background thread.
//!
//! One field per topic, never a JSON document. A payload that has to be parsed before it can
//! be read is one no browser, `mosquitto_sub` or dashboard can chart, and the structure a
//! document would carry is structure MQTT already has in the topic path.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddrV4, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

/// The broker's settings: Stem owns the broker, and every daemon holds its own client and
/// announces itself under `health/`. Its address and port are generated from `clockwork.dot`;
/// the keepalive is in the `stem` crate.
mod cfg {
    /// How long a connection attempt may take before it counts as failed and backs off. The
    /// broker is across the WiFi, and a broker that is off would otherwise leave the attempt
    /// to the kernel's own SYN retries, the better part of two minutes.
    pub const MQTT_CONNECT_TIMEOUT_S: u64 = 5;

    /// How long to wait for the CONNACK before treating the broker as unusable. Generous: the
    /// only things that make it slow are a broker still starting up and a WiFi link catching
    /// up.
    pub const MQTT_CONNACK_TIMEOUT_S: u64 = 5;

    /// How long a write may block before the broker counts as gone. Writes land in the socket
    /// buffer and return at once while the broker keeps up; one that blocks this long is
    /// writing to a peer that is no longer reading.
    pub const MQTT_WRITE_TIMEOUT_S: u64 = 5;

    /// Messages that may sit queued for the telemetry thread: enough for every device's
    /// bundles arriving together, as the 1 Hz ones do; past it, the newest reading is worth
    /// more than the backlog behind it.
    pub const MQTT_QUEUE_DEPTH: usize = 1024;

    /// The largest packet the client encodes. Every topic is a short path and every payload
    /// one scalar, so a message that does not fit is a sender's mistake, logged and dropped.
    pub const MQTT_PACKET_MAX: usize = 512;

    /// Reconnect backoff, matching the other receivers': a broker that comes back is picked up
    /// without anything being restarted.
    pub const MQTT_RETRY_MAX_S: u64 = 30;

    /// How long a shutdown waits for the goodbye to actually leave. The alternative to waiting
    /// is the will firing and an orderly stop being recorded as a crash, so it is worth a
    /// moment — but only a moment, because a wedged broker must never be able to hold up a
    /// shutdown.
    pub const MQTT_FAREWELL_MS: u64 = 500;
}

/// How long a drain waits for the broker. The client never subscribes and has nothing to wait
/// for, so this bounds how long the loop sits in a drain rather than how long it waits for an
/// answer.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(50);

/// One retained message on its way to the broker, at its topic as given: what Memory relays is
/// a device's, and already names it.
struct Message {
    topic: String,
    payload: String,
}

/// What the telemetry thread can be asked to do. Both arrive on one channel so the goodbye
/// cannot overtake a publish that was already queued.
enum Command {
    Publish(Message),
    Farewell,
}

/// A stage's end of the telemetry: a bounded queue and nothing else.
///
/// Cloneable, so any stage can hold one. Every send is non-blocking and a full queue drops the
/// message rather than waiting — a broker that has wedged must cost telemetry and never a
/// frame, and the newest reading is worth more than the backlog behind it anyway.
#[derive(Clone)]
pub struct Publisher {
    outgoing: SyncSender<Command>,
}

impl Publisher {
    /// Queues one message at `topic` as given, retained: a relay of what a device sent, which
    /// already names that device, kept so a subscriber arriving later sees the last. Never
    /// blocks, never fails loudly.
    pub fn relay(&self, topic: impl Into<String>, payload: impl Into<String>) {
        let message = Command::Publish(Message {
            topic: topic.into(),
            payload: payload.into(),
        });
        // A dropped reading is logged nowhere on purpose: the condition that fills this queue
        // is a broker that stopped reading, which the health tree already reports, and a log
        // line per dropped message would be a second failure printed on top of the first.
        let _ = self.outgoing.try_send(message);
    }
}

/// The shutdown end of one client, held by whatever handles SIGTERM.
///
/// Separate from `Publisher` because it is not something a pipeline stage should be able to
/// reach: saying goodbye ends the connection for everybody.
pub struct Farewell {
    outgoing: SyncSender<Command>,
    said: Arc<AtomicBool>,
}

impl Farewell {
    /// Queues the goodbye, retrying a full queue until `expires`. Returns whether it was
    /// queued: a thread that has already exited will never say it.
    fn ask(&self, expires: Instant) -> bool {
        loop {
            match self.outgoing.try_send(Command::Farewell) {
                Ok(()) => return true,
                // The queue is full because the thread is busy or the broker is slow. Try again
                // until the deadline, rather than dropping the goodbye over a queue that was
                // about to drain anyway.
                Err(TrySendError::Full(_)) if Instant::now() < expires => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return false,
            }
        }
    }

    /// Says the goodbye and waits, briefly, for it to actually leave.
    ///
    /// Waiting is the point. The caller is about to `exit`, and a goodbye still sitting in a
    /// channel is a goodbye the broker never hears — at which point the will fires and an
    /// orderly stop is recorded as a crash. The wait is bounded, because a wedged broker must
    /// not be able to hold up a shutdown: past it the will fires and `gone` is, at worst,
    /// pessimistic about a rig that really is going away.
    pub fn say(&self) {
        let expires = Instant::now() + Duration::from_millis(cfg::MQTT_FAREWELL_MS);
        if !self.ask(expires) {
            return;
        }
        while !self.said.load(Ordering::Acquire) && Instant::now() < expires {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

/// Spawns a client and returns the handles the rest of the process speaks through.
///
/// `service` names the health topic and is the client ID. `broker` is the rig's, generated
/// from its `clockwork.dot`.
pub fn spawn(service: &'static str, broker: SocketAddrV4) -> (Publisher, Farewell) {
    let (outgoing, incoming) = sync_channel(cfg::MQTT_QUEUE_DEPTH);
    let said = Arc::new(AtomicBool::new(false));

    let thread_said = said.clone();
    thread::Builder::new()
        .name(format!("telemetry-{service}"))
        .spawn(move || {
            cortex::sched::background();
            // Every retained topic this client has published, replayed on each connect.
            //
            // Retained state is a promise that a late subscriber sees the live picture, and a
            // broker restart breaks it: the broker keeps nothing across one, so whatever was
            // published once at startup would otherwise be gone for good.
            let mut retained = BTreeMap::new();
            let mut backoff_s = 1u64;
            loop {
                match session(
                    service,
                    broker,
                    &incoming,
                    &thread_said,
                    &mut retained,
                    &mut backoff_s,
                ) {
                    // The goodbye went out and the connection was closed cleanly, or nobody
                    // is left to say anything. There is nothing left to reconnect for.
                    Ok(()) => return,
                    Err(e) => eprintln!("telemetry: {service}: {e}"),
                }
                thread::sleep(Duration::from_secs(backoff_s));
                backoff_s = (backoff_s * 2).min(cfg::MQTT_RETRY_MAX_S);
            }
        })
        .expect("failed to spawn telemetry thread");

    (
        Publisher {
            outgoing: outgoing.clone(),
        },
        Farewell { outgoing, said },
    )
}

/// Connects, announces, and pumps the queue. Returns only once the goodbye has been said, or
/// once nobody is left to say anything.
fn session(
    service: &str,
    address: SocketAddrV4,
    incoming: &Receiver<Command>,
    said: &AtomicBool,
    retained: &mut BTreeMap<String, String>,
    backoff_s: &mut u64,
) -> std::io::Result<()> {
    let mut stream = TcpStream::connect_timeout(
        &address.into(),
        Duration::from_secs(cfg::MQTT_CONNECT_TIMEOUT_S),
    )?;
    // Nagle would hold a small publish back waiting for company that never comes; these are
    // one-datagram-sized messages seconds apart.
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(Duration::from_secs(cfg::MQTT_WRITE_TIMEOUT_S)))?;

    let mut buffer = [0u8; cfg::MQTT_PACKET_MAX];
    let len = stem::connect(&mut buffer, service).expect("a CONNECT fits the packet buffer");
    stream.write_all(&buffer[..len])?;
    read_connack(&mut stream)?;
    eprintln!("telemetry: {service}: connected to {address}");
    // Only once the broker has accepted us: a broker that keeps refusing must keep backing off,
    // and one that was simply restarted must not be waited on as if it had.
    *backoff_s = 1;

    let health = [stem::HEALTH, service];
    let len = stem::publish(&mut buffer, &health, stem::CONNECTED, true)
        .expect("a health message fits the packet buffer");
    stream.write_all(&buffer[..len])?;
    // Only messages that fitted were retained, so each fits again.
    for (topic, payload) in retained.iter() {
        let len = stem::publish(&mut buffer, &[topic], payload, true)
            .expect("a retained message fitted when it was first sent");
        stream.write_all(&buffer[..len])?;
    }

    // Half the keepalive: the broker drops a client at 1.5× the interval it was given, so
    // pinging twice per interval means one lost ping is not a disconnection.
    let tick = Duration::from_secs(u64::from(stem::KEEPALIVE_S) / 2);
    // The client's side of the same rule. A broker can vanish without a word — one across a
    // network losing power — and writes into its dead connection keep succeeding into the
    // socket buffer for many minutes. Each ping's answer is read at the next ping's drain, so a
    // slow link costs nothing; two pings in a row with nothing read back mean the broker has
    // gone, and the session ends so the reconnect can begin.
    let mut unanswered = 0;
    loop {
        match incoming.recv_timeout(tick) {
            Ok(Command::Publish(Message { topic, payload })) => {
                let Ok(len) = stem::publish(&mut buffer, &[&topic], &payload, true) else {
                    eprintln!(
                        "telemetry: {service}: {topic} does not fit {} bytes, dropped",
                        cfg::MQTT_PACKET_MAX
                    );
                    continue;
                };
                // Recorded before the write, so a value whose write is what failed is the one
                // the next connect replays.
                retained.insert(topic, payload);
                stream.write_all(&buffer[..len])?;
            }
            Ok(Command::Farewell) => {
                // On this connection, and followed by a clean DISCONNECT. A goodbye sent over
                // a second connection would be overwritten moments later by the will firing
                // on this one — the broker publishes it whenever a connection drops without a
                // DISCONNECT, and an exiting process drops all of them.
                let len = stem::publish(&mut buffer, &health, stem::DISCONNECTED, true)
                    .expect("a health message fits the packet buffer");
                stream.write_all(&buffer[..len])?;
                stream.write_all(&stem::DISCONNECT)?;
                stream.flush()?;
                said.store(true, Ordering::Release);
                return Ok(());
            }
            // Nothing to say. The ping is what keeps the broker from declaring the will on a
            // client that is simply quiet. Draining only here, after the one packet that is
            // ever answered, keeps the drain's wait off every publish.
            Err(RecvTimeoutError::Timeout) => {
                stream.write_all(&stem::PINGREQ)?;
                if drain(&mut stream)? {
                    unanswered = 0;
                } else {
                    unanswered += 1;
                    if unanswered >= 2 {
                        return Err(std::io::Error::new(
                            ErrorKind::TimedOut,
                            "broker stopped answering pings",
                        ));
                    }
                }
            }
            // Every sender is gone, so nothing will be said any more, the goodbye included.
            // Waiting on would return at once, every time.
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

/// Reads and discards whatever the broker sent — PINGRESPs, only — and notices a close.
/// Returns whether anything arrived, which is how a broker that has gone silent is noticed.
///
/// Unread bytes are not free: a PINGRESP every fifteen seconds fills the receive buffer over
/// days, the TCP window shuts, and the broker eventually blocks writing to a client that was
/// never listening. Draining is what keeps a publish-only client honest.
fn drain(stream: &mut TcpStream) -> std::io::Result<bool> {
    stream.set_read_timeout(Some(DRAIN_TIMEOUT))?;
    let mut scratch = [0u8; 64];
    let mut heard = false;
    loop {
        match stream.read(&mut scratch) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::ConnectionAborted,
                    "broker closed the connection",
                ));
            }
            Ok(_) => heard = true,
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                return Ok(heard);
            }
            Err(e) => return Err(e),
        }
    }
}

fn read_connack(stream: &mut TcpStream) -> std::io::Result<()> {
    // The connection is unusable until it arrives, which is why this one read waits far longer
    // than the drain does.
    stream.set_read_timeout(Some(Duration::from_secs(cfg::MQTT_CONNACK_TIMEOUT_S)))?;
    let mut packet = [0u8; stem::CONNACK_LEN];
    stream.read_exact(&mut packet)?;
    stem::read_connack(&packet).map_err(|refused| match refused {
        stem::Refused::NotConnack(header) => std::io::Error::new(
            ErrorKind::InvalidData,
            format!("expected CONNACK, got 0x{header:02x}"),
        ),
        stem::Refused::Code(code) => std::io::Error::new(
            ErrorKind::PermissionDenied,
            format!("broker refused the connection, code {code}"),
        ),
    })
}
