//! Memory: the OSC every device sends to it on chaos-pi, recorded into GreptimeDB and relayed
//! onto the broker, so a show can be read back afterwards and watched live from anywhere a
//! subscriber can reach Mosquitto. Its own heartbeat and the Pi's health go the same way, as
//! `fixtures/memory.oscquery.json` has them, and so does every device's health, synthesised
//! from its heartbeat as `fixtures/health.oscquery.json` has it.
//!
//! Every value is relayed as it arrives, at its address without the leading slash, retained,
//! so a subscriber connecting later sees the last of each, the values sent once on connect
//! included; until the broker restarts, which keeps nothing. It is recorded only once the clock has synchronised since boot, because before that it would be filed under
//! the stale time the Pi booted with.
//!
//! An address no fixture file has is recorded all the same and logged once: a device flashed
//! from another commit than Memory was built from still gets recorded.

mod board;
mod greptime;
mod telemetry;

include!(concat!(env!("OUT_DIR"), "/network.rs"));

mod fixtures {
    include!(concat!(env!("OUT_DIR"), "/fixtures.rs"));
}

use cortex::osc::{self, Value};
use cortex::report::{self, health};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::ErrorKind;
use std::net::UdpSocket;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The largest UDP datagram, so none is ever cut short.
const MAX_PACKET_LEN: usize = 65_536;
/// The longest wait between attempts to bind, while the access point's address is not up.
const BIND_RETRY_MAX_S: u64 = 30;
/// timesyncd creates this once it has synchronised the clock.
const SYNCHRONIZED: &str = "/run/systemd/timesync/synchronized";
/// The rate of every state in `fixtures/memory.oscquery.json`, as the file gives it.
const STATE_HZ: f64 = fixtures::memory::SysMemoryTemperature::RATE_HZ;
/// How long a device's heartbeat may be missing before it counts as lost.
const HEALTH_TIMEOUT: Duration = Duration::from_millis(2500);

struct Memory {
    relay: telemetry::Publisher,
    backlog: greptime::Backlog,
    synchronised: AtomicBool,
    known: HashSet<&'static str>,
    /// What has been logged, so each is logged once.
    logged: Mutex<HashSet<String>>,
    /// Each device whose file has a heartbeat, `/health/<device>/uptime_s`, and when its last
    /// arrived, by the monotonic clock; none yet since Memory started.
    heartbeats: Mutex<BTreeMap<String, Option<Instant>>>,
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is before 1970")
        .as_millis() as u64
}

/// `at` by the wall clock, as RFC 3339 in UTC, to the millisecond.
fn rfc3339(at: Instant) -> String {
    let ms = unix_ms().saturating_sub(at.elapsed().as_millis() as u64);
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .expect("a time since 1970 in range")
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Memory {
    fn synchronised(&self) -> bool {
        if self.synchronised.load(Ordering::Relaxed) {
            return true;
        }
        if !Path::new(SYNCHRONIZED).exists() {
            return false;
        }
        if !self.synchronised.swap(true, Ordering::Relaxed) {
            eprintln!("memory: the clock has synchronised, recording");
        }
        true
    }

    fn record(&self, address: &str, value: Value) {
        if let Some(device) = health::uptime_s_device(address) {
            self.heartbeats
                .lock()
                .unwrap()
                .insert(device.to_owned(), Some(Instant::now()));
        }
        self.relay
            .relay(address.trim_start_matches('/'), value.to_string());
        if !self.known.contains(address) {
            self.once(format!(
                "{address} is in no fixture file, recorded all the same"
            ));
        }
        if self.synchronised() {
            self.backlog.push(address, &value, unix_ms());
        }
    }

    /// Each device's health, synthesised from its heartbeats: when the last arrived, once one
    /// has, and whether it is still arriving.
    fn report_health(&self) {
        let heartbeats = self.heartbeats.lock().unwrap().clone();
        for (device, seen) in heartbeats {
            let connected = seen.is_some_and(|at| at.elapsed() < HEALTH_TIMEOUT);
            // Only by a synchronised clock: before it, the wall clock is the stale time the Pi
            // booted with, and the arrival is turned into wall time only here, so a heartbeat
            // that came before the clock synchronised is dated right once it has.
            if let Some(at) = seen
                && self.synchronised()
            {
                self.record(&health::last_seen(&device), Value::Text(rfc3339(at)));
            }
            let state = if connected { "connected" } else { "lost" };
            self.record(&health::state(&device), Value::Text(state.into()));
        }
    }

    fn once(&self, message: String) {
        if self.logged.lock().unwrap().insert(message.clone()) {
            eprintln!("memory: {message}");
        }
    }
}

fn main() {
    let (relay, farewell) = telemetry::spawn("memory", network::MQTT);
    let backlog = greptime::Backlog::default();
    greptime::spawn(backlog.clone());
    let known: HashSet<&'static str> = fixtures::KNOWN
        .iter()
        .flat_map(|paths| paths.iter().copied())
        .collect();
    let heartbeats = known
        .iter()
        .filter_map(|address| health::uptime_s_device(address))
        .map(|device| (device.to_owned(), None))
        .collect();
    let memory = Arc::new(Memory {
        relay,
        backlog,
        synchronised: AtomicBool::new(false),
        known,
        logged: Mutex::new(HashSet::new()),
        heartbeats: Mutex::new(heartbeats),
    });

    let receiving = memory.clone();
    thread::Builder::new()
        .name("osc".into())
        .spawn(move || receive(&receiving))
        .expect("failed to spawn the osc thread");

    let reporting = memory.clone();
    thread::Builder::new()
        .name("report".into())
        .spawn(move || report(&reporting))
        .expect("failed to spawn the report thread");

    // systemd stops Memory with SIGTERM, and one started by hand is stopped with SIGINT. The
    // goodbye goes to the broker first, so a stop is not recorded as a crash.
    let mut signals = Signals::new([SIGINT, SIGTERM]).expect("signal registration failed");
    signals.forever().next();
    eprintln!("memory: shutting down");
    farewell.say();
}

/// Binds the OSC port, waiting for its address to come up, and receives for ever.
fn receive(memory: &Memory) {
    let bind = network::OSC_BIND;
    let mut backoff_s = 1u64;
    loop {
        // The address not being there yet is the access point not being up, which the backoff
        // waits out. Any other bind failure, such as a second Memory holding the port, will
        // not go away by waiting.
        match UdpSocket::bind(bind) {
            Ok(socket) => {
                eprintln!("memory: listening for OSC on {bind}");
                backoff_s = 1;
                let mut buffer = vec![0u8; MAX_PACKET_LEN];
                loop {
                    let (len, sender) = match socket.recv_from(&mut buffer) {
                        Ok(received) => received,
                        Err(e) => {
                            eprintln!("memory: receive failed: {e}");
                            break;
                        }
                    };
                    match osc::decode(&buffer[..len]) {
                        Ok(messages) => {
                            for (address, mut values) in messages {
                                if values.len() == 1 {
                                    memory.record(&address, values.remove(0));
                                } else {
                                    memory.once(format!(
                                        "{address} carries {} values, not one, and is not recorded",
                                        values.len()
                                    ));
                                }
                            }
                        }
                        Err(why) => memory.once(format!("a packet from {sender} refused: {why}")),
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::AddrNotAvailable => {
                eprintln!("memory: bind {bind}: {e}, again in {backoff_s}s");
            }
            Err(e) => panic!("memory: bind {bind}: {e}"),
        }
        thread::sleep(Duration::from_secs(backoff_s));
        backoff_s = (backoff_s * 2).min(BIND_RETRY_MAX_S);
    }
}

/// Memory's own OSC, recorded as every device's is, but handed over here rather than sent, so
/// it is recorded while the WiFi is down. What does not change goes once the clock has
/// synchronised, which is when it can be recorded; the rest every second, and every device's
/// health after it.
fn report(memory: &Memory) {
    let mut connected = false;
    let mut warned = BTreeSet::new();
    report::every(Duration::from_secs_f64(1.0 / STATE_HZ), |_| {
        if !connected && memory.synchronised() {
            for (address, value) in board::connect() {
                memory.record(&address, value);
            }
            connected = true;
        }
        for (address, value) in board::state(&mut warned) {
            memory.record(&address, value);
        }
        memory.report_health();
    })
}
