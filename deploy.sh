#!/usr/bin/env bash
#
# deploy.sh — rsync the sources to claw-pi, then run remote-deploy.sh there to build both rig
#             binaries natively and restart whichever one the symlink selects.
#
# Run from the repo root on the Mac (riverrain):
#
#   ./deploy.sh                 build, install, restart the rig that is already selected
#   ./deploy.sh mage-brain      … and point the symlink at the mage rig first
#   ./deploy.sh claw-brain      … and point it back at the claw
#
# Naming a rig is the only thing that ever moves the symlink; a plain deploy leaves whatever
# is live exactly where it is. Stops at the first error so you can see how far it got.
#
set -euo pipefail
cd "$(dirname "$0")"

PI="claw-pi"               # ssh alias (key-based)
PI_DIR="tensegrity-open-claw"
RIG="${1:-}"               # empty means "leave the symlink alone"

step() { printf '\n\033[1m=== %s ===\033[0m\n' "$1"; }

# Fail here rather than on the Pi: a typo should not cost a full remote build to discover.
case "$RIG" in
    ""|claw-brain|mage-brain) ;;
    *) echo "unknown rig '$RIG' — expected claw-brain or mage-brain" >&2; exit 1 ;;
esac

# --- 1. Ship sources to the Pi --------------------------------------------
# The Pi's clock used to be pushed from here, because cargo decides freshness by mtime and
# a lagging Pi clock makes every rsynced source look newer than every build artifact. It
# now runs systemd-timesyncd against the Debian pool (`timedatectl timesync-status`), so
# both ends already agree and the push was only fighting timesyncd for the clock.

# What the Pi needs in order to build and run, and nothing else. Named rather than excluded,
# so adding something is a decision instead of an oversight — the design documents, the
# reference PDFs, the editor config and the git history all stay on the Mac.
#
#   the four crates  the members of the cargo workspace
#   claw.qxw         the claw's patch (what is where) and its scenes
#   mage.qxw         the mage rig's patch
#   fixtures/*.qxf   the definitions (what each fixture's channels mean)
#   eyeball/         the vision daemon and its unit
#   stem/            the broker configuration, and whatever else Stem grows
#   brain.service    installed as the systemd unit
#
# The workspaces and the definitions are not optional: each rig's build reads its own `.qxw`
# and resolves it against `fixtures/`, and a missing definition fails the build deliberately,
# because the Pi has no QLC+ library to fall back on. Shipping them is what makes "save in
# QLC+, deploy" enough to get an edited scene, address or mode onto the rig.
DIRECTORIES=(cortex cortex-build claw-brain mage-brain fixtures eyeball stem)
FILES=(Cargo.toml claw.qxw mage.qxw brain.service remote-deploy.sh)

step "rsync sources to $PI:$PI_DIR"
# --checksum, not the default size+mtime check: git checkouts and editor saves bump source
# mtimes without changing content, and a bumped mtime would land newer than the Pi's build
# artifacts and trigger a spurious cargo rebuild. Content-based sync leaves unchanged files
# (and their mtimes) untouched, so cargo stays incremental.
#
# --delete per directory, so a file removed here also leaves the Pi: a stale fixture
# definition lingering there would satisfy a build that ought to fail.
#
# --exclude=target because build output is never shipped: the Pi builds natively, and a crate
# directory can still carry a stray target/ from before the cargo workspace put one at the
# root. Excluding it also protects the Pi's own from --delete.
for directory in "${DIRECTORIES[@]}"; do
    rsync -az --checksum --delete --exclude=target "$directory/" "$PI:$PI_DIR/$directory/"
done
rsync -az --checksum "${FILES[@]}" "$PI:$PI_DIR/"

# --- 2. Build and restart on the Pi ---------------------------------------
step "run remote deploy on $PI"
ssh "$PI" "bash $PI_DIR/remote-deploy.sh $RIG"

step "deploy complete"
say "The installation of the Open Claw has completed. Go check it out"
