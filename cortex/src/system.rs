//! What a Raspberry Pi running Raspberry Pi OS is, and how it is holding up: the readings every
//! program built on cortex reports under `/sys/`, `/wifi/` and `/health/`, and that Memory
//! reports more of.
//!
//! What every Pi has panics when it cannot be read: the install is broken. The WiFi readers
//! are telemetry about a link that comes and goes, and the brain aborts on a panic, so a
//! WiFi reading that cannot be had is the empty reading instead.

/// The target triplet this was compiled for.
pub const TRIPLET: &str = env!("TRIPLET");

/// The WiFi interface on both Pis: chaos-pi's access point, memory-pi's only network.
pub const WIFI: &str = "wlan0";

/// The Pi, as its device tree names it.
pub fn board() -> String {
    read("/proc/device-tree/model")
        .trim_end_matches('\0')
        .trim()
        .to_owned()
}

/// The CPU cores.
pub fn cores() -> usize {
    std::thread::available_parallelism()
        .expect("the number of cores")
        .get()
}

/// The top clock the CPU is allowed, in MHz.
pub fn cpu_freq_mhz() -> u64 {
    let khz: u64 = read("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
        .trim()
        .parse()
        .expect("cpuinfo_max_freq in kHz");
    khz / 1000
}

/// Time since the Pi booted, in seconds.
pub fn uptime_s() -> u64 {
    read("/proc/uptime")
        .split(' ')
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .expect("/proc/uptime in seconds") as u64
}

/// One of `/proc/meminfo`'s figures, `MemTotal` or `MemAvailable`, in KB, as the kernel gives
/// it.
pub fn meminfo_kb(key: &str) -> u64 {
    read("/proc/meminfo")
        .lines()
        .find_map(|line| {
            line.strip_prefix(key)?
                .strip_prefix(':')?
                .trim()
                .strip_suffix("kB")?
                .trim()
                .parse()
                .ok()
        })
        .unwrap_or_else(|| panic!("/proc/meminfo has no {key} in kB"))
}

/// The memory available to start programs in, `MemAvailable`, in KB.
pub fn mem_avail_kb() -> u64 {
    meminfo_kb("MemAvailable")
}

/// The RAM fitted, in KB, decoded from the revision code in `/proc/cpuinfo`.
///
/// Not the kernel's `MemTotal`, which is what is left once the GPU and the kernel have taken
/// their share. A new-style code (bit 23), which every Pi since the 2 B has, carries the size
/// in bits 20–22, as 256 MB doubled that many times.
pub fn installed_ram_kb() -> u64 {
    let cpuinfo = read("/proc/cpuinfo");
    let revision = cpuinfo
        .lines()
        .find_map(|line| line.strip_prefix("Revision")?.split(':').nth(1))
        .expect("/proc/cpuinfo has a Revision");
    let code = u32::from_str_radix(revision.trim(), 16).expect("the Revision in hex");
    assert!(
        code & (1 << 23) != 0,
        "board revision {code:x} is old-style and does not give the RAM"
    );
    (256 * 1024) << ((code >> 20) & 0x7)
}

/// [`WIFI`]'s MAC address; empty if it cannot be read.
pub fn wifi_mac() -> String {
    std::fs::read_to_string(format!("/sys/class/net/{WIFI}/address"))
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

/// [`WIFI`]'s IPv4 address, dotted-quad; empty while it has none.
pub fn wifi_ip() -> String {
    output("ip", &["-o", "-4", "addr", "show", "dev", WIFI])
        .split_whitespace()
        .skip_while(|word| *word != "inet")
        .nth(1)
        .and_then(|cidr| cidr.split('/').next())
        .unwrap_or("")
        .to_owned()
}

/// How strongly [`WIFI`] hears the access point, in dBm; 0 while it is not associated, as an
/// access point never is.
pub fn wifi_rssi() -> i32 {
    output("iw", &["dev", WIFI, "link"])
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("signal:")?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

/// `path`'s contents; panics if it cannot be read.
pub fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// What `program` prints, or nothing if it cannot be run or fails.
fn output(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}
