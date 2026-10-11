//! Compiled-in, per-board configuration keyed by WiFi station MAC address.
//!
//! ## Why config is keyed by MAC
//!
//! The same firmware binary is flashed to every ball: `deploy.sh` builds a single `ball`
//! binary, and `remote-deploy.sh` flashes that one binary to every attached board. A board
//! therefore cannot know at compile time *which* ball it is — it must self-identify at
//! runtime. The stable per-board identity is its WiFi station MAC, programmed into efuse at
//! manufacture and read at boot via
//! `efuse::interface_mac_address(InterfaceMacAddress::Station)` (before WiFi starts). This
//! module maps each known MAC to that board's DMX base address; WiFi credentials, universe,
//! and sACN port are the same for every board and live as global constants below.
//!
//! ## Getting a board's MAC the first time
//!
//! 1. Flash the firmware and watch the RTT log. Every board prints its MAC at boot as
//!    `mac address:    XX:XX:XX:XX:XX:XX`, and an unprovisioned one follows it with the
//!    `Board` row to paste.
//! 2. Add that row to `BOARDS` below, set its DMX address, and re-flash.
//!
//! An unprovisioned MAC panics on purpose (see `dmx_config_for`) so a board is never
//! silently run with the wrong DMX address.
//!
//! Caveat: this is the *station* MAC, **not** the USB-JTAG debug-unit MAC that appears in
//! the `/dev/serial/by-id` names of `remote-deploy.sh`'s `DEVICE_MAP`. Do not copy the
//! DEVICE_MAP values here — read the station MAC from the RTT log.

use rtt_target::rprintln;

use crate::models::{DmxConfig, WifiConfig};

// ── Global settings (identical for every board) ───────────────────────────────
// The SSID and passphrase of the Pi's access point. `hostapd.conf` on the Pi is what the
// AP actually uses; these have to be changed along with it.
const SSID: &str = "closed claw DMX";
const PASSWORD: &str = "close-that-claw";
// What the brain sends on: `UNIVERSE` and `SACN_PORT` in `cortex/src/config.rs`.
const UNIVERSE: u16 = 1;
const SACN_PORT: u16 = 5568;

// ── Per-board settings, keyed by station MAC ───────────────────────────────────
struct Board {
    /// WiFi station MAC, read from efuse at boot — the board's stable identity.
    mac: [u8; 6],
    /// DMX base address; the ball's other slots follow it.
    dmx_address: u16,
}

const BOARDS: &[Board] = &[
    Board { mac: [0x40, 0x4C, 0xCA, 0xFB, 0x79, 0x10], dmx_address: 1 }, // TODO set DMX address
];

/// Returns the DMX configuration for the board with the given station MAC.
/// Panics if the MAC is not provisioned in `BOARDS` — an unknown board must not
/// run with a guessed DMX address — or if its row holds an invalid one.
pub fn dmx_config_for(mac: [u8; 6]) -> DmxConfig {
    let board = BOARDS.iter().find(|board| board.mac == mac).unwrap_or_else(|| {
        rprintln!("unprovisioned board: add this row to config::BOARDS in config.rs:");
        rprintln!(
            "    Board {{ mac: [0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}], dmx_address: 1 }}, // TODO set DMX address",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );
        panic!("unprovisioned board MAC");
    });
    DmxConfig::new(board.dmx_address, UNIVERSE, SACN_PORT)
}

/// The WiFi credentials shared by every board. A `const`, so that a malformed SSID or
/// passphrase fails the build instead of the board.
pub const WIFI_CONFIG: WifiConfig = WifiConfig::new(SSID, PASSWORD);
