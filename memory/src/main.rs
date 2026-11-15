//! Memory: the OSC every device sends to memory-pi, recorded into GreptimeDB and relayed onto
//! the broker, so a show can be read back afterwards and watched live from anywhere a
//! subscriber can reach Mosquitto. Its own heartbeat and memory-pi's health go the same way, as
//! `fixtures/memory.oscquery.json` has them.
//!
//! Every value is relayed as it arrives, at its address without the leading slash, not
//! retained: OSC has no retention, and the relay does not pretend otherwise. It is recorded
//! only once the clock has synchronised since boot, because before that it would be filed under
//! the stale time the Pi booted with.
//!
//! An address no fixture file has is recorded all the same and logged once: a device flashed
//! from another commit than Memory was built from still gets recorded.

mod board;
mod greptime;
mod osc;

include!(concat!(env!("OUT_DIR"), "/network.rs"));

mod fixtures {
    include!(concat!(env!("OUT_DIR"), "/fixtures.rs"));
}

use cortex::telemetry;
use osc::Value;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::collections::{BTreeSet, HashSet};
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
/// The rate of every state in `fixtures/memory.oscquery.json`.
const STATE_PERIOD: Duration = Duration::from_secs(1);

struct Memory {
    relay: telemetry::Publisher,
    backlog: greptime::Backlog,
    synchronised: AtomicBool,
    known: HashSet<&'static str>,
    /// What has been logged, so each is logged once.
    logged: Mutex<HashSet<String>>,
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
        self.relay
            .relay(address.trim_start_matches('/'), value.to_string());
        if !self.known.contains(address) {
            self.once(format!(
                "{address} is in no fixture file, recorded all the same"
            ));
        }
        if self.synchronised() {
            let at_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the clock is before 1970")
                .as_millis() as u64;
            self.backlog.push(address, &value, at_ms);
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
    let memory = Arc::new(Memory {
        relay,
        backlog,
        synchronised: AtomicBool::new(false),
        known: fixtures::KNOWN
            .iter()
            .flat_map(|paths| paths.iter().copied())
            .collect(),
        logged: Mutex::new(HashSet::new()),
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
/// synchronised, which is when it can be recorded; the rest every second.
fn report(memory: &Memory) {
    let started = Instant::now();
    let mut connected = false;
    let mut warned = BTreeSet::new();
    loop {
        if !connected && memory.synchronised() {
            for (address, value) in board::connect() {
                memory.record(address, value);
            }
            connected = true;
        }
        memory.record(
            fixtures::memory::MemoryUptime::PATH,
            Value::Int(started.elapsed().as_secs() as i32),
        );
        for (address, value) in board::state(&mut warned) {
            memory.record(address, value);
        }
        thread::sleep(STATE_PERIOD);
    }
}
