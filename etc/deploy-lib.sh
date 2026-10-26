# deploy-lib.sh — what both Pis' remote deploys share, sourced by each of them: claw-pi's
# etc/remote-deploy.sh and memory-pi's etc-memory-pi/remote-deploy.sh. One copy, so a fix here
# reaches both Pis.
#
# Expects REPO to be set by the script sourcing it, to where the sources landed on the Pi.

step() { printf '\n\033[1m--- %s ---\033[0m\n' "$1"; }

# Each Python daemon gets its own venv and its own pip install, so its requirements.txt is the
# only statement of what it needs and the deploy is what acts on it. Separate venvs rather than
# one shared: a health daemon has to keep reporting when another daemon is exactly what has
# gone wrong, which it cannot do while sharing that daemon's dependency tree.
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
    # the network at all. Both Pis reach PyPI over claw-pi's 4G modem, so that is the
    # difference between a deploy that needs the uplink and one that does not.
    sudo "$venv/bin/pip" install -r "$requirements"
}

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
