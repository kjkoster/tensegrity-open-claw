//! The OSC every device built on cortex reports, from the shared files in `fixtures/` it
//! implements: `sys`, `wifi`, `health` and `dmx`, each for a device by its name. A program that
//! is several devices, as the brain is the face as well, reports the blocks for each, the
//! readings taken once for all of them.
//!
//! Every block's addresses are generated from its file, `{device}` the argument, so an address
//! the file does not have does not build. A message here is an address and its one value, as
//! [`osc::bundle`] takes them; how they go out, sent or recorded, is the program's.

use crate::osc::{self, Value};
use crate::system;
use std::net::{SocketAddrV4, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

include!(concat!(env!("OUT_DIR"), "/shared_sys.rs"));
include!(concat!(env!("OUT_DIR"), "/shared_wifi.rs"));
include!(concat!(env!("OUT_DIR"), "/shared_health.rs"));
include!(concat!(env!("OUT_DIR"), "/shared_dmx.rs"));

/// One message: an address and its value.
pub type Message = (String, Value);

/// When a block's leaves go: once, on connect, or every second, as its file has them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum When {
    Connect,
    Second,
}

/// The `sys`, `wifi` and `health` leaves that go at `when`, for each of `devices`, which run
/// on this Pi: each reading taken once, and sent under every device's name.
///
/// `/health/<device>/uptime_s` is the device's heartbeat; the rest of that block is Memory's,
/// which it synthesises from these. chaos-pi is the access point and hears none, so its
/// `/wifi/<device>/rssi` is 0, as the file has it for a station not associated.
pub fn shared(devices: &[&str], when: When) -> Vec<Message> {
    let readings: Vec<(fn(&str) -> String, Value)> = match when {
        When::Connect => vec![
            (sys::board, Value::Text(system::board())),
            (sys::cores, Value::Int(system::cores() as i32)),
            (sys::triplet, Value::Text(system::TRIPLET.to_owned())),
            (sys::cpu_freq_mhz, Value::Int(system::cpu_freq_mhz() as i32)),
            (wifi::mac, Value::Text(system::wifi_mac())),
            (wifi::ip, Value::Text(system::wifi_ip())),
        ],
        When::Second => vec![
            (sys::mem_avail_kb, Value::Int(system::mem_avail_kb() as i32)),
            (wifi::rssi, Value::Int(system::wifi_rssi())),
            (health::uptime_s, Value::Int(system::uptime_s() as i32)),
        ],
    };
    devices
        .iter()
        .flat_map(|device| {
            readings
                .iter()
                .map(move |(address, value)| (address(device), value.clone()))
        })
        .collect()
}

/// A program's own leaves, at their generated addresses, as messages.
pub fn messages<const N: usize>(leaves: [(&str, Value); N]) -> Vec<Message> {
    leaves
        .into_iter()
        .map(|(address, value)| (address.to_owned(), value))
        .collect()
}

/// `/dmx/<device>/…`: where the device sits on the universe, sent on connect.
pub fn dmx(device: &str, universe: u16, address: u16) -> Vec<Message> {
    vec![
        (dmx::universe(device), Value::Int(i32::from(universe))),
        (dmx::address(device), Value::Int(i32::from(address))),
    ]
}

/// Sends bundles to a fixed set of recipients, from a socket of its own.
pub struct Sender {
    socket: UdpSocket,
    recipients: &'static [SocketAddrV4],
}

impl Sender {
    pub fn new(recipients: &'static [SocketAddrV4]) -> Sender {
        Sender {
            socket: UdpSocket::bind("0.0.0.0:0").expect("report: socket bind failed"),
            recipients,
        }
    }

    /// Sends `messages` as one bundle to every recipient. A recipient that is down is not the
    /// sender's business: the bundle after this one goes all the same, so nothing is logged.
    pub fn send(&self, messages: &[Message]) {
        let packet = osc::bundle(messages);
        for recipient in self.recipients {
            let _ = self.socket.send_to(&packet, recipient);
        }
    }
}

/// Calls `tick` every `period` for ever, with the count of ticks before it, on the calling
/// thread. Due times step by the period, so the rate does not drift with the time a pass
/// takes; after a stall, again from now rather than in a burst.
pub fn every(period: Duration, mut tick: impl FnMut(u64)) -> ! {
    let mut due = Instant::now();
    for ticks in 0.. {
        tick(ticks);
        due += period;
        let now = Instant::now();
        if due <= now {
            due = now + period;
        }
        thread::sleep(due - now);
    }
    unreachable!("a u64 of ticks does not run out")
}
