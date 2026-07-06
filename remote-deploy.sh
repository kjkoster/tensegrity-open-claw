#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON claw-pi (pushed there by deploy.sh).
# Builds brain natively, installs/restarts the brain daemon, then flashes
# every attached MCU based on its /dev/serial/by-id name.
# Absent boards are simply skipped.
#
set -euo pipefail

BIN_DIR="$HOME/binaries"
. $HOME/.cargo/env

step() { printf '\n\033[1m--- %s ---\033[0m\n' "$1"; }

# ── Device-to-binary table ────────────────────────────────────────────────────
# Map each board's full /dev/serial/by-id suffix to the binary it should receive.
# Add one entry per board; the script fails if a connected device is not listed.
#
# To find a board's ID: ls /dev/serial/by-id/  (while the board is plugged in)
#
declare -A DEVICE_MAP=(
  ["usb-Espressif_USB_JTAG_serial_debug_unit_AC:A7:04:2C:4F:D8-if00"]="ponytail"
  ["usb-Espressif_USB_JTAG_serial_debug_unit_DC:B4:D9:3B:B1:A4-if00"]="ponytail"
  ["usb-Espressif_USB_JTAG_serial_debug_unit_AC:A7:04:2C:50:FC-if00"]="ponytail"
  ["usb-Espressif_USB_JTAG_serial_debug_unit_1C:DB:D4:75:AB:7C-if00"]="ponytail"
)
# ─────────────────────────────────────────────────────────────────────────────

# --- 1. build brain natively on the Pi ------------------------------------
step "build brain"
( cd "$HOME/brain" && cargo build --release )

# --- 2. brain daemon -------------------------------------------------------
step "install and restart brain daemon"
sudo install -m 0755 "$HOME/brain/target/release/brain" /usr/local/bin/brain
sudo install -m 0644 "$HOME/brain/brain.service" /etc/systemd/system/brain.service
echo "brain daemon installed"
sudo systemctl daemon-reload
sudo systemctl enable brain
sudo systemctl restart brain
echo "brain daemon restarted"

# --- 3. flash attached microcontrollers ------------------------------------
step "flash attached microcontrollers"
shopt -s nullglob
found=0
for dev in /dev/serial/by-id/*; do
  found=1
  name="$(basename "$dev")"
  if [[ -v DEVICE_MAP["$name"] ]]; then
    binary="${DEVICE_MAP[$name]}"
    echo ">> $name  -> $binary"
    espflash flash --port "$dev" "$BIN_DIR/$binary"
  else
    echo "ERROR: unknown device $name — add it to DEVICE_MAP in remote-deploy.sh" >&2
    exit 1
  fi
done
[ "$found" -eq 0 ] && echo "no MCUs attached; nothing to flash."

step "remote deploy complete"
