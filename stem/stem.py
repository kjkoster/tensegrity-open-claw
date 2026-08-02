#!/usr/bin/env python3
"""Stem: what the machine is, and how it is holding up.

The health daemon, and deliberately the least capable process on the rig. It reads, it
publishes, and it does nothing else — no watchdog petting, no restarting, no re-dialling.
Those duties are designed but not built here yet, and keeping them out means Stem's death
costs telemetry and never uptime.

Two subtrees, both retained, because both answer questions asked after the interesting moment:

    stem/specs/…   what this machine is — written once, never changes while it runs
    stem/stats/…   how it is doing — temperature, throttling, memory, load, uplink

The camera is not here. Everything about it — what it is, how it is configured, and whether it
answers — belongs to eyeball under `eyeball/camera/`, so that one subtree is the whole answer
rather than two subtrees that have to be read together.

One field per topic: `stem/stats/temperature_c`, `stem/stats/throttling/since_boot/under_voltage`.
Nested readings become nested topics, so a subscriber can take a single number without knowing
anything about the shape it arrived in.

The throttle word is the reason `stats` exists at all. A Pi that browns out under a PoE
injector or cooks in a sealed cabinet reports both as flags that latch until reboot, and
finding them by SSH after a bad show is finding them too late.

Liveness is not published here. Every daemon publishes its own on `health/<service>` with a
will behind it, so a crashed process says so without Stem having to notice — including a
crashed Stem.
"""

import os
import platform
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rig_mqtt

SERVICE = "stem"
# Slow on purpose. Nothing here moves fast enough to be worth sampling often, and a telemetry
# tree that scrolls is one nobody reads.
STATS_INTERVAL_S = float(os.environ.get("STEM_STATS_INTERVAL_S", "30"))

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


def log(message):
    print(f"stem: {message}", file=sys.stderr, flush=True)


def read(path, default=None):
    """Reads a small file, returning `default` for anything the machine does not have."""
    try:
        with open(path, "r") as handle:
            return handle.read().strip("\x00").strip()
    except OSError:
        return default


def run(command):
    """Runs a helper and returns its output, or None. A missing tool is never fatal."""
    try:
        finished = subprocess.run(
            command, capture_output=True, text=True, timeout=5, check=False
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    return finished.stdout.strip() if finished.returncode == 0 else None


# ── Specs ────────────────────────────────────────────────────────────────────


def cpu_field(name):
    text = read("/proc/cpuinfo", "")
    found = re.search(rf"^{name}\s*:\s*(.+)$", text, re.MULTILINE)
    return found.group(1).strip() if found else None


def total_memory_mb():
    found = re.search(r"^MemTotal:\s+(\d+) kB", read("/proc/meminfo", ""), re.MULTILINE)
    return round(int(found.group(1)) / 1024) if found else None


def os_name():
    found = re.search(r'^PRETTY_NAME="?([^"\n]+)"?', read("/etc/os-release", ""), re.MULTILINE)
    return found.group(1) if found else None


def specs():
    """What this machine is. Published once, retained, and never republished.

    It exists because the numbers that matter downstream — the pose rate a core can sustain,
    whether a wheel exists for this interpreter — are answers to questions about this exact
    board, and reading them out of a telemetry tree beats an SSH session and a remembered
    command line.
    """
    return {
        "model": read("/proc/device-tree/model"),
        "revision": cpu_field("Revision"),
        "serial": cpu_field("Serial"),
        "kernel": platform.release(),
        "architecture": platform.machine(),
        "word_size": 64 if sys.maxsize > 2**32 else 32,
        "os": os_name(),
        "cores": os.cpu_count(),
        "memory_mb": total_memory_mb(),
        "python": platform.python_version(),
        "firmware": run(["vcgencmd", "version"]),
    }


# ── Stats ────────────────────────────────────────────────────────────────────


def temperature_c():
    # From sysfs rather than vcgencmd: one file read against a subprocess, every interval,
    # for the same millidegrees.
    raw = read("/sys/class/thermal/thermal_zone0/temp")
    return round(int(raw) / 1000.0, 1) if raw and raw.isdigit() else None


def throttling():
    """Decodes `get_throttled` into flags, keeping the raw word for the unrecognised bits.

    `now` is a condition to react to; `since_boot` is a condition to explain a bad show with.
    They are separated because they are answers to genuinely different questions and the raw
    hex answers neither without a table nobody has to hand at the time.
    """
    output = run(["vcgencmd", "get_throttled"])
    if not output or "=" not in output:
        return None
    try:
        word = int(output.split("=", 1)[1], 16)
    except ValueError:
        return None
    return {
        "raw": f"0x{word:x}",
        "now": {name: bool(word >> bit & 1) for name, bit in THROTTLE_FLAGS.items()},
        "since_boot": {
            name: bool(word >> (bit + THROTTLE_LATCHED_SHIFT) & 1)
            for name, bit in THROTTLE_FLAGS.items()
        },
    }


def memory_mb():
    text = read("/proc/meminfo", "")
    fields = {}
    for key in ("MemTotal", "MemAvailable"):
        found = re.search(rf"^{key}:\s+(\d+) kB", text, re.MULTILINE)
        if found:
            fields[key] = round(int(found.group(1)) / 1024)
    if "MemTotal" not in fields or "MemAvailable" not in fields:
        return None
    # Available rather than free: free excludes the page cache, reads alarmingly low on a
    # healthy box, and would have somebody hunting a leak that is not there.
    return {
        "total": fields["MemTotal"],
        "available": fields["MemAvailable"],
        "used": fields["MemTotal"] - fields["MemAvailable"],
    }


def addresses():
    """Interface → address, so a tree tells you the tunnel is up and the modem has a lease."""
    output = run(["ip", "-o", "-4", "addr", "show"])
    if output is None:
        return None
    found = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) >= 4:
            found[fields[1]] = fields[3]
    return found


def disk_free_mb(path="/"):
    try:
        usage = os.statvfs(path)
    except OSError:
        return None
    return round(usage.f_bavail * usage.f_frsize / 1024 / 1024)


def stats():
    return {
        "at": time.time(),
        "uptime_s": round(float(read("/proc/uptime", "0").split()[0])),
        "temperature_c": temperature_c(),
        "throttling": throttling(),
        # Named rather than a three-element list, because `load/1m` says what it is and
        # `load/0` needs a reader who already knows the order.
        "load": dict(zip(("1m", "5m", "15m"), os.getloadavg())),
        "memory_mb": memory_mb(),
        "disk_free_mb": disk_free_mb(),
        "addresses": addresses(),
    }


# ── Main loop ────────────────────────────────────────────────────────────────


def main():
    telemetry = rig_mqtt.Telemetry.connect(SERVICE)

    inventory = specs()
    telemetry.publish("specs", inventory, retain=True)
    log(f"{inventory['model']} — {inventory['os']}, {inventory['cores']} cores, "
        f"{inventory['memory_mb']} MB, python {inventory['python']}")

    # Latched flags are worth a line in the journal the first time they are seen, because a
    # brownout that happened before the last reboot explains a slow show that nobody would
    # otherwise connect to the power supply.
    warned = set()

    while True:
        reading = stats()
        telemetry.publish("stats", reading, retain=True)

        latched = reading["throttling"]["since_boot"] if reading["throttling"] else {}
        for name, raised in latched.items():
            if raised and name not in warned:
                warned.add(name)
                log(f"{name} has occurred since boot ({reading['throttling']['raw']})")

        time.sleep(STATS_INTERVAL_S)


if __name__ == "__main__":
    main()
