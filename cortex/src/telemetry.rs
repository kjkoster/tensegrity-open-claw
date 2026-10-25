//! The brain's connections to the broker: the health contract, and a place to publish from.
//!
//! Every daemon on the rig holds its own client and announces itself on `health/<service>`,
//! so that a dead process says so without anything else having to notice. This is the brain's
//! end of that, and the reason it cannot be delegated to Stem: only the process itself can
//! hand the broker a will, and only a will distinguishes "stopped" from "died".
//!
//! One process holds one client. A part that lives inside the brain — the face, the mouth —
//! lives and dies with it, so a will of its own would only repeat `health/brain`. It publishes
//! its own subtree, `face/…`, through the brain's connection, with a [`Publisher::part`], so
//! the tree still reads the way the rig is built rather than the way it is compiled.
//!
//! The packets and the health words are the `stem` crate's, shared with any firmware. The brain
//! publishes, never subscribes, always at QoS 0.
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
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddrV4, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long a drain waits for the broker. The client never subscribes and has nothing to wait
/// for, so this bounds how long the loop sits in a drain rather than how long it waits for an
/// answer.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(50);

/// One retained-or-not message on its way to the broker.
struct Message {
    scope: Scope,
    topic: String,
    payload: String,
    retain: bool,
}

/// Which root a message's topic hangs under.
enum Scope {
    /// The client's own subtree: `brain/rig`.
    Service,
    /// A part's subtree, on the client's connection: `face/estop`.
    Part(&'static str),
    /// A rig-wide root with the owner beneath it — the part, or the client when there is none:
    /// `sys/brain/board`, `dmx/face/address`. One subscription to the root then covers every
    /// owner on the rig.
    Rooted {
        root: &'static str,
        owner: Option<&'static str>,
    },
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
    outgoing: Option<SyncSender<Command>>,
    /// The part this publisher speaks for, whose name prefixes its topics in place of the
    /// client's; `None` for the client itself.
    part: Option<&'static str>,
}

impl Publisher {
    /// A publisher that goes nowhere, for a rig built without telemetry.
    pub fn disabled() -> Self {
        Self {
            outgoing: None,
            part: None,
        }
    }

    /// A publisher for a part inside this process, on this client's connection: its topics go
    /// under `<part>/` instead of the client's name.
    pub fn part(&self, part: &'static str) -> Publisher {
        Publisher {
            outgoing: self.outgoing.clone(),
            part: Some(part),
        }
    }

    /// Queues one message under this publisher's prefix — the client's name, or its part's.
    /// Never blocks, never fails loudly.
    pub fn publish(&self, topic: impl Into<String>, payload: impl Into<String>, retain: bool) {
        let scope = self.part.map_or(Scope::Service, Scope::Part);
        self.send(scope, topic.into(), payload.into(), retain);
    }

    /// Queues one fact about what this client runs on, retained, under `sys/<owner>/`.
    pub fn publish_sys(&self, leaf: impl Into<String>, payload: impl Into<String>) {
        self.send_rooted("sys", leaf.into(), payload.into());
    }

    /// Queues one fact about where this client or part sits on the universe, retained, under
    /// `dmx/<owner>/`.
    pub fn publish_dmx(&self, leaf: impl Into<String>, payload: impl Into<String>) {
        self.send_rooted("dmx", leaf.into(), payload.into());
    }

    fn send_rooted(&self, root: &'static str, leaf: String, payload: String) {
        let scope = Scope::Rooted {
            root,
            owner: self.part,
        };
        self.send(scope, leaf, payload, true);
    }

    fn send(&self, scope: Scope, topic: String, payload: String, retain: bool) {
        let Some(outgoing) = &self.outgoing else {
            return;
        };
        let message = Command::Publish(Message {
            scope,
            topic,
            payload,
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
        let mut pending = Some(Command::Farewell);
        while let Some(command) = pending.take() {
            match self.outgoing.try_send(command) {
                Ok(()) => return true,
                // The queue is full because the thread is busy or the broker is slow. Hand the
                // command back and try again until the deadline, rather than dropping the
                // goodbye over a queue that was about to drain anyway.
                Err(TrySendError::Full(returned)) if Instant::now() < expires => {
                    pending = Some(returned);
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return false,
            }
        }
        false
    }
}

/// Says every goodbye and waits, briefly, for all of them to actually leave.
///
/// Waiting is the point. The caller is about to `exit`, and a goodbye still sitting in a
/// channel is a goodbye the broker never hears — at which point the will fires and an orderly
/// stop is recorded as a crash. All of them share one deadline, so a second client does not
/// double the shutdown, and it is bounded, because a wedged broker must not be able to hold up
/// a shutdown: past it the will fires and `gone` is, at worst, pessimistic about a rig that
/// really is going away.
pub fn farewell(farewells: &[Farewell]) {
    let expires = Instant::now() + Duration::from_millis(cfg::MQTT_FAREWELL_MS);
    let asked: Vec<&Farewell> = farewells.iter().filter(|f| f.ask(expires)).collect();
    while !asked.iter().all(|f| f.said.load(Ordering::Acquire)) && Instant::now() < expires {
        thread::sleep(Duration::from_millis(2));
    }
}

/// Spawns a client and returns the handles the rest of the process speaks through.
///
/// `service` names the health topic, is the client ID, and prefixes every other topic this
/// client publishes, so the tree says which part of the rig asserted a thing.
pub fn spawn(service: &'static str) -> (JoinHandle<()>, Publisher, Farewell) {
    let (outgoing, incoming) = sync_channel(cfg::MQTT_QUEUE_DEPTH);
    let said = Arc::new(AtomicBool::new(false));

    let thread_said = said.clone();
    let handle = thread::Builder::new()
        .name(format!("telemetry-{service}"))
        .spawn(move || {
            // Every retained topic this client has published, replayed on each connect.
            //
            // Retained state is a promise that a late subscriber sees the live picture, and a
            // broker restart breaks it: the broker keeps nothing across one, so whatever was
            // published once at startup would otherwise be gone for good.
            let mut retained = BTreeMap::new();
            let mut backoff_s = 1u64;
            loop {
                match session(service, &incoming, &thread_said, &mut retained, &mut backoff_s) {
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
        handle,
        Publisher {
            outgoing: Some(outgoing.clone()),
            part: None,
        },
        Farewell { outgoing, said },
    )
}

/// Connects, announces, and pumps the queue. Returns only once the goodbye has been said, or
/// once nobody is left to say anything.
fn session(
    service: &str,
    incoming: &Receiver<Command>,
    said: &AtomicBool,
    retained: &mut BTreeMap<String, String>,
    backoff_s: &mut u64,
) -> std::io::Result<()> {
    let address = SocketAddrV4::new(cfg::MQTT_ADDRESS, stem::PORT);
    let mut stream = TcpStream::connect(address)?;
    // Nagle would hold a small publish back waiting for company that never comes; these are
    // one-datagram-sized messages seconds apart.
    stream.set_nodelay(true)?;

    stream.write_all(&stem::connect(service))?;
    read_connack(&mut stream)?;
    eprintln!("telemetry: {service}: connected to {address}");
    // Only once the broker has accepted us: a broker that keeps refusing must keep backing off,
    // and one that was simply restarted must not be waited on as if it had.
    *backoff_s = 1;

    let health = stem::health_topic(service);
    stream.write_all(&stem::publish(&health, stem::CONNECTED, true))?;
    for (topic, payload) in retained.iter() {
        stream.write_all(&stem::publish(topic, payload, true))?;
    }

    // Half the keepalive: the broker drops a client at 1.5× the interval it was given, so
    // pinging twice per interval means one lost ping is not a disconnection.
    let tick = Duration::from_secs(u64::from(stem::KEEPALIVE_S) / 2);
    loop {
        match incoming.recv_timeout(tick) {
            Ok(Command::Publish(message)) => {
                let topic = match message.scope {
                    Scope::Service => format!("{service}/{}", message.topic),
                    Scope::Part(part) => format!("{part}/{}", message.topic),
                    Scope::Rooted { root, owner } => {
                        stem::rooted_topic(root, owner.unwrap_or(service), &message.topic)
                    }
                };
                let packet = stem::publish(&topic, &message.payload, message.retain);
                // Recorded before the write, so a value whose write is what failed is the one
                // the next connect replays.
                if message.retain {
                    retained.insert(topic, message.payload);
                }
                stream.write_all(&packet)?;
            }
            Ok(Command::Farewell) => {
                // On this connection, and followed by a clean DISCONNECT. A goodbye sent over
                // a second connection would be overwritten moments later by the will firing
                // on this one — the broker publishes it whenever a connection drops without a
                // DISCONNECT, and an exiting process drops all of them.
                stream.write_all(&stem::publish(&health, stem::DISCONNECTED, true))?;
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
                drain(&mut stream)?;
            }
            // Every sender is gone, so nothing will be said any more, the goodbye included.
            // Waiting on would return at once, every time.
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

/// Reads and discards whatever the broker sent — PINGRESPs, only — and notices a close.
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
