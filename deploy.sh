#!/usr/bin/env bash
#
# deploy.sh — build the brain here for claw-pi, rsync it and what the Pi runs beside it, then
#             run remote-deploy.sh there to install it and restart what changed.
#
# Run from the repo root on the Mac (riverrain):  ./deploy.sh
# Stops at the first error so you can see how far it got.
#
set -euo pipefail
cd "$(dirname "$0")"

PI="claw-pi"               # ssh alias (key-based)
PI_DIR="tensegrity-open-claw"

# The Pi's target: 64-bit ARM Linux, linked statically against musl, so the binary carries its
# own C library. `.cargo/config.toml` names the linker; `rustup target add` it once per Mac.
TARGET="aarch64-unknown-linux-musl"

step() { printf '\n\033[1m=== %s ===\033[0m\n' "$1"; }

# --- Draw the network ------------------------------------------------------
# `clockwork.svg` is drawn from `clockwork.dot` on every deploy, so the picture never lags the
# graph the build reads. It stays on the Mac: it is git-ignored and nothing ships it.
step "draw clockwork.svg"
dot -Tsvg clockwork.dot -o clockwork.svg

# --- Build the brain -------------------------------------------------------
# Here rather than on the Pi: the Mac builds in seconds what the Pi took minutes over, and the
# Pi then needs no toolchain, no crate index and no sources. The build reads `clockwork.qxw` and
# `fixtures/` and resolves the show table, so "save in QLC+, deploy" still gets an edited
# patch onto the rig. First, so a build that fails leaves the Pi untouched.
step "build brain for $TARGET"
cargo build --release --target "$TARGET"

# --- Ship to the Pi --------------------------------------------------------
# What the Pi runs, and nothing else. Named rather than excluded, so adding something is a
# decision instead of an oversight — the sources, the design documents, the reference PDFs
# and the git history all stay on the Mac.
#
#   brain/          the binary and its systemd unit
#   stem/           Stem's daemon, the Python modules every health daemon shares, and their
#                    unit and requirements; not the `stem` crate, which is built in here
#   etc/            what the deploy installs onto the Pi that belongs to no daemon: the
#                    access point's and the wired segment's configuration, the uplink
#                    watchdog (its script, its unit and its timer), the boot configuration,
#                    and remote-deploy.sh, which does the installing
#
# --checksum, not the default size+mtime check, so a file whose contents did not change is not
# copied again. --delete for the directories, so a file removed here also leaves the Pi.
step "rsync to $PI:$PI_DIR"
# rsync creates only the last directory of a destination, so a freshly set-up Pi needs these.
ssh "$PI" "mkdir -p $PI_DIR/brain $PI_DIR/stem $PI_DIR/etc"
rsync -az --checksum \
    "brain/brain.service" "target/$TARGET/release/brain" "$PI:$PI_DIR/brain/"
rsync -az --checksum --delete \
    --exclude=src --exclude=Cargo.toml --exclude=target --exclude=__pycache__ \
    "stem/" "$PI:$PI_DIR/stem/"
rsync -az --checksum --delete "etc/" "$PI:$PI_DIR/etc/"

# --- Boot configuration ---------------------------------------------------
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

# --- Install and restart on the Pi ----------------------------------------
step "run remote deploy on $PI"
ssh "$PI" "bash $PI_DIR/etc/remote-deploy.sh"

step "deploy complete"
say "The installation of the Open Claw has completed. Go check it out"
