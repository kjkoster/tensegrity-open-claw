#!/usr/bin/env bash
#
# deploy.sh — rsync the sources to claw-pi, then run remote-deploy.sh there to build the
#             brain natively and restart it.
#
#             The MCU firmware is the exception: it is built here, on the Mac, and only the
#             binary is shipped, for remote-deploy.sh to flash onto every attached board.
#
# Run from the repo root on the Mac (riverrain):  ./deploy.sh
# Stops at the first error so you can see how far it got.
#
set -euo pipefail
cd "$(dirname "$0")"

PI="claw-pi"               # ssh alias (key-based)
PI_DIR="tensegrity-open-claw"

step() { printf '\n\033[1m=== %s ===\033[0m\n' "$1"; }

# --- 1. Build MCU firmware locally ----------------------------------------
# Ahead of the rsync, so firmware that does not compile fails here, before anything on the Pi
# has been touched. Inside the crate's own directory because that is what selects its target
# and toolchain: `ball` is not a member of the workspace the Pi builds.
BALL_BINARY="ball/target/riscv32imc-unknown-none-elf/release/ball"

step "build ball -> riscv32imc-unknown-none-elf    (ESP32-C3)"
( cd ball && cargo build --release )

# --- 2. Ship sources to the Pi --------------------------------------------
# The Pi's clock used to be pushed from here, because cargo decides freshness by mtime and
# a lagging Pi clock makes every rsynced source look newer than every build artifact. It
# now runs systemd-timesyncd against the Debian pool (`timedatectl timesync-status`), so
# both ends already agree and the push was only fighting timesyncd for the clock.

# What the Pi needs in order to build and run, and nothing else. Named rather than excluded,
# so adding something is a decision instead of an oversight — the design documents, the
# reference PDFs, the editor config and the git history all stay on the Mac.
#
#   the five crates  the members of the cargo workspace
#   claw.qxw         the claw's patch (what is where) and its scenes
#   fixtures/*.qxf   the definitions (what each fixture's channels mean)
#   stem/           the broker configuration, and whatever else Stem grows
#   etc/            what the deploy installs onto the Pi that belongs to no daemon: the
#                    access point's configuration, the uplink watchdog (its script, its
#                    unit and its timer), the boot configuration, and remote-deploy.sh,
#                    which does the installing
#   attach.sh        the probe-rs wrapper, which re-execs its own copy on the Pi
#
# A daemon's systemd unit travels inside that daemon's directory.
#
# `ball/` is absent on purpose: the Pi flashes that firmware and never builds it, so it gets
# the binary, below, and not the crate.
#
# The workspace and the definitions are not optional: the build reads `claw.qxw` and
# resolves it against `fixtures/`, and a missing definition fails the build deliberately,
# because the Pi has no QLC+ library to fall back on. Shipping them is what makes "save in
# QLC+, deploy" enough to get an edited scene, address or mode onto the rig.
DIRECTORIES=(ball-dmx cortex cortex-build rig-mqtt brain fixtures stem etc)
FILES=(Cargo.toml claw.qxw attach.sh)

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

step "rsync MCU binaries to $PI:$PI_DIR/binaries"
ssh "$PI" "mkdir -p '$PI_DIR/binaries'"
rsync -az "$BALL_BINARY" "$PI:$PI_DIR/binaries/ball"

# --- 2b. Boot configuration -----------------------------------------------
# The rig's settings live in their own file beside config.txt, included from it, so the rest
# of config.txt stays as Raspberry Pi OS ships it and an OS upgrade can rewrite it freely.
#
# The file is synced on every deploy, and the include line is added once if it is missing.
# Neither reboots: a bad boot configuration on a Pi reachable over 4G is a trip to site, so
# when it takes effect stays a person's call.
#
# Plain `rsync -c`, not `-a`: the boot partition is FAT, which has no owners or permissions
# to preserve, and asking for them fails. `--itemize-changes` prints a line only when the file
# actually changed, which is what decides whether a reboot is owed. Nothing here reboots.
step "boot configuration on $PI"
BOOT_DIR="/boot/firmware"
BOOT_CONFIG="config-claw.txt"
INCLUDE_LINE="include $BOOT_CONFIG"

changed=$(rsync -c --itemize-changes --rsync-path="sudo rsync" \
    "etc/$BOOT_CONFIG" "$PI:$BOOT_DIR/$BOOT_CONFIG")
if [ -n "$changed" ]; then
    echo "$BOOT_DIR/$BOOT_CONFIG changed — reboot $PI for it to take effect"
else
    echo "$BOOT_DIR/$BOOT_CONFIG unchanged"
fi

if ssh "$PI" "grep -qxF '$INCLUDE_LINE' $BOOT_DIR/config.txt"; then
    echo "$BOOT_DIR/config.txt includes $BOOT_CONFIG"
else
    # `[all]` first, so the include applies whatever conditional section the file happens to
    # end in.
    ssh "$PI" "printf '\n[all]\n%s\n' '$INCLUDE_LINE' | sudo tee -a $BOOT_DIR/config.txt >/dev/null"
    echo "added '$INCLUDE_LINE' to $BOOT_DIR/config.txt — reboot $PI for it to take effect"
fi

# --- 3. Build, restart and flash on the Pi --------------------------------
step "run remote deploy on $PI"
ssh "$PI" "bash $PI_DIR/etc/remote-deploy.sh"

step "deploy complete"
say "The installation of the Open Claw has completed. Go check it out"
