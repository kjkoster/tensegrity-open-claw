use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use cortex_build::{Addressed, Sacn};

// The rig's QLC+ workspace. A ball is a fixture patched in it like any other, so where it
// listens is read from the same file the brain's patch is generated from.
const WORKSPACE: &str = "../claw.qxw";

// What makes a patched fixture a ball: it resolves through this definition. The definition is
// also what the firmware reads its channels out of, so their order and their bands have no
// second copy here.
const DEFINITION: &str = "../fixtures/ball.qxf";

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

    println!("cargo:rustc-env=TRIPLET={}", env::var("TARGET").unwrap());

    let addressing = cortex_build::addressing(Path::new(WORKSPACE));
    let definition = fs::canonicalize(DEFINITION)
        .unwrap_or_else(|e| panic!("cannot find {DEFINITION}: {e}"));
    let balls: Vec<&Addressed> = addressing
        .fixtures
        .iter()
        .filter(|fixture| fs::canonicalize(&fixture.definition).is_ok_and(|path| path == definition))
        .collect();

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("patch.rs"), render_patch(&addressing.sacn, &balls)).unwrap();
    fs::write(out_dir.join("layout.rs"), render_layout(&balls)).unwrap();
}

/// Renders one `DmxConfig` constant per ball the workspace patches.
///
/// Constants and nothing else, so the firmware gets its addressing without a table to search
/// or anything to allocate, and a board row naming a ball that is no longer patched stops
/// compiling.
fn render_patch(sacn: &Sacn, balls: &[&Addressed]) -> String {
    let mut out = String::new();
    writeln!(out, "// Generated from {WORKSPACE}. Do not edit.").unwrap();
    for ball in balls {
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

/// Renders where each channel sits among a ball's slots, and the bands each declares, under
/// the names the brain's generated `Ball` uses for them.
///
/// One layout for every ball, because one binary is flashed to every board: balls patched in
/// different modes would need a layout each, and the board cannot know which it is until it
/// reads its MAC.
fn render_layout(balls: &[&Addressed]) -> String {
    let Some(first) = balls.first() else {
        panic!("{WORKSPACE} patches no fixture through {DEFINITION}");
    };
    if let Some(other) = balls.iter().find(|ball| ball.fields != first.fields) {
        panic!(
            "{} and {} are patched in different modes of {DEFINITION}; every ball runs the \
             same firmware, so they must share one",
            first.name, other.name
        );
    }

    let mut out = String::new();
    writeln!(out, "// Generated from {DEFINITION}, as {WORKSPACE} patches it. Do not edit.").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "pub const LEN: usize = {};", first.channels).unwrap();
    for field in &first.fields {
        let name = field.name.to_ascii_uppercase();
        writeln!(out).unwrap();
        writeln!(out, "pub const {name}: usize = {};", field.offset).unwrap();
        for band in &field.bands {
            writeln!(
                out,
                "pub const {name}_{}: Band = Band::new({}, {});",
                band.ident, band.min, band.max
            )
            .unwrap();
        }
    }
    out
}

/// The value of a `key=value` line, verbatim: hostapd does not trim or unquote it either.
fn setting<'a>(conf: &'a str, key: &str) -> Option<&'a str> {
    conf.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}
