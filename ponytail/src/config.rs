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
//! starts). This module maps each known MAC to that board's DMX base address (and
//! optional BLE target); WiFi credentials, universe, and sACN port are the same for
//! every board and live as global constants below.
//!
//! ## Getting a board's MAC the first time
//!
//! 1. Flash the firmware and watch the RTT log. Every board prints its MAC at
//!    boot as `mac address:    XX:XX:XX:XX:XX:XX`.
//! 2. Add a `Board` row to `BOARDS` below and re-flash.
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
const SSID: &str = "closed claw DMX";
const PASSWORD: &str = "close-that-claw";
const UNIVERSE: u16 = 1;
const SACN_PORT: u16 = 5568;

// ── Per-board settings, keyed by station MAC ───────────────────────────────────
// One row per board. Six channels per fixture: Fixture A → DMX start address 1
// (slots 1–6), Fixture B → 7 (slots 7–12). Matches brain's fixture layout.
//
// TODO: replace these placeholder station MACs with the real ones read from the RTT
// `mac=` line of each board (see module docs). For a board running the BLE bridge,
// also sniff its fixture's BLE MAC — and writable characteristic UUID (see `ble`) —
// and set `ble_target`. Until a board's true station MAC is listed here, it panics
// at boot.
struct Board {
    /// WiFi station MAC, read from efuse at boot — the board's stable identity.
    mac: [u8; 6],
    /// DMX base address (the Intensity slot); the other five channels follow.
    dmx_address: u16,
    /// The bridged Telink fixture's BLE MAC, or `None` if this board drives its LEDs
    /// over PWM rather than bridging to an original controller.
    ble_target: Option<[u8; 6]>,
}

const BOARDS: &[Board] = &[
    Board { mac: [0xAC, 0xA7, 0x04, 0x2C, 0x4F, 0xD8], dmx_address: 1, ble_target: Some([0xA4, 0xC1, 0x38, 0x40, 0x91, 0xE3]) },
    Board { mac: [0xDC, 0xB4, 0xD9, 0x3B, 0xB1, 0xA4], dmx_address: 7, ble_target: Some([0xA4, 0xC1, 0x38, 0x40, 0x91, 0xE4]) }, // TODO completely made up
];

/// Returns the BLE target MAC for the board with the given station MAC, or `None`
/// if the board is unknown or drives its LEDs over PWM.
pub fn ble_target_mac_for(mac: [u8; 6]) -> Option<[u8; 6]> {
    BOARDS
        .iter()
        .find(|board| board.mac == mac)
        .and_then(|board| board.ble_target)
}

/// Returns the DMX configuration for the board with the given station MAC.
/// Panics if the MAC is not provisioned in `BOARDS` — an unknown board must not
/// run with a guessed DMX address.
pub fn dmx_config_for(mac: [u8; 6]) -> DmxConfig {
    let board = BOARDS.iter().find(|board| board.mac == mac).unwrap_or_else(|| {
        rprintln!("unprovisioned board: add this row to config::BOARDS in config.rs:");
        rprintln!(
            "    Board {{ mac: [0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}], dmx_address: 1, ble_target: None }}, // TODO set DMX address",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );
        panic!("unprovisioned board MAC");
    });
    DmxConfig::new(board.dmx_address, UNIVERSE, SACN_PORT).expect("hardcoded DMX config is invalid")
}

/// Returns the WiFi credentials shared by every board.
pub fn wifi_config() -> WifiConfig {
    WifiConfig::new(SSID.into(), PASSWORD.into()).expect("hardcoded WiFi config is invalid")
}
