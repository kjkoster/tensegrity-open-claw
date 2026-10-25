#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON claw-pi (pushed there by deploy.sh).
# Builds the brain natively, installs it and restarts it, then flashes every attached MCU
# based on its /dev/serial/by-id name.
#
set -euo pipefail

. $HOME/.cargo/env

# Where the sources landed, asked of this script's own location rather than spelled out. A
# second copy of the path here could disagree with the one deploy.sh rsyncs to, and the
# result would be a clean deploy that rebuilt and reinstalled the previous sources.
REPO="$(cd "$(dirname "$0")/.." && pwd)"

step() { printf '\n\033[1m--- %s ---\033[0m\n' "$1"; }

# ── Device-to-binary table ────────────────────────────────────────────────────
# Map each board's full /dev/serial/by-id name to the binary it should receive.
# Add one entry per board; the script fails if a connected device is not listed.
#
# To find a board's ID: ls /dev/serial/by-id/  (while the board is plugged in)
#
declare -A DEVICE_MAP=(
  ["usb-Espressif_USB_JTAG_serial_debug_unit_40:4C:CA:FB:79:10-if00"]="ball"
)
# ─────────────────────────────────────────────────────────────────────────────

# Each Python daemon gets its own venv and its own pip install, so its requirements.txt is the
# only statement of what it needs and the deploy is what acts on it. Separate venvs rather than
# one shared: Stem has to keep reporting when another daemon is exactly what has gone wrong,
# which it cannot do while sharing that daemon's dependency tree.
#
# Created with --system-site-packages, so anything already present from apt stays visible as a
# floor under whatever pip resolves.
python_daemon() {
    local venv="$1" requirements="$2"
    if [ ! -x "$venv/bin/python3" ]; then
        sudo mkdir -p "$(dirname "$venv")"
        sudo python3 -m venv --system-site-packages "$venv"
        echo "created $venv"
    fi
    # Not quiet, and not tolerated: a deploy that cannot install what a daemon imports has
    # failed, and finding that out here beats finding it in a crash loop.
    #
    # No --upgrade either. With it, every deploy re-queries the index for newer versions of
    # everything; without it, pip sees the requirements already satisfied and does not touch
    # the network at all. This Pi reaches PyPI over a 4G modem, so that is the difference
    # between a deploy that needs the uplink and one that does not.
    sudo "$venv/bin/pip" install -r "$requirements"
}

# --- 1. 4G uplink watchdog -------------------------------------------------
# Ahead of everything else, for the same reason the broker goes ahead of the build and a
# stronger one: this is what gets the Pi back onto the network after the modem wedges, and the
# rig is a long way from the person who would otherwise have to walk to it. A deploy that dies
# at the build should still have installed the way back in.
#
# The script carries its own configuration — the modem's USB vendor, the ping targets, the
# tunnel address and the thresholds — and finds the modem itself on every run, so nothing in it
# names a port, a hub or an interface. That makes this copy the source of truth for those
# values: anything retuned on the Pi and not carried back here is overwritten by the next
# deploy.
#
# Only the timer is enabled. The service is `oneshot` and deliberately has no `[Install]`
# section — something has to decide when the check runs, and that is the timer's whole job.
step "install 4G uplink watchdog"
sudo install -m 0755 "$REPO/etc/4g-watchdog.sh" /usr/local/sbin/4g-watchdog.sh
sudo install -m 0644 "$REPO/etc/4g-watchdog.service" /etc/systemd/system/4g-watchdog.service
sudo install -m 0644 "$REPO/etc/4g-watchdog.timer" /etc/systemd/system/4g-watchdog.timer
sudo systemctl daemon-reload
sudo systemctl enable 4g-watchdog.timer
# Restarted and not merely enabled, so an edited schedule takes effect on this deploy rather
# than at whatever reboot happens next. The timer only: the service is the check itself, and
# firing it from here would have it judge an uplink in the middle of being deployed over.
if sudo systemctl restart 4g-watchdog.timer; then
    echo "4G watchdog timer running"
else
    echo "WARNING: 4g-watchdog.timer did not start — journalctl -u 4g-watchdog.timer -n 40"
fi

# --- 2. WiFi access point --------------------------------------------------
# The versioned configuration replaces whatever is on the Pi, so a setting changed there by
# hand does not survive a deploy. 0600 because it holds the passphrase.
#
# Restarted on every deploy, changed or not. That drops every station for a few seconds, and
# it is also what clears the access point's memory of them before the boards are flashed
# further down, which restarts them without their saying goodbye.
step "install and restart WiFi access point"
sudo install -m 0600 "$REPO/etc/hostapd.conf" /etc/hostapd/hostapd.conf
sudo systemctl restart hostapd

# --- 3. Stem's broker ------------------------------------------------------
# Before anything that reports to it. Every daemon on this rig holds its own client
# connection and announces itself on `health/`, so the broker is not one more service among
# them — it is the thing that lets the rest of this script be watched while it runs. Bringing
# it up ahead of the build also means a build that fails leaves the rig's telemetry intact
# rather than taking it down with the deploy.
#
# The config is versioned here and the package is not installed from here: `apt install
# mosquitto` is a one-time act on a new Pi, while this file changes with the rig. A Pi without
# the package is a Pi whose rig still has to come up, so its absence is reported rather than
# repaired — telemetry is observability, and observability is never a reason to fail a deploy.
step "install Stem's broker configuration"
if [ -d /etc/mosquitto/conf.d ]; then
    sudo install -m 0644 "$REPO/stem/stem.conf" /etc/mosquitto/conf.d/stem.conf
    sudo systemctl enable mosquitto
    if sudo systemctl restart mosquitto; then
        echo "broker restarted"
    else
        echo "WARNING: mosquitto did not start — journalctl -u mosquitto -n 40"
    fi
else
    echo "mosquitto not installed — skipping (sudo apt install mosquitto mosquitto-clients)"
fi

# --- 4. build brain natively on the Pi ------------------------------------
step "build brain"
( cd "$REPO" && cargo build --release )

# --- 5. brain daemon -------------------------------------------------------
step "install and restart brain daemon"
# Removed first, so that what lands is a file of its own whatever was there before: a symlink
# at this path must be replaced, never written through to the binary it points at.
sudo rm -f /usr/local/bin/brain
sudo install -m 0755 "$REPO/target/release/brain" /usr/local/bin/brain
sudo install -m 0644 "$REPO/brain/brain.service" /etc/systemd/system/brain.service
sudo systemctl daemon-reload
sudo systemctl enable brain
sudo systemctl restart brain
echo "brain daemon restarted"

# --- 6. Stem ---------------------------------------------------------------
# rig_mqtt.py installs beside its importer rather than into a site-packages directory: the
# daemon is run by absolute path, which puts /usr/local/bin first on its import path, and
# one file copy beats owning a package layout for a single shared module.
step "install and restart Stem"
sudo install -m 0755 "$REPO/stem/stem.py" /usr/local/bin/stem.py
sudo install -m 0644 "$REPO/stem/rig_mqtt.py" /usr/local/bin/rig_mqtt.py
sudo install -m 0644 "$REPO/stem/stem.service" /etc/systemd/system/stem.service
python_daemon /opt/stem/venv "$REPO/stem/requirements.txt"
sudo systemctl daemon-reload
sudo systemctl enable stem
if sudo systemctl restart stem; then
    echo "stem daemon restarted"
else
    echo "WARNING: stem did not start — journalctl -u stem -n 40"
fi

# --- 7. flash attached microcontrollers ------------------------------------
# Last, so a board that is not in DEVICE_MAP fails the deploy only after the rig is back up.
# Absent boards are simply skipped.
#
# A board is left in its bootloader after flashing rather than reset at once. Its radio went
# dead the moment flashing began, without a goodbye, so the access point still lists it as
# associated and would ignore its attempts to rejoin until that entry timed out. With every
# board flashed and quiet, the access point is told to drop its stations — the fixtures and
# any laptop alike, which the hostapd restart above already did once — and only then are the
# boards reset, so each one is let straight back in on its first attempt.
step "flash attached microcontrollers"
shopt -s nullglob
flashed=()
for dev in /dev/serial/by-id/*; do
  name="$(basename "$dev")"
  if [[ -v DEVICE_MAP["$name"] ]]; then
    binary="${DEVICE_MAP[$name]}"
    echo ">> $name  -> $binary"
    espflash flash --after no-reset --port "$dev" "$REPO/binaries/$binary"
    flashed+=("$dev")
  else
    echo "ERROR: unknown device $name — add it to DEVICE_MAP in remote-deploy.sh" >&2
    exit 1
  fi
done

if [ "${#flashed[@]}" -eq 0 ]; then
  echo "no MCUs attached; nothing to flash."
else
  step "drop stations from the access point, then start the flashed boards"
  # The interface from the configuration rather than spelled out here, so the two cannot
  # disagree about which one hostapd is serving. The repository's copy, which is the one just
  # installed: the installed one is root's alone, for the passphrase in it.
  iface="$(sed -n 's/^interface=//p' "$REPO/etc/hostapd.conf")"
  for station in $(sudo hostapd_cli -i "$iface" list_sta); do
    echo ">> deauthenticate $station"
    sudo hostapd_cli -i "$iface" deauthenticate "$station"
  done
  for dev in "${flashed[@]}"; do
    espflash reset --port "$dev"
  done
fi

step "remote deploy complete"
