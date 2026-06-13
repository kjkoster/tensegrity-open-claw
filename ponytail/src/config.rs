//! Compiled-in, per-board configuration keyed by WiFi station MAC address.
//!
//! ## Why config is keyed by MAC
//!
//! The same firmware binary is flashed to every ponytail board: `deploy.sh`
//! builds and stages a single `ponytail` binary, and `remote-deploy.sh` flashes
//! that one binary to every attached board. A board therefore cannot know at
//! compile time *which* fixture it is — it must self-identify at runtime. The
//! stable per-board identity is its WiFi station MAC, programmed into efuse at
//! manufacture and read at boot via
//! `efuse::interface_mac_address(InterfaceMacAddress::Station)` (before WiFi
//! starts). This module maps each known MAC to that board's DMX base address;
//! WiFi credentials, universe, and sACN port are the same for every board and
//! live as global constants below.
//!
//! ## Getting a board's MAC the first time
//!
//! 1. Flash the firmware and watch the RTT log. Every board prints its MAC at
//!    boot as `mac=XX:XX:XX:XX:XX:XX`.
//! 2. Add a `(mac, dmx_base_address)` row to `BOARDS` below and re-flash.
//!
//! An unprovisioned MAC panics on purpose (see `dmx_config_for`) so a board is
//! never silently run with the wrong DMX address.
//!
//! Caveat: this is the *station* MAC, **not** the USB-JTAG debug-unit MAC shown
//! in `remote-deploy.sh`'s `DEVICE_MAP` (e.g. `AC:A7:04:2C:4F:D8`). Do not copy
//! the DEVICE_MAP values here — read the station MAC from the RTT log.

use rtt_target::rprintln;

use crate::models::{DmxConfig, WifiConfig};

// ── Global settings (identical for every board) ───────────────────────────────
const SSID: &str = "radiowaves";
const PASSWORD: &str = "IkWilInternetten!!";
const UNIVERSE: u16 = 1;
const SACN_PORT: u16 = 5568;

// ── Per-board DMX base address, keyed by station MAC ───────────────────────────
// Fixture A → DMX start address 1, Fixture B → 6 (matches brain's fixture layout).
//
// TODO: replace these placeholder MACs with the real station MACs read from the
// RTT `mac=` line of each board (see module docs). Until a board's true station
// MAC is listed here, it will panic at boot.
const BOARDS: &[([u8; 6], u16)] = &[
    ([0xAC, 0xA7, 0x04, 0x2C, 0x4F, 0xD8], 1), // Ponytail fixture
    ([0xDC, 0xB4, 0xD9, 0x3B, 0xB1, 0xA4], 6), // Ponytail fixture
];

/// Returns the DMX configuration for the board with the given station MAC.
/// Panics if the MAC is not provisioned in `BOARDS` — an unknown board must not
/// run with a guessed DMX address.
pub fn dmx_config_for(mac: [u8; 6]) -> DmxConfig {
    for (board_mac, address) in BOARDS {
        if *board_mac == mac {
            return DmxConfig::new(*address, UNIVERSE, SACN_PORT)
                .expect("hardcoded DMX config is invalid");
        }
    }
    rprintln!("unprovisioned board: add this row to config::BOARDS in config.rs:");
    rprintln!(
        "    ([0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}], 1), // TODO set DMX base address",
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5]
    );
    panic!("unprovisioned board MAC");
}

/// Returns the WiFi credentials shared by every board.
pub fn wifi_config() -> WifiConfig {
    WifiConfig::new(SSID.into(), PASSWORD.into()).expect("hardcoded WiFi config is invalid")
}
