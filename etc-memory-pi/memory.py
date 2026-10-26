#!/usr/bin/env python3
"""Memory's health daemon: what memory-pi is, and how it is holding up.

Stem's counterpart on memory-pi, and as deliberately limited: it reads, it publishes, and it
does nothing else. It is a process of its own rather than part of the recorder so that it keeps
reporting when the recorder or Greptime is what has gone wrong.

Two subtrees, both retained, as Stem's:

    sys/memory/…   the hardware: board, cores, CPU frequency and installed RAM — written once
    memory/…       how it is doing — temperature, throttling, memory, load, disks, its WiFi

What any Pi has is read through `rig_board`, which Stem shares; what is memory-pi's alone — the
SSD and its own end of the WiFi link — is read here.
"""

import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rig_board
import rig_mqtt

SERVICE = "memory"
# Stem's cadence, for the same reason: nothing here moves fast enough to sample more often.
STATS_INTERVAL_S = float(os.environ.get("MEMORY_STATS_INTERVAL_S", "30"))

TANK = "/tank"
WIFI_INTERFACE = "wlan0"


def log(message):
    print(f"memory: {message}", file=sys.stderr, flush=True)


def tank_free_mb():
    """Free space on the SSD, or None while it is not mounted.

    Checked first because /tank without the SSD is a directory on the SD card, and its free
    space would read as the SSD's.
    """
    if not os.path.ismount(TANK):
        return None
    return rig_board.disk_free_mb(TANK)


def wifi_signal_dbm():
    """How strongly memory-pi hears the access point. Stem reports the same link from claw-pi's
    end, as this Pi's station; the two disagreeing by a lot points at an antenna."""
    output = rig_board.run(["iw", "dev", WIFI_INTERFACE, "link"])
    if output is None:
        return None
    found = re.search(r"^\s*signal:\s*(-?\d+)", output, re.MULTILINE)
    return int(found.group(1)) if found else None


def stats():
    return {
        **rig_board.board_stats(),
        "tank_free_mb": tank_free_mb(),
        "wifi": {"signal_dbm": wifi_signal_dbm()},
    }


def main():
    telemetry = rig_mqtt.Telemetry.connect(SERVICE)

    hardware = rig_board.system()
    telemetry.publish_sys(hardware)
    log(f"{hardware['board']}, {hardware['cores']} cores, {hardware['ram_kb']} KB RAM")

    warned = set()

    while True:
        reading = stats()
        for name, value in reading.items():
            telemetry.publish(name, value, retain=True)

        rig_board.warn_latched(reading, warned, log)

        time.sleep(STATS_INTERVAL_S)


if __name__ == "__main__":
    main()
