#!/usr/bin/env bash
#
# deploy-memory-pi.sh — rsync memory-pi's configuration to it, then run remote-deploy.sh there
#                       to install it.
#
# Run from the repo root on the Mac (riverrain):  ./deploy-memory-pi.sh
# Stops at the first error so you can see how far it got.
#
# memory-pi alone: deploy-chaos-pi.sh is claw-pi's and never touches it, so either Pi deploys
# while the other is down.
#
set -euo pipefail
cd "$(dirname "$0")"

PI="memory-pi"             # ssh alias (key-based)
PI_DIR="tensegrity-open-claw"

step() { printf '\n\033[1m=== %s ===\033[0m\n' "$1"; }

# --- Render the Pi's network configuration ---------------------------------
# Its connection to the access point is a template, filled from what `clockwork.dot` says
# about memory-pi, so the WiFi and the addresses are written once, there. Rendered afresh
# under the git-ignored `target/`, before anything reaches the Pi, so a dot that does not
# render leaves it untouched.
step "render $PI's configuration from clockwork.dot"
RENDERED="target/$PI"
rm -rf "$RENDERED"
python3 -I clockwork/pi_network.py clockwork.dot memory_pi "$RENDERED" \
    pi/memory-pi/claw.nmconnection

# --- Ship configuration to the Pi -----------------------------------------
#   pi/memory-pi/       memory-pi's own configuration and daemons, and remote-deploy.sh, which
#                       installs them; not its WiFi profile's template
#   rendered/           what was rendered from the dot above: its WiFi profile, which holds the
#                       passphrase, so the owner's alone
#   pi/deploy-lib.sh    what both Pis' remote deploys share
#   stem/rig_*.py       Stem's shared modules: the MQTT conventions and the board readers
#   stem/stem.conf      Stem's broker configuration: the broker runs on memory-pi
#
# --delete, so a file removed here also leaves the Pi rather than lingering there to be
# installed by a remote-deploy.sh that no longer knows about it.
#
# The directories are made first, because rsync creates only the last level of a missing
# destination and a fresh Pi has none of them.
step "rsync configuration to $PI:$PI_DIR"
ssh "$PI" "mkdir -p $PI_DIR/pi/memory-pi $PI_DIR/stem $PI_DIR/rendered"
rsync -az --checksum --delete --exclude='*.nmconnection' --exclude=__pycache__ \
    "pi/memory-pi/" "$PI:$PI_DIR/pi/memory-pi/"
rsync -az --checksum --delete --chmod=D700,F600 "$RENDERED/" "$PI:$PI_DIR/rendered/"
rsync -az --checksum "pi/deploy-lib.sh" "$PI:$PI_DIR/pi/"
rsync -az --checksum "stem/rig_mqtt.py" "stem/rig_board.py" "stem/stem.conf" "$PI:$PI_DIR/stem/"

# --- Install on the Pi ----------------------------------------------------
step "run remote deploy on $PI"
ssh "$PI" "bash $PI_DIR/pi/memory-pi/remote-deploy.sh"

step "deploy complete"
say "The installation of the memory pi has completed. Go check it out"
