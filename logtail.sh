#!/usr/bin/env bash
#
# logtail.sh — follow the brain's log on claw-pi from here.
#
# Run from anywhere on the Mac:  ./logtail.sh
# Extra arguments go to journalctl, e.g. ./logtail.sh -n 200 for more history.
#
set -euo pipefail

PI="claw-pi"               # ssh alias (key-based)

# Timestamped to the millisecond, in the rig's local time: the Pi's clock is UTC, and an hour
# off from every time the show is written in reads wrong. Pass -o cat for the bare lines.
# -t gives the remote journalctl a terminal, so Ctrl-C stops it instead of leaving it running
# on the Pi.
exec ssh -t "$PI" TZ=Europe/Amsterdam journalctl -u brain -f -o short-precise "$@"
