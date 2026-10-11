extern crate alloc;

use alloc::string::String;
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
