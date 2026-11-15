//! What memory-pi is and how it is holding up, read as `fixtures/memory.oscquery.json` names
//! it: the shared blocks, `cortex::report`'s, and Memory's own leaves on top of them.
//!
//! Every file and tool read here is part of every Pi running Raspberry Pi OS, so a reading that
//! cannot be taken panics: the install is broken, and systemd's restarts say so in the journal
//! rather than the recording showing a board with readings missing.

use cortex::osc::Value;
use cortex::report::{self, Message, When, messages};
use cortex::system::{self, read};
use std::collections::BTreeSet;
use std::ffi::CString;
use std::os::unix::fs::MetadataExt;
use std::process::Command;

/// The device Memory reports as.
const DEVICE: &str = "memory";
const TANK: &str = "/tank";

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
pub fn connect() -> Vec<Message> {
    use crate::fixtures::memory::*;
    let mut out = report::shared(&[DEVICE], When::Connect);
    out.extend(messages([
        (
            SysMemoryRam::PATH,
            Value::Int((system::installed_ram_kb() / 1024) as i32),
        ),
        (
            SysMemoryMemTotal::PATH,
            Value::Int((system::meminfo_kb("MemTotal") / 1024) as i32),
        ),
    ]));
    out
}

/// The readings sent every second. `warned` holds the latched throttle flags already logged.
pub fn state(warned: &mut BTreeSet<&'static str>) -> Vec<Message> {
    use crate::fixtures::memory::*;
    let throttled = throttled();
    for (bit, flag) in THROTTLE_FLAGS.iter().enumerate() {
        if throttled >> (bit as u32 + THROTTLE_LATCHED_SHIFT) & 1 == 1 && warned.insert(flag) {
            eprintln!("memory: {flag} has occurred since boot (0x{throttled:x})");
        }
    }
    let now = |bit: u32| Value::Bool(throttled >> bit & 1 == 1);
    let since = |bit: u32| now(bit + THROTTLE_LATCHED_SHIFT);
    let mut out = report::shared(&[DEVICE], When::Second);
    out.extend(messages([
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
        (SysMemoryDiskFree::PATH, Value::Int(free_mb("/") as i32)),
        (SysMemoryTankFree::PATH, Value::Float(tank_free_mb())),
    ]));
    out
}

fn number<T: std::str::FromStr>(path: &str) -> T {
    read(path)
        .trim()
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
