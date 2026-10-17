//! The ball's connection to the broker: `health/ball`, and everything else under `ball/`.
//!
//! The broker is Stem's, on the Pi, reached over the access point. Its address is the gateway
//! the DHCP lease names, so the firmware carries no second copy of the Pi's address.
//!
//! Every topic is retained. The ball has nothing to say that is an event rather than a state,
//! and whoever subscribes after the interesting moment should still see the ball.
//!
//! Publishing never waits and never drops what matters. [`publish`] overwrites a topic's
//! pending value rather than queueing behind it, so the newest reading is always the one that
//! goes out, the pending set is bounded by the number of topics, and a ball that spends minutes
//! looking for the access point still has its whole state to send the moment it is back.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use core::cell::RefCell;
use embassy_futures::select::{Either, select};
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_sync::{blocking_mutex::Mutex, blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer, with_deadline, with_timeout};
use embedded_io_async::{Read, ReadExactError, Write};
use rtt_target::rprintln;

const SERVICE: &str = "ball";

/// The broker answers within milliseconds over the access point. Past these the connection
/// is taken to be dead rather than slow.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);
/// Half the keepalive: the broker drops a client at 1.5× the interval it was given, so pinging
/// twice per interval means one lost ping is not a disconnection.
const PING_EVERY: Duration = Duration::from_secs(rig_mqtt::KEEPALIVE_S as u64 / 2);
/// Reconnect backoff, matching the brain's.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);
/// How long a farewell waits for the goodbye to leave. Bounded, because a ball going to sleep
/// must not be kept awake by a broker that has gone quiet: past it the will fires instead.
const FAREWELL_TIMEOUT: Duration = Duration::from_millis(500);

/// What has been published and not yet sent, by topic.
static PENDING: Mutex<CriticalSectionRawMutex, RefCell<BTreeMap<&'static str, String>>> =
    Mutex::new(RefCell::new(BTreeMap::new()));
static PUBLISHED: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static FAREWELL: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static SAID: Signal<CriticalSectionRawMutex, ()> = Signal::new();

enum Error {
    NoGateway,
    Timeout,
    Connect(embassy_net::tcp::ConnectError),
    Socket(embassy_net::tcp::Error),
    Closed,
    Refused(rig_mqtt::Refused),
    NotPingresp([u8; 2]),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NoGateway => write!(f, "the lease names no gateway"),
            Error::Timeout => write!(f, "the broker did not answer"),
            Error::Connect(e) => write!(f, "cannot connect ({:?})", e),
            Error::Socket(e) => write!(f, "socket error ({:?})", e),
            Error::Closed => write!(f, "the broker closed the connection"),
            Error::Refused(refused) => write!(f, "refused ({:?})", refused),
            Error::NotPingresp(answer) => write!(f, "answered a ping with {:02x?}", answer),
        }
    }
}

impl From<embassy_net::tcp::Error> for Error {
    fn from(e: embassy_net::tcp::Error) -> Self {
        Error::Socket(e)
    }
}

impl From<ReadExactError<embassy_net::tcp::Error>> for Error {
    fn from(e: ReadExactError<embassy_net::tcp::Error>) -> Self {
        match e {
            ReadExactError::UnexpectedEof => Error::Closed,
            ReadExactError::Other(e) => Error::Socket(e),
        }
    }
}

/// Sets `ball/<topic>` to `payload`. Never waits; callable before the network is up, and from
/// any task.
pub fn publish(topic: &'static str, payload: impl Into<String>) {
    let payload = payload.into();
    PENDING.lock(|pending| pending.borrow_mut().insert(topic, payload));
    PUBLISHED.signal(());
}

/// Says `disconnected` and closes the connection cleanly, for a ball about to sleep, so the
/// sleep reads as a deliberate stop and not as `gone`. Returns once the goodbye has left, or
/// once it is clear that it will not.
#[expect(dead_code, reason = "the deep sleep that says this is not built yet")]
pub async fn farewell() {
    SAID.reset();
    FAREWELL.signal(());
    let _ = with_timeout(FAREWELL_TIMEOUT, SAID.wait()).await;
}

/// Holds the ball's connection to the broker, for as long as the ball runs.
#[embassy_executor::task]
pub async fn run(stack: Stack<'static>) -> ! {
    // Everything ever published, replayed on each connect. The broker keeps nothing across a
    // restart, and the Pi restarting under a ball that never lost power is the ordinary case.
    let mut latest: BTreeMap<&'static str, String> = BTreeMap::new();
    let mut rx_buffer = [0u8; 64];
    let mut tx_buffer = [0u8; 256];
    let mut backoff = RETRY_FIRST;
    loop {
        stack.wait_config_up().await;

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        // Nagle would hold a reading back waiting for company. And a timeout, because over
        // WiFi a link that is gone often closes nothing: a write into a dead connection would
        // otherwise wait for buffer space forever. Pings answered every fifteen seconds keep a
        // live connection well inside it.
        socket.set_nagle_enabled(false);
        socket.set_timeout(Some(Duration::from_secs(u64::from(rig_mqtt::KEEPALIVE_S))));

        match session(stack, &mut socket, &mut latest, &mut backoff).await {
            Ok(()) => {
                SAID.signal(());
                rprintln!("telemetry: said goodbye");
                // Nothing to reconnect for: the ball is on its way to sleep.
                core::future::pending::<()>().await;
            }
            Err(e) => {
                rprintln!("telemetry: {}, retrying in {} s", e, backoff.as_secs());
            }
        }
        // A reset rather than a FIN, and flushed so it is actually sent before the socket's
        // buffers are reused for the next one.
        socket.abort();
        let _ = socket.flush().await;
        drop(socket);

        Timer::after(backoff).await;
        backoff = (backoff * 2).min(RETRY_MAX);
    }
}

/// Connects, announces, and sends whatever is published. Returns only once the goodbye has
/// been said.
async fn session(
    stack: Stack<'static>,
    socket: &mut TcpSocket<'_>,
    latest: &mut BTreeMap<&'static str, String>,
    backoff: &mut Duration,
) -> Result<(), Error> {
    let network = stack.config_v4().ok_or(Error::NoGateway)?;
    let broker = network.gateway.ok_or(Error::NoGateway)?;
    with_timeout(CONNECT_TIMEOUT, socket.connect((broker, rig_mqtt::PORT)))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(Error::Connect)?;

    socket.write_all(&rig_mqtt::connect(SERVICE)).await?;
    let mut connack = [0u8; rig_mqtt::CONNACK_LEN];
    with_timeout(ANSWER_TIMEOUT, socket.read_exact(&mut connack))
        .await
        .map_err(|_| Error::Timeout)??;
    rig_mqtt::read_connack(&connack).map_err(Error::Refused)?;
    rprintln!("telemetry: connected to {}", broker);
    // Only once the broker has accepted us, so a broker that keeps refusing keeps backing off.
    *backoff = RETRY_FIRST;

    // What changes from one connection to the next, so it is told on every one.
    publish("ip", format!("{}", network.address.address()));
    publish("uptime_s",format!("{}", Instant::now().as_secs()));

    let health = rig_mqtt::health_topic(SERVICE);
    socket.write_all(&rig_mqtt::publish(&health, rig_mqtt::CONNECTED, true)).await?;
    take_pending(latest);
    for (topic, payload) in latest.iter() {
        send(socket, topic, payload).await?;
    }

    let mut next_ping = Instant::now() + PING_EVERY;
    loop {
        match with_deadline(next_ping, select(PUBLISHED.wait(), FAREWELL.wait())).await {
            Ok(Either::First(())) => {
                for (topic, payload) in take_pending(latest) {
                    send(socket, topic, &payload).await?;
                }
            }
            Ok(Either::Second(())) => {
                // On this connection, and followed by a clean DISCONNECT: that is what tells the
                // broker not to publish the will.
                socket
                    .write_all(&rig_mqtt::publish(&health, rig_mqtt::DISCONNECTED, true))
                    .await?;
                socket.write_all(&rig_mqtt::DISCONNECT).await?;
                socket.flush().await?;
                socket.close();
                return Ok(());
            }
            // Read back, unlike on the brain: over WiFi the answer is the only evidence that the
            // broker is still there. It also keeps the receive buffer from filling.
            Err(_) => {
                socket.write_all(&rig_mqtt::PINGREQ).await?;
                let mut answer = [0u8; 2];
                with_timeout(ANSWER_TIMEOUT, socket.read_exact(&mut answer))
                    .await
                    .map_err(|_| Error::Timeout)??;
                if answer != rig_mqtt::PINGRESP {
                    return Err(Error::NotPingresp(answer));
                }
                next_ping += PING_EVERY;
            }
        }
    }
}

/// Moves what is pending into `latest`, returning it so it can be sent.
fn take_pending(latest: &mut BTreeMap<&'static str, String>) -> BTreeMap<&'static str, String> {
    let taken = PENDING.lock(|pending| core::mem::take(&mut *pending.borrow_mut()));
    for (topic, payload) in taken.iter() {
        latest.insert(topic, payload.clone());
    }
    taken
}

async fn send(socket: &mut TcpSocket<'_>, topic: &str, payload: &str) -> Result<(), Error> {
    let topic = format!("{SERVICE}/{topic}");
    socket.write_all(&rig_mqtt::publish(&topic, payload, true)).await?;
    Ok(())
}
