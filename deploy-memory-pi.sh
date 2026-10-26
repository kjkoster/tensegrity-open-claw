#!/usr/bin/env bash
#
# deploy-memory-pi.sh — rsync memory-pi's configuration to it, then run remote-deploy.sh there
#                       to install it.
#
# Run from the repo root on the Mac (riverrain):  ./deploy-memory-pi.sh
# Stops at the first error so you can see how far it got.
#
# memory-pi alone: deploy.sh is claw-pi's and never touches it, so either Pi deploys while the
# other is down.
#
set -euo pipefail
cd "$(dirname "$0")"

PI="memory-pi"             # ssh alias (key-based)
PI_DIR="tensegrity-open-claw"

step() { printf '\n\033[1m=== %s ===\033[0m\n' "$1"; }

# --- Ship configuration to the Pi -----------------------------------------
#   etc-memory-pi/      memory-pi's own configuration, and remote-deploy.sh, which installs it
#   etc/claw-wifi.env   the rig's WiFi, which claw-pi's access point is built from too
#
# --delete, so a file removed here also leaves the Pi rather than lingering there to be
# installed by a remote-deploy.sh that no longer knows about it.
#
# The directories are made first, because rsync creates only the last level of a missing
# destination and a fresh Pi has none of them.
step "rsync configuration to $PI:$PI_DIR"
ssh "$PI" "mkdir -p $PI_DIR/etc-memory-pi $PI_DIR/etc"
rsync -az --checksum --delete "etc-memory-pi/" "$PI:$PI_DIR/etc-memory-pi/"
rsync -az --checksum "etc/claw-wifi.env" "$PI:$PI_DIR/etc/"

# --- Install on the Pi ----------------------------------------------------
step "run remote deploy on $PI"
ssh "$PI" "bash $PI_DIR/etc-memory-pi/remote-deploy.sh"

step "deploy complete"
