use ball_dmx::STROBE_STEP_MS;
use embassy_net::Ipv4Address;
use embassy_time::Duration;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    watch::{Sender, Watch},
};

/// Receivers the DMX value is fanned out to. A `Watch` rather than a `Signal` so that a
/// second consumer is a change to this number and not to the plumbing: a `Signal` has a
/// single waker, and a second waiter on it would starve.
pub const DMX_CONSUMERS: usize = 1;

/// The shared latest DMX state, written by the sACN listener and observed by each consumer.
pub type DmxWatch = Watch<CriticalSectionRawMutex, DmxState, DMX_CONSUMERS>;
/// Producer handle into [`DmxWatch`] (held by the sACN listener).
pub type DmxSender = Sender<'static, CriticalSectionRawMutex, DmxState, DMX_CONSUMERS>;

/// What the universe last said, or that it has stopped saying anything.
///
/// Losing the universe is a state of its own and not a value: a value cannot tell a sender
/// that holds a channel steady from one that is gone, and the difference is whether the
/// last thing it asked for still stands.
#[derive(Clone, Copy, PartialEq)]
pub enum DmxState {
    Live(DmxValue),
    Lost,
}

/// Where each channel sits among a ball's slots and which bands it declares, generated from
/// `fixtures/ball.qxf` under the names the brain's `Ball` uses.
#[allow(
    dead_code,
    reason = "it names everything the definition declares, and the firmware reads only what it acts on"
)]
mod layout {
    use super::Band;

    include!(concat!(env!("OUT_DIR"), "/layout.rs"));
}

/// One band of a channel's range, as the definition declares it.
pub struct Band {
    min: u8,
    max: u8,
}

impl Band {
    const fn new(min: u8, max: u8) -> Self {
        Self { min, max }
    }

    fn contains(&self, value: u8) -> bool {
        (self.min..=self.max).contains(&value)
    }
}

/// One ball's consecutive DMX slots, starting at its base address.
#[derive(Clone, Copy, PartialEq)]
pub struct DmxValue([u8; DmxValue::LEN]);

impl DmxValue {
    pub const LEN: usize = layout::LEN;

    pub fn new(slots: [u8; Self::LEN]) -> Self {
        Self(slots)
    }

    /// The level of the onboard status LED.
    pub fn onboard_led(self) -> u8 {
        self.0[layout::INTENSITY]
    }

    /// Whether the motor is asked to run.
    pub fn motor(self) -> bool {
        layout::MOTOR_ON.contains(self.0[layout::MOTOR])
    }

    /// What the power LEDs are asked to do. A pattern value in no band the definition
    /// declares reads as direct, which is what a console that knows nothing of patterns sends.
    pub fn look(self) -> Look {
        let pattern = self.0[layout::PATTERN];
        let pattern = if layout::PATTERN_STROBE.contains(pattern) {
            Pattern::Strobe
        } else if layout::PATTERN_PULSE.contains(pattern) {
            Pattern::Pulse
        } else {
            Pattern::Direct
        };
        Look {
            pattern,
            white: self.0[layout::WHITE],
            blue: self.0[layout::BLUE],
            strobe_on: strobe_length(self.0[layout::STROBE_ON]),
            strobe_off: strobe_length(self.0[layout::STROBE_OFF]),
        }
    }
}

fn strobe_length(value: u8) -> Duration {
    Duration::from_millis(u64::from(value) * u64::from(STROBE_STEP_MS))
}

/// How the power LEDs read their levels.
#[derive(Clone, Copy, PartialEq)]
pub enum Pattern {
    /// The levels are the brightness, held.
    Direct,
    /// The levels are the peak of a repeating pulse: a rise, a hold, and a snap to dark.
    Pulse,
    /// The levels are the flashes of a square wave, with the lengths off the wire.
    Strobe,
}

/// Everything the power LEDs render from.
#[derive(Clone, Copy, PartialEq)]
pub struct Look {
    pub pattern: Pattern,
    pub white: u8,
    pub blue: u8,
    pub strobe_on: Duration,
    pub strobe_off: Duration,
}

impl Look {
    pub const DARK: Look = Look {
        pattern: Pattern::Direct,
        white: 0,
        blue: 0,
        strobe_on: Duration::from_millis(0),
        strobe_off: Duration::from_millis(0),
    };
}

#[derive(Clone, Copy)]
pub struct DmxConfig {
    address: u16,
    universe: u16,
    sacn_port: u16,
}

impl DmxConfig {
    /// Panics on a value outside the protocol's range: the configuration is compiled in,
    /// so a bad one is a mistake in the source and never something to recover from.
    pub const fn new(address: u16, universe: u16, sacn_port: u16) -> Self {
        assert!(address >= 1 && address <= 512, "DMX address must be 1-512");
        // E1.31 §9.1.1: universe 0 and 64000–65535 are reserved; valid range is 1–63999.
        assert!(universe >= 1 && universe <= 63999, "sACN universe must be 1-63999");
        assert!(sacn_port != 0, "sACN port must not be 0");
        Self { address, universe, sacn_port }
    }

    pub fn address(self) -> u16 { self.address }
    pub fn universe(self) -> u16 { self.universe }
    pub fn sacn_port(self) -> u16 { self.sacn_port }

    pub fn multicast_address(self) -> Ipv4Address {
        Ipv4Address::new(239, 255, (self.universe >> 8) as u8, self.universe as u8)
    }
}

#[derive(Clone, Copy)]
pub struct WifiConfig {
    ssid: &'static str,
    bssid: [u8; 6],
    psk: &'static str,
    country: [u8; 2],
    channel: u8,
}

impl WifiConfig {
    /// Panics on credentials the radio would not accept. See [`DmxConfig::new`].
    pub const fn new(
        ssid: &'static str,
        bssid: [u8; 6],
        psk: &'static str,
        country: &'static str,
        channel: u8,
    ) -> Self {
        assert!(!ssid.is_empty() && ssid.len() <= 32, "SSID must be 1-32 bytes");
        // The driver takes exactly 64 hex digits as the key itself; anything else it would
        // treat as a passphrase, and derive a different key from.
        assert!(is_hex_key(psk), "the WPA2 key must be 64 hex digits");
        assert!(!contains_nul(ssid), "SSID must not contain a NUL");
        assert!(country.len() == 2, "country code must be two letters");
        // Not 0, which is how an access point asks to pick its own channel: there would
        // be no telling which one to look on.
        assert!(channel >= 1 && channel <= 13, "WiFi channel must be 1-13");
        let country = country.as_bytes();
        Self { ssid, bssid, psk, country: [country[0], country[1]], channel }
    }

    pub fn ssid(self) -> &'static str { self.ssid }
    pub fn bssid(self) -> [u8; 6] { self.bssid }
    /// The WPA2 pre-shared key, as 64 hex digits.
    pub fn psk(self) -> &'static str { self.psk }
    pub fn country(self) -> [u8; 2] { self.country }
    pub fn channel(self) -> u8 { self.channel }
}

// A loop because iterators are not usable in a `const fn`.
const fn is_hex_key(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return false;
    }
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_hexdigit() {
            return false;
        }
        i += 1;
    }
    true
}

// A loop because iterators are not usable in a `const fn`.
const fn contains_nul(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0 {
            return true;
        }
        i += 1;
    }
    false
}
