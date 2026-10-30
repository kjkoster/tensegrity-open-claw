//! The camera's address, login, and the settings that belong to the camera rather than to who
//! is looking: what a consumer needs comes in its `View`.

use crate::view::Light;
use std::time::Duration;

/// Where the camera sits: the Pi's wired segment, at the address the Pi's DHCP server reserves
/// for it. Read at build time from `etc/claw-eth0.dnsmasq.conf`, where the reservation is.
pub const HOST: &str = env!("CAMERA_ADDRESS");

pub const USER: &str = "root";

/// The factory default, which firmware 6.15.2 comes out of a reset with, kept as it is: a
/// reset or replacement camera then takes the retina's login with nothing done to it. Safe to
/// leave because the camera is on the Pi's private wired segment, reachable from nowhere else.
pub const PASSWORD: &str = "pass";

/// Set over VAPIX on every connect, whatever is asked, and read back, so a factory reset or a
/// replacement camera comes up the same. Verified on firmware 6.15.2; the reasons for each are
/// on the `last-version-with-eyeball` tag, beside the old eyeball's copy of this list.
pub const SETTINGS: &[(&str, &str)] = &[
    // The mains here; 60 Hz buys banding.
    ("ImageSource.I0.CaptureFrequency", "50Hz"),
    // Bounded rather than locked: a lock freezes whatever was metered when it was set.
    ("ImageSource.I0.Sensor.Exposure", "auto"),
    // 10 ms, so someone walking is a shape and not a streak. The camera's own default is two
    // seconds.
    ("ImageSource.I0.Sensor.MaxExposureTime", "10000"),
    // Light spent on a short shutter rather than on low noise.
    ("ImageSource.I0.Sensor.ExposurePriority", "0"),
    ("ImageSource.I0.Sensor.ExposurePriorityLowlight", "0"),
];

/// The settings for the light a consumer asks for, beside `SETTINGS`.
pub const fn light(light: Light) -> &'static [(&'static str, &'static str)] {
    match light {
        Light::Infrared => &[
            // The sensor sees 850 nm, lit by the camera's own illuminator, which has no
            // switch of its own: this is the only infrared control there is.
            ("ImageSource.I0.DayNight.IrCutFilter", "no"),
            // The picture is infrared; colour carries nothing.
            ("Image.I0.Appearance.ColorEnabled", "no"),
            // Locked, so coloured light cannot be chased.
            ("ImageSource.I0.Sensor.WhiteBalance", "fixed_outdoor1"),
        ],
        Light::Colour => &[
            // In for good, not `auto`: the camera would take it out at dusk and turn the
            // colour grey.
            ("ImageSource.I0.DayNight.IrCutFilter", "yes"),
            ("Image.I0.Appearance.ColorEnabled", "yes"),
            ("ImageSource.I0.Sensor.WhiteBalance", "auto"),
        ],
    }
}

/// How long a connection to the camera may take to open, and the stream may go without a byte,
/// before the camera counts as gone and is reconnected.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// The cap on the backoff between attempts to reach the camera.
pub const RETRY_MAX_S: u64 = 30;
