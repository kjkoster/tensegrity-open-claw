#!/usr/bin/env bash
#
# deploy.sh — build MCU firmware locally, rsync brain source + MCU binaries to
#             claw-pi, then run remote-deploy.sh there to build brain natively
#             and flash every attached MCU.
#
# Run from the repo root on the Mac (riverrain):  ./deploy.sh
# Stops at the first error so you can see exactly how far it got.
#
set -euo pipefail
cd "$(dirname "$0")"

PI="claw-pi"               # ssh alias (key-based)
REMOTE_DIR="binaries"      # ~/binaries on the Pi
ESP_ENV="./export-esp.sh"  # written by espup: LIBCLANG_PATH + xtensa gcc on PATH

step() { printf '\n\033[1m=== %s ===\033[0m\n' "$1"; }

# --- 1. build MCU firmware locally ----------------------------------------
step "build ponytail -> xtensa-esp32s3-none-elf    (XIAO-ESP32-S3)"
source "$ESP_ENV"
( cd ponytail && cargo build --release )

# --- 2. Collect MCU binaries into a staging dir ---------------------------
step "stage binaries"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

cp ponytail/target/xtensa-esp32s3-none-elf/release/ponytail "$STAGE/ponytail"

ls -l "$STAGE"

# --- 3. Ship sources and binaries to the Pi --------------------------------
step "sync clock on $PI (it has no NTP/RTC, so its clock drifts behind)"
# cargo decides freshness by mtime: rsync stamps brain sources with the Mac's
# (correct) mtimes, but the Pi builds artifacts with its own clock. If the Pi clock
# lags the Mac, every source looks newer than every artifact and cargo rebuilds the
# world. Push the Mac's UTC time to the Pi so both ends share one clock.
ssh "$PI" "sudo date -u -s '$(date -u '+%Y-%m-%d %H:%M:%S')'" >/dev/null

step "rsync brain source to $PI:brain"
# --checksum, not the default size+mtime check: git checkouts and editor saves bump
# source mtimes without changing content, and a bumped mtime would land newer than
# the Pi's build artifacts and trigger a spurious cargo rebuild. Content-based sync
# leaves unchanged files (and their mtimes) untouched, so cargo stays incremental.
rsync -az --checksum --delete --exclude=target brain/ "$PI:brain/"

step "rsync binaries to $PI:$REMOTE_DIR"
ssh "$PI" "mkdir -p '$REMOTE_DIR'"
rsync -az "$STAGE"/ "$PI:$REMOTE_DIR/"
rsync -az remote-deploy.sh "$PI:$REMOTE_DIR/remote-deploy.sh"

# --- 4. Deploy + flash on the Pi ------------------------------------------
step "run remote deploy on $PI"
ssh "$PI" "bash '$REMOTE_DIR/remote-deploy.sh'"

step "deploy complete"
