//! What memory-pi is and how it is holding up, read as `fixtures/memory.oscquery.json` names
//! it under `/sys/memory/`.
//!
//! Every file and tool read here is part of every Pi running Raspberry Pi OS, so a reading that
//! cannot be taken panics: the install is broken, and systemd's restarts say so in the journal
//! rather than the recording showing a board with readings missing.

use crate::osc::Value;
use std::collections::BTreeSet;
use std::ffi::CString;
use std::os::unix::fs::MetadataExt;
use std::process::Command;

const TANK: &str = "/tank";
const WIFI: &str = "wlan0";

/// Bits of `vcgencmd get_throttled`, in the fixture's order. The low nibble is happening now;
/// the same four shifted into the high half are latched since boot.
const THROTTLE_FLAGS: [&str; 4] = [
    "under_voltage",
    "arm_capped",
    "throttled",
    "soft_temp_limit",
];
const THROTTLE_LATCHED_SHIFT: u32 = 16;

/// What does not change while the Pi runs.
pub fn connect() -> Vec<(&'static str, Value)> {
    use crate::fixtures::memory::*;
    vec![
        (
            SysMemoryBoard::PATH,
            Value::Text(
                read("/proc/device-tree/model")
                    .trim_end_matches('\0')
                    .to_owned(),
            ),
        ),
        (
            SysMemoryCores::PATH,
            Value::Int(
                std::thread::available_parallelism()
                    .expect("the number of cores")
                    .get() as i32,
            ),
        ),
        (
            SysMemoryCpuFreq::PATH,
            Value::Int(
                number::<i32>("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq") / 1000,
            ),
        ),
        (SysMemoryRam::PATH, Value::Int(installed_ram_mb())),
        (SysMemoryMemTotal::PATH, Value::Int(meminfo_mb("MemTotal"))),
        (
            SysMemoryWifiMac::PATH,
            Value::Text(read(&format!("/sys/class/net/{WIFI}/address"))),
        ),
    ]
}

/// The readings sent every second. `warned` holds the latched throttle flags already logged.
pub fn state(warned: &mut BTreeSet<&'static str>) -> Vec<(&'static str, Value)> {
    use crate::fixtures::memory::*;
    let throttled = throttled();
    for (bit, flag) in THROTTLE_FLAGS.iter().enumerate() {
        if throttled >> (bit as u32 + THROTTLE_LATCHED_SHIFT) & 1 == 1 && warned.insert(flag) {
            eprintln!("memory: {flag} has occurred since boot (0x{throttled:x})");
        }
    }
    let now = |bit: u32| Value::Bool(throttled >> bit & 1 == 1);
    let since = |bit: u32| now(bit + THROTTLE_LATCHED_SHIFT);
    let uptime: f64 = read("/proc/uptime")
        .split(' ')
        .next()
        .and_then(|s| s.parse().ok())
        .expect("/proc/uptime in seconds");
    vec![
        (SysMemoryUptime::PATH, Value::Int(uptime as i32)),
        (
            SysMemoryTemperature::PATH,
            Value::Float(number::<i32>("/sys/class/thermal/thermal_zone0/temp") as f32 / 1000.0),
        ),
        (SysMemoryThrottlingRaw::PATH, Value::Int(throttled as i32)),
        (SysMemoryThrottlingNowUnderVoltage::PATH, now(0)),
        (SysMemoryThrottlingNowArmCapped::PATH, now(1)),
        (SysMemoryThrottlingNowThrottled::PATH, now(2)),
        (SysMemoryThrottlingNowSoftTempLimit::PATH, now(3)),
        (SysMemoryThrottlingSinceBootUnderVoltage::PATH, since(0)),
        (SysMemoryThrottlingSinceBootArmCapped::PATH, since(1)),
        (SysMemoryThrottlingSinceBootThrottled::PATH, since(2)),
        (SysMemoryThrottlingSinceBootSoftTempLimit::PATH, since(3)),
        (
            SysMemoryMemAvailable::PATH,
            Value::Int(meminfo_mb("MemAvailable")),
        ),
        (SysMemoryDiskFree::PATH, Value::Int(free_mb("/") as i32)),
        (SysMemoryTankFree::PATH, Value::Float(tank_free_mb())),
        (SysMemoryWifiRssi::PATH, Value::Int(wifi_rssi())),
        (SysMemoryWifiIp::PATH, Value::Text(wifi_ip())),
    ]
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .trim()
        .to_owned()
}

fn number<T: std::str::FromStr>(path: &str) -> T {
    read(path)
        .parse()
        .unwrap_or_else(|_| panic!("{path} holds no number"))
}

fn run(program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{program}: {e}"));
    if !output.status.success() {
        panic!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The RAM fitted, decoded from the board's revision code; not `MemTotal`, which is what is
/// left once the GPU and the kernel have taken their share. A new-style code (bit 23), which
/// every Pi since the 2 B has, carries the size in bits 20–22, as 256 MB doubled that many
/// times.
fn installed_ram_mb() -> i32 {
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
    256 << ((code >> 20) & 0x7)
}

fn meminfo_mb(key: &str) -> i32 {
    let meminfo = read("/proc/meminfo");
    let kb: i64 = meminfo
        .lines()
        .find_map(|line| {
            line.strip_prefix(key)?
                .strip_prefix(':')?
                .trim()
                .strip_suffix(" kB")?
                .parse()
                .ok()
        })
        .unwrap_or_else(|| panic!("/proc/meminfo has no {key}"));
    (kb / 1024) as i32
}

fn throttled() -> u32 {
    let answer = run("vcgencmd", &["get_throttled"]);
    let hex = answer
        .strip_prefix("throttled=0x")
        .unwrap_or_else(|| panic!("vcgencmd get_throttled said {answer:?}"));
    u32::from_str_radix(hex, 16)
        .unwrap_or_else(|_| panic!("vcgencmd get_throttled said {answer:?}"))
}

/// Free space for an unprivileged user on the file system holding `path`.
fn free_mb(path: &str) -> u64 {
    let c_path = CString::new(path).unwrap();
    // SAFETY: `statvfs` is plain data, so all zeroes is a valid value, and the call only writes
    // into it.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a NUL-terminated string and `stat` outlives the call.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        panic!("statvfs {path}: {}", std::io::Error::last_os_error());
    }
    stat.f_bavail as u64 * stat.f_frsize as u64 / 1024 / 1024
}

/// NaN while the SSD is not mounted: /tank is then a directory on the SD card, on the same
/// device as /, and its free space would read as the SSD's.
fn tank_free_mb() -> f32 {
    let device = |path: &str| {
        std::fs::metadata(path)
            .unwrap_or_else(|e| panic!("{path}: {e}"))
            .dev()
    };
    if device(TANK) == device("/") {
        return f32::NAN;
    }
    free_mb(TANK) as f32
}

/// How strongly memory-pi hears the access point; 0 while it is not associated.
fn wifi_rssi() -> i32 {
    run("iw", &["dev", WIFI, "link"])
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

/// wlan0's IPv4 address, without its prefix length; empty while it has none.
fn wifi_ip() -> String {
    run("ip", &["-o", "-4", "addr", "show", "dev", WIFI])
        .split_whitespace()
        .skip_while(|word| *word != "inet")
        .nth(1)
        .and_then(|cidr| cidr.split('/').next())
        .unwrap_or("")
        .to_owned()
}
