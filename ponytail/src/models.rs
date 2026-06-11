extern crate alloc;

use alloc::string::String;
use embassy_net::Ipv4Address;

pub const DMX_MAXVALUE: u8 = 255;

#[derive(Clone, Copy, PartialEq)]
pub struct DmxValue([u8; 5]);

impl DmxValue {
    pub const LEN: usize = 5;

    pub fn new(slots: [u8; Self::LEN]) -> Self {
        Self(slots)
    }

    pub fn intensity(self) -> u8 { self.0[0] }
    pub fn red(self) -> u8 { self.0[1] }
    pub fn green(self) -> u8 { self.0[2] }
    pub fn blue(self) -> u8 { self.0[3] }
    pub fn white(self) -> u8 { self.0[4] }
}

#[derive(Clone, Copy)]
pub struct DmxConfig {
    address: u16,
    universe: u16,
    sacn_port: u16,
}

impl DmxConfig {
    pub fn new(address: u16, universe: u16, sacn_port: u16) -> Result<Self, ()> {
        if !(1..=512).contains(&address) {
            return Err(());
        }
        // E1.31 §9.1.1: universe 0 and 64000–65535 are reserved; valid range is 1–63999.
        if !(1..=63999).contains(&universe) {
            return Err(());
        }
        if sacn_port == 0 {
            return Err(());
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

#[derive(Clone)]
pub struct WifiConfig {
    ssid: String,
    password: String,
}

impl WifiConfig {
    pub fn new(ssid: String, password: String) -> Result<Self, ()> {
        if ssid.is_empty() || ssid.len() > 32 {
            return Err(());
        }
        if ssid.bytes().any(|b| b == 0) {
            return Err(());
        }
        // WPA2-PSK requires 8–63 ASCII characters; 64-byte form is a raw PMK hex string.
        if password.len() < 8 || password.len() > 64 {
            return Err(());
        }
        if password.bytes().any(|b| b == 0) {
            return Err(());
        }
        Ok(Self { ssid, password })
    }

    pub fn ssid(&self) -> &str { &self.ssid }
    pub fn password(&self) -> &str { &self.password }
}
