#!/usr/bin/env python3
"""Runs ON chaos-pi. Prints a board's serial log, from its first line, until Ctrl-C.

    python3 -I board-log.py <port>

The board is restarted once the port is open, so nothing it prints at boot is lost to the time
it takes to open: on a NodeMCU, RTS drives EN through the USB serial chip, and DTR drives GPIO0,
held high so it boots its firmware and not its bootloader. 74880 baud, the ESP8266 boot ROM's,
which the firmware uses too, so the ROM's reset cause reads in the same log as the firmware.

pyserial is the one esptool's Debian package installs. `attach.sh` runs this from the Mac.
"""

import sys
import time

import serial


def main(path):
    port = serial.Serial()
    port.port = path
    port.baudrate = 74880
    # Set before opening, so the port opens with the board held in reset rather than restarting
    # it once on open and again below.
    port.dtr = False
    port.rts = True
    port.open()
    time.sleep(0.1)
    port.rts = False
    out = sys.stdout.buffer
    try:
        while True:
            out.write(port.read(port.in_waiting or 1))
            out.flush()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(f"usage: {sys.argv[0]} <port>")
    main(sys.argv[1])
