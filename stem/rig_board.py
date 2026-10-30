"""What any of the rig's Pis is, and how it is holding up: the readings every health daemon
shares.

Stem reads claw-pi through this and memory's health daemon reads memory-pi, so a fix to how a
reading is taken lands on both Pis. What only one Pi has — claw-pi's modem and access point,
memory-pi's SSD — stays in that Pi's daemon.

Every file and tool read here is part of every Pi running Raspberry Pi OS, so a reader that
cannot get its value raises: the install is broken, and the daemon stops and says so rather
than reporting a board with readings missing.
"""

import os
import re
import subprocess
import time

# Bit positions in `vcgencmd get_throttled`. The low nibble is happening now; the same four
# shifted into the high half are latched since boot and are the ones that catch the brownout
# that happened while nobody was watching.
THROTTLE_FLAGS = {
    "under_voltage": 0,
    "arm_capped": 1,
    "throttled": 2,
    "soft_temp_limit": 3,
}
THROTTLE_LATCHED_SHIFT = 16


def read(path):
    """Reads a small file."""
    with open(path, "r") as handle:
        return handle.read().strip("\x00").strip()


def run(command):
    """Runs a helper and returns its output. One that is missing, fails or hangs raises; the
    caller catches `CalledProcessError` only where the helper fails for a device that can go
    away. Only its output is captured: what it says on stderr, which is why it failed, goes on
    to the daemon's own and into the journal."""
    finished = subprocess.run(command, stdout=subprocess.PIPE, text=True, timeout=5, check=True)
    return finished.stdout.strip()


# ── System ───────────────────────────────────────────────────────────────────


def cpu_field(name):
    found = re.search(rf"^{name}\s*:\s*(.+)$", read("/proc/cpuinfo"), re.MULTILINE)
    if not found:
        raise LookupError(f"/proc/cpuinfo has no {name}")
    return found.group(1).strip()


def system():
    """The hardware, under the `sys` subtree every part of the rig publishes its own under.
    Published once, retained. The health daemons are not compiled, so unlike the Rust parts
    they have no target triplet to report.
    """
    return {
        "board": read("/proc/device-tree/model"),
        "cores": os.cpu_count(),
        "cpu_frequency_mhz": cpu_frequency_mhz(),
        "ram_kb": installed_ram_kb(),
    }


def installed_ram_kb():
    """The RAM fitted, in KB, decoded from the board's revision code.

    Not `MemTotal`, which is what is left once the GPU and the kernel have taken their share. A
    new-style code (bit 23), which every Pi since the 2 B has, carries the size in bits 20–22,
    as 256 MB doubled that many times.
    """
    code = int(cpu_field("Revision"), 16)
    if not code & (1 << 23):
        raise ValueError(f"board revision {code:x} is old-style and does not give the RAM")
    return (256 * 1024) << ((code >> 20) & 0x7)


def cpu_frequency_mhz():
    """The top frequency the CPU is allowed to run at, which is what it can sustain when the
    rig is busy, rather than wherever the governor happens to have it at the moment."""
    return int(read("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")) // 1000


# ── Stats ────────────────────────────────────────────────────────────────────


def temperature_c():
    # From sysfs rather than vcgencmd: one file read against a subprocess, every interval,
    # for the same millidegrees.
    return round(int(read("/sys/class/thermal/thermal_zone0/temp")) / 1000.0, 1)


def throttling():
    """Decodes `get_throttled` into flags, keeping the raw word for the unrecognised bits.

    `now` is a condition to react to; `since_boot` is a condition to explain a bad show with.
    They are separated because they are answers to genuinely different questions and the raw
    hex answers neither without a table nobody has to hand at the time.
    """
    word = int(run(["vcgencmd", "get_throttled"]).split("=", 1)[1], 16)
    return {
        "raw": f"0x{word:x}",
        "now": {name: bool(word >> bit & 1) for name, bit in THROTTLE_FLAGS.items()},
        "since_boot": {
            name: bool(word >> (bit + THROTTLE_LATCHED_SHIFT) & 1)
            for name, bit in THROTTLE_FLAGS.items()
        },
    }


def memory_mb():
    text = read("/proc/meminfo")
    fields = {}
    for key in ("MemTotal", "MemAvailable"):
        found = re.search(rf"^{key}:\s+(\d+) kB", text, re.MULTILINE)
        if not found:
            raise LookupError(f"/proc/meminfo has no {key}")
        fields[key] = round(int(found.group(1)) / 1024)
    # Available rather than free: free excludes the page cache, reads alarmingly low on a
    # healthy box, and would have somebody hunting a leak that is not there.
    return {
        "total": fields["MemTotal"],
        "available": fields["MemAvailable"],
        "used": fields["MemTotal"] - fields["MemAvailable"],
    }


def addresses():
    """Interface → address, so a tree tells you the tunnel is up and the modem has a lease."""
    found = {}
    for line in run(["ip", "-o", "-4", "addr", "show"]).splitlines():
        fields = line.split()
        if len(fields) >= 4:
            found[fields[1]] = fields[3]
    return found


def disk_free_mb(path="/"):
    usage = os.statvfs(path)
    return round(usage.f_bavail * usage.f_frsize / 1024 / 1024)


def board_stats():
    """The part of every health daemon's reading that any Pi has."""
    return {
        "at": time.time(),
        "uptime_s": round(float(read("/proc/uptime").split()[0])),
        "temperature_c": temperature_c(),
        "throttling": throttling(),
        # Named rather than a three-element list, because `load/1m` says what it is and
        # `load/0` needs a reader who already knows the order.
        "load": dict(zip(("1m", "5m", "15m"), os.getloadavg())),
        "memory_mb": memory_mb(),
        "disk_free_mb": disk_free_mb(),
        "addresses": addresses(),
    }


def warn_latched(reading, warned, log):
    """Logs each latched throttle flag the first time it is seen.

    Worth a line in the journal, because a brownout that happened before the last reboot
    explains a slow show that nobody would otherwise connect to the power supply. `warned` is
    the caller's set of flags already logged.
    """
    for name, raised in reading["throttling"]["since_boot"].items():
        if raised and name not in warned:
            warned.add(name)
            log(f"{name} has occurred since boot ({reading['throttling']['raw']})")
