#!/usr/bin/env bash
#
# logtail.sh — follow the brain's log on claw-pi from here.
#
# Run from anywhere on the Mac:  ./logtail.sh
# Extra arguments go to journalctl, e.g. ./logtail.sh -n 200 for more history.
#
set -euo pipefail

PI="claw-pi"               # ssh alias (key-based)

# -o cat drops journald's own timestamp and host prefix, so the lines read as the brain
# wrote them; pass -o short-precise to get the timestamps back. -t gives the remote journalctl a terminal, so Ctrl-C stops it instead of leaving it
# running on the Pi.
exec ssh -t "$PI" journalctl -u brain -f -o cat "$@"
