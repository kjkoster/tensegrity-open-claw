#!/usr/bin/env python3
"""Prints the board table: every device `clockwork.dot` gives a MAC, as
`<mac> <device> <chip>`, one line each.

    python3 -I clockwork/boards.py <clockwork.dot>

`deploy-chaos-pi.sh` ships it to `claw-pi`, where a board on USB is known by the MAC its chip
reports and flashed with `binaries/<device>.bin`, telling esptool the chip rather than have it
guess; `attach.sh` finds a board's MAC in it. The MAC is the one the chip reports to
`esptool read_mac`, which on an ESP8266 is also the one it joins the WiFi with, so it is
written in the dot beside the board's address.
"""

import pathlib
import sys

# The dot's reader, beside this file. Put on the path by hand: run with -I, Python adds no
# directory of its own, not even this one.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from dot import Dot, fail, macs  # noqa: E402


def main(dot_path):
    for mac, (device, _, chip) in sorted(macs(Dot(dot_path)).items()):
        print(mac, device, chip)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        fail(__doc__.strip().splitlines()[3].strip())
    main(sys.argv[1])
