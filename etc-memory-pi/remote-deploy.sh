#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON memory-pi (pushed there by deploy-memory-pi.sh).
# Installs memory-pi's configuration.
#
set -euo pipefail

# Where the sources landed, asked of this script's own location rather than spelled out, so it
# cannot disagree with the path deploy-memory-pi.sh rsyncs to.
REPO="$(cd "$(dirname "$0")/.." && pwd)"

. "$REPO/etc/deploy-lib.sh"

# --- WiFi ------------------------------------------------------------------
# claw-pi's access point is memory-pi's only network, at a fixed address. Loaded but not
# brought up: this deploy arrives over that very link, and switching it now would cut the
# deploy off halfway. The profile takes over at the next boot.
step "install WiFi connection"
wifi_profile "$REPO/etc-memory-pi/claw.nmconnection" claw
sudo nmcli connection reload
echo "reboot memory-pi for a changed WiFi connection to take effect"

# --- The SSD -----------------------------------------------------------------
# The repo's line replaces whatever line mounts /tank, and the rest of /etc/fstab is left as the
# image wrote it. Written only when it differs, so a deploy with nothing new leaves the file
# alone.
#
# Never unmounted: Greptime writes there, and a deploy can run at any time. A changed line is
# applied with a remount, which takes new options on a mounted file system without letting go
# of it; a changed device or file system type cannot be, and waits for a reboot. A /tank that is
# not mounted yet — the first deploy, or an SSD plugged in after boot — is mounted here.
step "mount the SSD on /tank"
tank_line="$(grep -v -e '^#' -e '^$' "$REPO/etc-memory-pi/fstab-tank")"
if grep -qxF "$tank_line" /etc/fstab; then
    echo "/etc/fstab already mounts /tank"
else
    # Built in full before anything is written, so /etc/fstab is never read and replaced at
    # the same time.
    fstab="$(awk '$2 != "/tank"' /etc/fstab)"
    printf '%s\n%s\n' "$fstab" "$tank_line" | sudo install -m 0644 /dev/stdin /etc/fstab
    sudo systemctl daemon-reload
    echo "/etc/fstab now mounts /tank"
    if mountpoint -q /tank; then
        sudo mount -o remount /tank
        echo "/tank remounted with the new options"
    fi
fi
sudo mkdir -p /tank
if mountpoint -q /tank; then
    echo "/tank mounted"
else
    sudo mount /tank
    echo "/tank mounted now"
fi

# --- Health daemon -----------------------------------------------------------
# rig_mqtt.py and rig_board.py are Stem's, shipped here by deploy-memory-pi.sh. They install
# beside their importer, as on claw-pi: the daemon is run by absolute path, which puts
# /usr/local/bin first on its import path.
step "install and restart the health daemon"
sudo install -m 0755 "$REPO/etc-memory-pi/memory.py" /usr/local/bin/memory.py
sudo install -m 0644 "$REPO/stem/rig_mqtt.py" /usr/local/bin/rig_mqtt.py
sudo install -m 0644 "$REPO/stem/rig_board.py" /usr/local/bin/rig_board.py
sudo install -m 0644 "$REPO/etc-memory-pi/memory.service" /etc/systemd/system/memory.service
python_daemon /opt/memory/venv "$REPO/etc-memory-pi/requirements.txt"
sudo systemctl daemon-reload
sudo systemctl enable memory
sudo systemctl restart memory
echo "memory daemon restarted"

step "remote deploy complete"
