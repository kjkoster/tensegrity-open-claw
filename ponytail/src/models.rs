extern crate alloc;

use alloc::string::String;
use embassy_net::Ipv4Address;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    watch::{Receiver, Sender, Watch},
};

pub const DMX_MAXVALUE: u8 = 255;

/// Independent consumers that observe the latest DMX value in parallel: the PWM
/// `led_fixture` and the BLE bridge. Both personalities run at once, so the DMX
/// value is fanned out over a `Watch` (one latest value, per-receiver "seen"
/// tracking) rather than a `Signal` (single waker — a second waiter would starve).
pub const DMX_CONSUMERS: usize = 2;

/// The shared latest DMX value, written by the sACN listener and observed by each
/// consumer personality. See [`DMX_CONSUMERS`].
pub type DmxWatch = Watch<CriticalSectionRawMutex, DmxValue, DMX_CONSUMERS>;
/// Producer handle into [`DmxWatch`] (held by the sACN listener).
pub type DmxSender = Sender<'static, CriticalSectionRawMutex, DmxValue, DMX_CONSUMERS>;
/// Per-consumer handle out of [`DmxWatch`] (one each for PWM and BLE).
pub type DmxReceiver = Receiver<'static, CriticalSectionRawMutex, DmxValue, DMX_CONSUMERS>;

/// Which 7E/EF BLE fixture a board bridges to. Both speak the same colour/white/
/// brightness frames — the two-star bench test proved byte1 is don't-care — so only
/// the GATT layout, the power-frame bytes, and whether a gobo motor exists diverge.
/// This enum is just the identity; the per-dialect bytes and UUIDs live in `ble`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dialect {
    /// LEDBLE / HM-10 fixture (advertises `LEDBLE-NN-XXXX`, e.g. `LEDBLE-02-91E3`):
    /// service 0xFFE0 / char 0xFFE1, has a gobo motor.
    Ledble,
    /// ELK-BLEDOM fixture (advertises `ELK-BLEDOM` / `ELK-BLEDWM`, e.g.
    /// `ELK-BLEDWM 45`): service 0xFFF0 / char 0xFFF3, no gobo.
    Elk,
}

/// A bridged fixture's BLE identity: its MAC and the dialect its controller speaks.
/// MAC and dialect are meaningless apart, so they travel together as one value
/// through config, `main`, and into `ble::run`.
#[derive(Clone, Copy)]
pub struct BleTarget {
    mac: [u8; 6],
    dialect: Dialect,
}

impl BleTarget {
    /// `const` so board rows can be built in the `config::BOARDS` constant.
    pub const fn new(mac: [u8; 6], dialect: Dialect) -> Self {
        Self { mac, dialect }
    }

    pub fn mac(self) -> [u8; 6] { self.mac }
    pub fn dialect(self) -> Dialect { self.dialect }
}

/// One fixture's DMX slots. Six channels: Intensity, R, G, B, White, and
/// Gobo rotation. The PWM personality ignores Gobo rotation; the BLE personality
/// (`ble`) uses all six.
#[derive(Clone, Copy, PartialEq)]
pub struct DmxValue([u8; 6]);

impl DmxValue {
    pub const LEN: usize = 6;

    pub fn new(slots: [u8; Self::LEN]) -> Self {
        Self(slots)
    }

    pub fn intensity(self) -> u8 { self.0[0] }
    pub fn red(self) -> u8 { self.0[1] }
    pub fn green(self) -> u8 { self.0[2] }
    pub fn blue(self) -> u8 { self.0[3] }
    pub fn white(self) -> u8 { self.0[4] }
    pub fn gobo(self) -> u8 { self.0[5] }
}

#[derive(Debug)]
pub enum DmxConfigError {
    Address,
    Universe,
    Port,
}

#[derive(Clone, Copy)]
pub struct DmxConfig {
    address: u16,
    universe: u16,
    sacn_port: u16,
}

impl DmxConfig {
    pub fn new(address: u16, universe: u16, sacn_port: u16) -> Result<Self, DmxConfigError> {
        if !(1..=512).contains(&address) {
            return Err(DmxConfigError::Address);
        }
        // E1.31 §9.1.1: universe 0 and 64000–65535 are reserved; valid range is 1–63999.
        if !(1..=63999).contains(&universe) {
            return Err(DmxConfigError::Universe);
        }
        if sacn_port == 0 {
            return Err(DmxConfigError::Port);
        }
        Ok(Self { address, universe, sacn_port })
    }

    pub fn address(self) -> u16 { self.address }
    pub fn universe(self) -> u16 { self.universe }
    pub fn sacn_port(self) -> u16 { self.sacn_port }

    pub fn multicast_address(self) -> Ipv4Address {
        Ipv4Address::new(239, 255, (self.universe >> 8) as u8, self.universe as u8)
    }
}

#[derive(Debug)]
pub enum WifiConfigError {
    SsidLength,
    SsidNul,
    PasswordLength,
    PasswordNul,
}

#[derive(Clone)]
pub struct WifiConfig {
    ssid: String,
    password: String,
}

impl WifiConfig {
    pub fn new(ssid: String, password: String) -> Result<Self, WifiConfigError> {
        if ssid.is_empty() || ssid.len() > 32 {
            return Err(WifiConfigError::SsidLength);
        }
        if ssid.bytes().any(|b| b == 0) {
            return Err(WifiConfigError::SsidNul);
        }
        // WPA2-PSK requires 8–63 ASCII characters; 64-byte form is a raw PMK hex string.
        if password.len() < 8 || password.len() > 64 {
            return Err(WifiConfigError::PasswordLength);
        }
        if password.bytes().any(|b| b == 0) {
            return Err(WifiConfigError::PasswordNul);
        }
        Ok(Self { ssid, password })
    }

    pub fn ssid(&self) -> &str { &self.ssid }
    pub fn password(&self) -> &str { &self.password }
}
