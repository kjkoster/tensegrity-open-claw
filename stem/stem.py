#!/usr/bin/env python3
"""Stem: what the machine is, and how it is holding up.

The health daemon, and deliberately the least capable process on the rig. It reads, it
publishes, and it does nothing else — no watchdog petting, no restarting, no re-dialling.
Those duties are designed but not built here yet, and keeping them out means Stem's death
costs telemetry and never uptime.

Three subtrees, all retained, because all answer questions asked after the interesting moment:

    stem/sys/…     the hardware: board, cores, CPU frequency and installed RAM — written once
    stem/specs/…   the rest of what this machine is — written once, never changes while it runs
    stem/stats/…   how it is doing — temperature, throttling, memory, load, uplink

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

# The access point's interface, whose stations' signal is what "WiFi strength" means on a Pi
# that is the access point and has no signal of its own.
AP_INTERFACE = "wlan0"


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


def system():
    """The hardware, under the `sys` subtree every part of the rig publishes its own under.
    Published once, retained, like the specs. Stem is not compiled, so unlike the Rust parts it
    has no target triplet to report.
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
    new-style code (bit 23) carries the size in bits 20–22, as 256 MB doubled that many times;
    an old-style code does not say, and is left out.
    """
    revision = cpu_field("Revision")
    try:
        code = int(revision, 16)
    except (TypeError, ValueError):
        return None
    if not code & (1 << 23):
        return None
    return (256 * 1024) << ((code >> 20) & 0x7)


def cpu_frequency_mhz():
    """The top frequency the CPU is allowed to run at, which is what it can sustain when the
    rig is busy, rather than wherever the governor happens to have it at the moment."""
    khz = read("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
    return int(khz) // 1000 if khz and khz.isdigit() else None


def total_memory_mb():
    found = re.search(r"^MemTotal:\s+(\d+) kB", read("/proc/meminfo", ""), re.MULTILINE)
    return round(int(found.group(1)) / 1024) if found else None


def os_name():
    found = re.search(r'^PRETTY_NAME="?([^"\n]+)"?', read("/etc/os-release", ""), re.MULTILINE)
    return found.group(1) if found else None


def specs():
    """What this machine is. Published once, retained, and never republished.

    It exists because the numbers that matter downstream — what a core can sustain, whether
    a wheel exists for this interpreter — are answers to questions about this exact board, and reading them out of a telemetry tree beats an SSH session and a remembered
    command line.
    """
    return {
        "revision": cpu_field("Revision"),
        "serial": cpu_field("Serial"),
        "kernel": platform.release(),
        "architecture": platform.machine(),
        "word_size": 64 if sys.maxsize > 2**32 else 32,
        "os": os_name(),
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


def wifi_stations():
    """Station MAC → how strongly the access point hears it, in dBm.

    The ball's own `ball/wifi/rssi_dbm` is the same link heard from the other end; the two
    disagreeing by a lot points at an antenna rather than at distance.
    """
    output = run(["iw", "dev", AP_INTERFACE, "station", "dump"])
    if output is None:
        return None
    stations = {}
    station = None
    for line in output.splitlines():
        found = re.match(r"Station ([0-9a-f:]{17})", line)
        if found:
            station = found.group(1)
            continue
        found = re.match(r"\s*signal:\s*(-?\d+)", line)
        if found and station:
            stations[station] = {"signal_dbm": int(found.group(1))}
    return stations


def mmcli(*arguments):
    """`mmcli … -K` as key → value, with ModemManager's `--` for "nothing" left out."""
    output = run(["mmcli", *arguments, "-K"])
    if output is None:
        return {}
    fields = {}
    for line in output.splitlines():
        key, separator, value = line.partition(" : ")
        value = value.strip()
        if separator and value != "--":
            fields[key.strip()] = value
    return fields


def cellular():
    """The uplink modem as ModemManager sees it.

    Looked up by its listing on every read rather than by a fixed number, because ModemManager
    numbers a modem afresh each time it reappears, and a dongle that has been power-cycled by
    the watchdog comes back as the next one.

    No dBm figures: this modem has no extended signal support, so ModemManager's 0–100 quality
    is all there is. Every key is always present, so a modem that drops out clears its topics
    instead of leaving the last reading standing.
    """
    path = mmcli("-L").get("modem-list.value[1]")
    modem = mmcli("-m", path) if path else {}
    quality = modem.get("modem.generic.signal-quality.value")
    recent = modem.get("modem.generic.signal-quality.recent")
    return {
        "status": modem.get("modem.generic.state"),
        "network": modem.get("modem.generic.access-technologies.value[1]"),
        "registration": modem.get("modem.3gpp.registration-state"),
        "operator": modem.get("modem.3gpp.operator-name"),
        "signal_quality_pct": int(quality) if quality and quality.isdigit() else None,
        # ModemManager's own word on whether the quality is a fresh reading or a held one.
        "signal_quality_recent": recent == "yes" if recent else None,
    }


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
        "wifi": {"stations": wifi_stations()},
        "4g": cellular(),
    }


# ── Main loop ────────────────────────────────────────────────────────────────


def main():
    telemetry = rig_mqtt.Telemetry.connect(SERVICE)

    hardware = system()
    telemetry.publish("sys", hardware, retain=True)
    inventory = specs()
    telemetry.publish("specs", inventory, retain=True)
    log(f"{hardware['board']} — {inventory['os']}, {hardware['cores']} cores, "
        f"{inventory['memory_mb']} MB, python {inventory['python']}")

    # Latched flags are worth a line in the journal the first time they are seen, because a
    # brownout that happened before the last reboot explains a slow show that nobody would
    # otherwise connect to the power supply.
    warned = set()
    # Stations seen last time, so one that leaves has its topics cleared rather than left
    # standing at its last signal, which would read as a station that is still there.
    stations = set()

    while True:
        reading = stats()
        telemetry.publish("stats", reading, retain=True)

        present = set(reading["wifi"]["stations"] or {})
        for gone in stations - present:
            telemetry.publish(f"stats/wifi/stations/{gone}/signal_dbm", None, retain=True)
        stations = present

        latched = reading["throttling"]["since_boot"] if reading["throttling"] else {}
        for name, raised in latched.items():
            if raised and name not in warned:
                warned.add(name)
                log(f"{name} has occurred since boot ({reading['throttling']['raw']})")

        time.sleep(STATS_INTERVAL_S)


if __name__ == "__main__":
    main()
