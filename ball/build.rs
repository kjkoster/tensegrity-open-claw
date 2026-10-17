use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// The rig's QLC+ workspace. A ball is a fixture patched in it like any other, so where it
// listens is read from the same file the brain's patch is generated from.
const WORKSPACE: &str = "../claw.qxw";

// How the workspace names a ball: the manufacturer and model of `fixtures/ball.qxf`.
const MANUFACTURER: &str = "Tensegrity";
const MODEL: &str = "Ball";

// The access point's own configuration. The firmware takes what it has to agree with the
// access point on from here, so the two cannot drift apart.
const HOSTAPD_CONF: &str = "../etc/hostapd.conf";

// hostapd setting -> the environment variable the firmware reads it from with `env!`.
const SETTINGS: &[(&str, &str)] = &[
    ("ssid", "WIFI_SSID"),
    ("wpa_passphrase", "WIFI_PASSPHRASE"),
    ("country_code", "WIFI_COUNTRY"),
    ("channel", "WIFI_CHANNEL"),
    ("ap_max_inactivity", "AP_MAX_INACTIVITY"),
];

fn main() {
    println!("cargo:rustc-link-arg=-Tlinkall.x");

    println!("cargo:rerun-if-changed={HOSTAPD_CONF}");
    let conf = fs::read_to_string(HOSTAPD_CONF)
        .unwrap_or_else(|e| panic!("cannot read {HOSTAPD_CONF}: {e}"));
    for (key, variable) in SETTINGS {
        let value = setting(&conf, key)
            .unwrap_or_else(|| panic!("{HOSTAPD_CONF} has no `{key}=` line"));
        println!("cargo:rustc-env={variable}={value}");
    }

    println!("cargo:rustc-env=FIRMWARE={}", firmware());

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("patch.rs"), render_patch()).unwrap();
}

/// The crate version and the commit it was built from, `-dirty` when the tree had changes.
///
/// It goes on `ball/firmware`, which is how a reflash is confirmed to have landed:
/// the version alone never changes between deploys, and the commit does.
fn firmware() -> String {
    // A build script that names any file reruns only when one of those changes, so the commit
    // has to be watched for explicitly: `HEAD` moves on a checkout, the index on a commit or a
    // stage, and the sources on an edit that would make the tree dirty.
    for watched in ["../.git/HEAD", "../.git/index", "src", "../rig-mqtt/src"] {
        println!("cargo:rerun-if-changed={watched}");
    }
    let output = Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .unwrap_or_else(|e| panic!("cannot run git: {e}"));
    if !output.status.success() {
        panic!(
            "git describe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let commit = String::from_utf8(output.stdout).unwrap();
    format!("{} {}", env::var("CARGO_PKG_VERSION").unwrap(), commit.trim())
}

/// Renders one `DmxConfig` constant per ball the workspace patches.
///
/// Constants and nothing else, so the firmware gets its addressing without a table to search
/// or anything to allocate, and a board row naming a ball that is no longer patched stops
/// compiling.
fn render_patch() -> String {
    let addressing = cortex_build::addressing(Path::new(WORKSPACE));
    let sacn = &addressing.sacn;

    let mut out = String::new();
    writeln!(out, "// Generated from {WORKSPACE}. Do not edit.").unwrap();
    for ball in addressing
        .fixtures
        .iter()
        .filter(|fixture| fixture.manufacturer == MANUFACTURER && fixture.model == MODEL)
    {
        writeln!(out).unwrap();
        writeln!(
            out,
            "/// {} — universe {}, DMX address {} (slots {}–{}).",
            ball.name,
            sacn.universe,
            ball.address,
            ball.address,
            ball.address + ball.channels - 1,
        )
        .unwrap();
        writeln!(
            out,
            "pub const {}: DmxConfig = DmxConfig::new({}, {}, {});",
            ball.ident, ball.address, sacn.universe, sacn.port,
        )
        .unwrap();
    }
    out
}

/// The value of a `key=value` line, verbatim: hostapd does not trim or unquote it either.
fn setting<'a>(conf: &'a str, key: &str) -> Option<&'a str> {
    conf.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}
