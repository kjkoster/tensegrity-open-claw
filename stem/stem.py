#!/usr/bin/env python3
"""Stem: what the machine is, and how it is holding up.

The health daemon, and deliberately the least capable process on the rig. It reads, it
publishes, and it does nothing else — no watchdog petting, no restarting, no re-dialling.
Those duties belong elsewhere: systemd's watchdogs keep the brain and the OS alive, and the
4G watchdog's timer keeps the uplink. Keeping them out means Stem's death costs telemetry and
never uptime.

Two subtrees, both retained, because both answer questions asked after the interesting moment:

    sys/stem/…     the hardware: board, cores, CPU frequency and installed RAM — written once
    stem/…         how it is doing — temperature, throttling, memory, load, uplink

One field per topic: `stem/temperature_c`, `stem/throttling/since_boot/under_voltage`. Nested
readings become nested topics, so a subscriber can take a single number without knowing
anything about the shape it arrived in.

The throttle word is the reason Stem reports how the Pi is doing at all. A Pi that browns out under a PoE
injector or cooks in a sealed cabinet reports both as flags that latch until reboot, and
finding them by SSH after a bad show is finding them too late.

Liveness is not published here. Every daemon publishes its own on `health/<service>` with a
will behind it, so a crashed process says so without Stem having to notice — including a
crashed Stem.

What any Pi has is read through `rig_board`, which memory-pi's health daemon shares; what is
claw-pi's alone — the access point's stations and the 4G modem — is read here.
"""

import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rig_board
import rig_mqtt

SERVICE = "stem"
# Slow on purpose. Nothing here moves fast enough to be worth sampling often, and a telemetry
# tree that scrolls is one nobody reads.
STATS_INTERVAL_S = float(os.environ.get("STEM_STATS_INTERVAL_S", "30"))

# The access point's interface, whose stations' signal is what "WiFi strength" means on a Pi
# that is the access point and has no signal of its own.
AP_INTERFACE = "wlan0"


def log(message):
    print(f"stem: {message}", file=sys.stderr, flush=True)


def wifi_stations():
    """Station MAC → how strongly the access point hears it, in dBm.

    A wireless fixture that reports its own signal hears the same link from the other end; the
    two disagreeing by a lot points at an antenna rather than at distance.
    """
    stations = {}
    station = None
    for line in rig_board.run(["iw", "dev", AP_INTERFACE, "station", "dump"]).splitlines():
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
    # The dongle can drop off the bus, or be reset by the 4G watchdog, between the listing
    # and the query: a modem that is not there reads as nothing, and its topics clear.
    try:
        output = rig_board.run(["mmcli", *arguments, "-K"])
    except subprocess.CalledProcessError:
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
    numbers a modem afresh each time it reappears, and a dongle that the 4G watchdog has reset
    or re-enumerated comes back as the next one.

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


def stats():
    return {
        **rig_board.board_stats(),
        "wifi": {"stations": wifi_stations()},
        "4g": cellular(),
    }


# ── Main loop ────────────────────────────────────────────────────────────────


def main():
    telemetry = rig_mqtt.Telemetry.connect(SERVICE)

    hardware = rig_board.system()
    telemetry.publish_sys(hardware)
    log(f"{hardware['board']}, {hardware['cores']} cores, {hardware['ram_kb']} KB RAM")

    warned = set()
    # Stations seen last time, so one that leaves has its topics cleared rather than left
    # standing at its last signal, which would read as a station that is still there.
    stations = set()

    while True:
        reading = stats()
        for name, value in reading.items():
            telemetry.publish(name, value, retain=True)

        present = set(reading["wifi"]["stations"])
        for gone in stations - present:
            telemetry.publish(f"wifi/stations/{gone}/signal_dbm", None, retain=True)
        stations = present

        rig_board.warn_latched(reading, warned, log)

        time.sleep(STATS_INTERVAL_S)


if __name__ == "__main__":
    main()
