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

step "remote deploy complete"
