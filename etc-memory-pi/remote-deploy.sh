#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON memory-pi (pushed there by deploy-memory-pi.sh).
# Installs memory-pi's configuration.
#
set -euo pipefail

# Where the sources landed, asked of this script's own location rather than spelled out, so it
# cannot disagree with the path deploy-memory-pi.sh rsyncs to.
REPO="$(cd "$(dirname "$0")/.." && pwd)"

step() { printf '\n\033[1m--- %s ---\033[0m\n' "$1"; }

# Installs a NetworkManager profile template with the rig's WiFi filled in from claw-wifi.env,
# the one place it is defined. Bash substitution rather than sed, so no character in the
# passphrase can break it; quoted, so not even `&`. 0600, root's, as NetworkManager requires
# of a profile holding a secret.
wifi_profile() {
    local template="$1" name="$2" profile
    . "$REPO/etc/claw-wifi.env"
    profile="$(<"$template")"
    profile="${profile//@SSID@/"$CLAW_SSID"}"
    profile="${profile//@PSK@/"$CLAW_PSK"}"
    profile="${profile//@CHANNEL@/"$CLAW_CHANNEL"}"
    printf '%s\n' "$profile" |
        sudo install -m 0600 /dev/stdin "/etc/NetworkManager/system-connections/$name.nmconnection"
}

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

step "remote deploy complete"
