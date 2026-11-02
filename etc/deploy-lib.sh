# deploy-lib.sh — what both Pis' remote deploys share, sourced by each of them: claw-pi's
# etc/remote-deploy.sh and memory-pi's etc-memory-pi/remote-deploy.sh. One copy, so a fix here
# reaches both Pis.
#
# Expects REPO to be set by the script sourcing it, to where the sources landed on the Pi.

step() { printf '\n\033[1m--- %s ---\033[0m\n' "$1"; }

# Restarts a unit and stops the deploy if it does not come up: a service that failed to start
# is a deploy that failed, and the journal says why.
restart() {
    local unit="$1"
    if ! sudo systemctl restart "$unit"; then
        echo "ERROR: $unit did not start — journalctl -u $unit -n 40" >&2
        exit 1
    fi
    echo "$unit restarted"
}

# Installs `source` at `target` with `mode` only when their contents differ, and then sets
# CHANGED, so a deploy restarts only what it actually changed: a restart costs a service its
# state, and the access point's costs every station its connection. The caller clears CHANGED
# before a group of files and restarts their service if it is set.
#
# Compared as root, because a target holding a secret is readable by root alone. A missing
# target differs. The target is removed before the install, so what lands is a file of its own
# whatever was there before: a symlink must be replaced, never written through.
#
# A function that only ever sets a flag rather than returning one, so it runs as a plain
# command under `set -e` and a failed install still stops the deploy.
install_changed() {
    local mode="$1" source="$2" target="$3"
    if sudo cmp -s "$source" "$target"; then
        return 0
    fi
    sudo rm -f "$target"
    sudo install -m "$mode" "$source" "$target"
    echo "installed $target"
    CHANGED=1
}

# Each Python daemon gets its own venv and its own pip install, so its requirements.txt is the
# only statement of what it needs and the deploy is what acts on it. Separate venvs rather than
# one shared: a health daemon has to keep reporting when another daemon is exactly what has
# gone wrong, which it cannot do while sharing that daemon's dependency tree.
#
# Created with --system-site-packages, so anything already present from apt stays visible as a
# floor under whatever pip resolves.
#
# The requirements pip last installed are kept in the venv, and pip runs only when they
# differ, setting CHANGED as `install_changed` does: a daemon whose packages changed needs its
# restart like one whose code did. Kept only after pip succeeds, so a failed install is tried
# again on the next deploy.
python_daemon() {
    local venv="$1" requirements="$2"
    if [ ! -x "$venv/bin/python3" ]; then
        sudo mkdir -p "$(dirname "$venv")"
        sudo python3 -m venv --system-site-packages "$venv"
        echo "created $venv"
    fi
    if sudo cmp -s "$requirements" "$venv/requirements.txt"; then
        return 0
    fi
    # Not quiet, and not tolerated: a deploy that cannot install what a daemon imports has
    # failed, and finding that out here beats finding it in a crash loop.
    #
    # No --upgrade either. With it, every deploy re-queries the index for newer versions of
    # everything; without it, pip sees the requirements already satisfied and does not touch
    # the network at all. Both Pis reach PyPI over claw-pi's 4G modem, so that is the
    # difference between a deploy that needs the uplink and one that does not.
    sudo "$venv/bin/pip" install -r "$requirements"
    sudo install -m 0644 "$requirements" "$venv/requirements.txt"
    CHANGED=1
}

# Installs a NetworkManager profile template with the rig's WiFi filled in from claw-wifi.env,
# the one place it is defined. Bash substitution rather than sed, so no character in the
# passphrase can break it; quoted, so not even `&`. 0600, root's, as NetworkManager requires
# of a profile holding a secret. Installed only when it differs, setting CHANGED as
# `install_changed` does.
wifi_profile() {
    local template="$1" name="$2" profile target
    . "$REPO/etc/claw-wifi.env"
    profile="$(<"$template")"
    profile="${profile//@SSID@/"$CLAW_SSID"}"
    profile="${profile//@PSK@/"$CLAW_PSK"}"
    profile="${profile//@CHANNEL@/"$CLAW_CHANNEL"}"
    # Compared and installed straight from the rendered text, so the passphrase never lands in
    # a file of its own on the way to root's copy.
    target="/etc/NetworkManager/system-connections/$name.nmconnection"
    if printf '%s\n' "$profile" | sudo cmp -s - "$target"; then
        return 0
    fi
    sudo rm -f "$target"
    printf '%s\n' "$profile" | sudo install -m 0600 /dev/stdin "$target"
    echo "installed $target"
    CHANGED=1
}
