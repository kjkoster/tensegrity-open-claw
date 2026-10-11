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
//! module maps each known MAC to the fixture that board is in the rig's QLC+ workspace.
//!
//! ## Why a board names a fixture and not an address
//!
//! A ball is patched in the workspace like every other fixture, and the brain's patch is
//! generated from that file. An address written here would be a second copy of it, right
//! until the first repatch. `build.rs` turns every ball in the workspace into a constant in
//! `patch` instead, carrying its DMX address, the universe and the sACN port, so a ball that
//! is renamed or removed in QLC+ stops this file compiling, and one patched there with no
//! board to be it is reported as unused.
//!
//! ## Getting a board's MAC the first time
//!
//! 1. Flash the firmware and watch the RTT log. Every board prints its MAC at boot as
//!    `mac address:    XX:XX:XX:XX:XX:XX`, and an unprovisioned one follows it with the
//!    `Board` row to paste.
//! 2. Add that row to `BOARDS` below, name its fixture, and re-flash.
//!
//! An unprovisioned MAC panics on purpose (see `dmx_config_for`) so a board is never
//! silently run as the wrong fixture.
//!
//! Caveat: this is the *station* MAC, **not** the USB-JTAG debug-unit MAC that appears in
//! the `/dev/serial/by-id` names of `remote-deploy.sh`'s `DEVICE_MAP`. Do not copy the
//! DEVICE_MAP values here — read the station MAC from the RTT log.

use embassy_time::Duration;
use rtt_target::rprintln;

use crate::models::{DmxConfig, WifiConfig};

// ── Global settings (identical for every board) ───────────────────────────────
// Read out of `etc/hostapd.conf` by `build.rs`, so the firmware is always built against
// the access point configuration that is deployed alongside it.
const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASSPHRASE");
const COUNTRY: &str = env!("WIFI_COUNTRY");
const CHANNEL: u64 = number(env!("WIFI_CHANNEL"));
// Seconds of silence after which the access point forgets a station.
const AP_MAX_INACTIVITY_S: u64 = number(env!("AP_MAX_INACTIVITY"));

const fn number(setting: &str) -> u64 {
    match u64::from_str_radix(setting, 10) {
        Ok(number) => number,
        Err(_) => panic!("a setting in etc/hostapd.conf that has to be a number is not one"),
    }
}

/// How long the radio stays quiet between connection attempts. An access point that still
/// lists this ball as associated from before a restart does not answer its attempts to
/// join, and every attempt it hears keeps that stale entry alive. It only forgets a station
/// that has been silent for its `ap_max_inactivity`, so the ball stays silent for that long
/// and a margin on top.
pub const WIFI_RETRY_SILENCE: Duration = Duration::from_secs(AP_MAX_INACTIVITY_S + 5);

// ── The balls the workspace patches ────────────────────────────────────────────
mod patch {
    use crate::models::DmxConfig;

    include!(concat!(env!("OUT_DIR"), "/patch.rs"));
}

// ── Per-board settings, keyed by station MAC ───────────────────────────────────
struct Board {
    /// WiFi station MAC, read from efuse at boot — the board's stable identity.
    mac: [u8; 6],
    /// The fixture this board is in the workspace, and with it where it listens.
    dmx: DmxConfig,
}

const BOARDS: &[Board] = &[
    Board { mac: [0x40, 0x4C, 0xCA, 0xFB, 0x79, 0x10], dmx: patch::BALL_5 },
];

/// Returns the DMX configuration for the board with the given station MAC.
/// Panics if the MAC is not provisioned in `BOARDS` — an unknown board must not
/// run as a guessed fixture.
pub fn dmx_config_for(mac: [u8; 6]) -> DmxConfig {
    let board = BOARDS.iter().find(|board| board.mac == mac).unwrap_or_else(|| {
        rprintln!("unprovisioned board: add this row to config::BOARDS in config.rs:");
        rprintln!(
            "    Board {{ mac: [0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}, 0x{:02X}], dmx: patch::BALL_ }}, // TODO name its fixture",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );
        panic!("unprovisioned board MAC");
    });
    board.dmx
}

/// The WiFi settings shared by every board. A `const`, so that a malformed one fails the
/// build instead of the board.
pub const WIFI_CONFIG: WifiConfig = {
    assert!(CHANNEL <= u8::MAX as u64, "WiFi channel must be 1-13");
    WifiConfig::new(SSID, PASSWORD, COUNTRY, CHANNEL as u8)
};
