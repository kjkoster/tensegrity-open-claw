#!/usr/bin/env bash
#
# deploy.sh — rsync the brain source and the QLC+ project to claw-pi, then run
#             remote-deploy.sh there to build brain natively and restart it.
#
# Run from the repo root on the Mac (riverrain):  ./deploy.sh
# Stops at the first error so you can see exactly how far it got.
#
set -euo pipefail
cd "$(dirname "$0")"

PI="claw-pi"               # ssh alias (key-based)

step() { printf '\n\033[1m=== %s ===\033[0m\n' "$1"; }

# --- 1. Ship sources to the Pi --------------------------------------------
# The Pi's clock used to be pushed from here, because cargo decides freshness by mtime and
# a lagging Pi clock makes every rsynced source look newer than every build artifact. It
# now runs systemd-timesyncd against the Debian pool (`timedatectl timesync-status`), so
# both ends already agree and the push was only fighting timesyncd for the clock.

step "rsync brain source to $PI:brain"
# --checksum, not the default size+mtime check: git checkouts and editor saves bump
# source mtimes without changing content, and a bumped mtime would land newer than
# the Pi's build artifacts and trigger a spurious cargo rebuild. Content-based sync
# leaves unchanged files (and their mtimes) untouched, so cargo stays incremental.
rsync -az --checksum --delete --exclude=target brain/ "$PI:brain/"

step "rsync QLC+ project to $PI"
# brain's build.rs reads both halves of the QLC+ project, and they must land beside brain/
# on the Pi because it resolves them as ../open-claw.qxw and ../fixtures:
#
#   open-claw.qxw   the patch (what is where) and the scenes
#   fixtures/*.qxf  the definitions (what each fixture's channels mean)
#
# Neither is optional — a missing definition fails the build, deliberately, because the Pi
# has no QLC+ library to fall back on. Shipping both is what makes "save in QLC+, deploy"
# enough to get an edited scene, address or mode onto the rig.
#
# --delete on fixtures/ so a definition removed here also leaves the Pi. A stale copy
# lingering there would satisfy a build that ought to fail.
rsync -az --checksum open-claw.qxw "$PI:open-claw.qxw"
rsync -az --checksum --delete fixtures/ "$PI:fixtures/"

rsync -az remote-deploy.sh "$PI:remote-deploy.sh"

# --- 2. Build and restart on the Pi ---------------------------------------
step "run remote deploy on $PI"
ssh "$PI" "bash remote-deploy.sh"

step "deploy complete"
say "The installation of the Open Claw has completed. Go check it out"
