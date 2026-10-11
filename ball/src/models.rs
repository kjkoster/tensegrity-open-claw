use embassy_net::Ipv4Address;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    watch::{Sender, Watch},
};

/// Receivers the DMX value is fanned out to. A `Watch` rather than a `Signal` so that a
/// second consumer is a change to this number and not to the plumbing: a `Signal` has a
/// single waker, and a second waiter on it would starve.
pub const DMX_CONSUMERS: usize = 1;

/// The shared latest DMX value, written by the sACN listener and observed by each consumer.
pub type DmxWatch = Watch<CriticalSectionRawMutex, DmxValue, DMX_CONSUMERS>;
/// Producer handle into [`DmxWatch`] (held by the sACN listener).
pub type DmxSender = Sender<'static, CriticalSectionRawMutex, DmxValue, DMX_CONSUMERS>;

/// One ball's consecutive DMX slots, starting at its base address.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DmxValue([u8; DmxValue::LEN]);

impl DmxValue {
    pub const LEN: usize = 6;

    pub fn new(slots: [u8; Self::LEN]) -> Self {
        Self(slots)
    }
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
    password: &'static str,
    country: [u8; 2],
    channel: u8,
}

impl WifiConfig {
    /// Panics on credentials the radio would not accept. See [`DmxConfig::new`].
    pub const fn new(
        ssid: &'static str,
        password: &'static str,
        country: &'static str,
        channel: u8,
    ) -> Self {
        assert!(!ssid.is_empty() && ssid.len() <= 32, "SSID must be 1-32 bytes");
        // WPA2-PSK requires 8–63 ASCII characters; 64-byte form is a raw PMK hex string.
        assert!(password.len() >= 8 && password.len() <= 64, "passphrase must be 8-64 bytes");
        assert!(!contains_nul(ssid), "SSID must not contain a NUL");
        assert!(!contains_nul(password), "passphrase must not contain a NUL");
        assert!(country.len() == 2, "country code must be two letters");
        // Not 0, which is how an access point asks to pick its own channel: there would
        // be no telling which one to look on.
        assert!(channel >= 1 && channel <= 13, "WiFi channel must be 1-13");
        let country = country.as_bytes();
        Self { ssid, password, country: [country[0], country[1]], channel }
    }

    pub fn ssid(self) -> &'static str { self.ssid }
    pub fn password(self) -> &'static str { self.password }
    pub fn country(self) -> [u8; 2] { self.country }
    pub fn channel(self) -> u8 { self.channel }
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
