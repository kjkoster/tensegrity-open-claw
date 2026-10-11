use std::fs;

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
}

/// The value of a `key=value` line, verbatim: hostapd does not trim or unquote it either.
fn setting<'a>(conf: &'a str, key: &str) -> Option<&'a str> {
    conf.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}
