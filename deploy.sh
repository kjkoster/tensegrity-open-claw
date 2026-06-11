#!/usr/bin/env bash
#
# deploy.sh — build every project for its target, ship the binaries to claw-pi,
#             restart the brain daemon, and flash every attached MCU.
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

# --- 1. build each project for its own target -----------------------------
step "build brain    -> aarch64-unknown-linux-gnu  (Raspberry Pi 3B)"
# std binary cross-linked for the Pi; zig provides the cross linker on macOS.
( cd brain && cargo zigbuild --release --target aarch64-unknown-linux-gnu )

step "build ponytail -> xtensa-esp32s3-none-elf    (XIAO-ESP32-S3)"
source "$ESP_ENV"
( cd ponytail && cargo build --release )

# --- 2. Collect the binaries into a staging dir ---------------------------
step "stage binaries"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

cp brain/target/aarch64-unknown-linux-gnu/release/brain     "$STAGE/brain"
cp brain/brain.service                                      "$STAGE/brain.service"
cp ponytail/target/xtensa-esp32s3-none-elf/release/ponytail "$STAGE/ponytail"

ls -l "$STAGE"

# --- 3. Ship them to the Pi -----------------------------------------------
step "rsync binaries to $PI:$REMOTE_DIR"
ssh "$PI" "mkdir -p '$REMOTE_DIR'"
rsync -az "$STAGE"/ "$PI:$REMOTE_DIR/"
rsync -az remote-deploy.sh "$PI:$REMOTE_DIR/remote-deploy.sh"

# --- 4. Deploy + flash on the Pi ------------------------------------------
step "run remote deploy on $PI"
ssh "$PI" "bash '$REMOTE_DIR/remote-deploy.sh'"

step "deploy complete"
