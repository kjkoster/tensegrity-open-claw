#!/usr/bin/env bash
#
# attach.sh — show a board's serial log on the Mac, without knowing which of chaos-pi's USB
#             ports it is on.
#
#   ./attach.sh            Wrist
#   ./attach.sh <device>   the board clockwork.dot gives that device's MAC
#
# The board is found on chaos-pi by the MAC its chip reports (pi/chaos-pi/boards.sh), the same
# way the deploy finds the boards it flashes, and restarted so its log starts at its first line.
# Ctrl-C ends it. Runs the copies deploy-chaos-pi.sh shipped, so deploy once before the first
# attach.
#
set -euo pipefail
cd "$(dirname "$0")"

PI="chaos-pi"               # ssh alias (key-based)
PI_DIR="tensegrity-open-claw"

device="${1:-wrist}"
mac="$(python3 -I clockwork/boards.py clockwork.dot | awk -v device="$device" '$2 == device { print $1 }')"
if [ -z "$mac" ]; then
    echo "attach.sh: clockwork.dot gives $device no MAC" >&2
    exit 1
fi

# -t, so the Pi's side has a terminal and Ctrl-C reaches board-log.py.
exec ssh -t "$PI" "port=\$(bash $PI_DIR/pi/chaos-pi/boards.sh $mac) \
    && exec python3 -I $PI_DIR/pi/chaos-pi/board-log.py \"\$port\""
