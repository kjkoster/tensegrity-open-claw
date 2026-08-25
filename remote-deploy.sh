#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON claw-pi (pushed there by deploy.sh).
# Builds both rig binaries natively, installs them, optionally selects a rig, and restarts.
#
#   bash remote-deploy.sh                 restart whichever rig the symlink already selects
#   bash remote-deploy.sh mage-brain      point the symlink at the mage rig, then restart
#
set -euo pipefail

. $HOME/.cargo/env

# Where the sources landed, asked of this script's own location rather than spelled out. A
# second copy of the path here could disagree with the one deploy.sh rsyncs to, and the
# result would be a clean deploy that rebuilt and reinstalled the previous sources.
REPO="$(cd "$(dirname "$0")" && pwd)"
RIG="${1:-}"

step() { printf '\n\033[1m--- %s ---\033[0m\n' "$1"; }

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

case "$RIG" in
    ""|claw-brain|mage-brain) ;;
    *) echo "unknown rig '$RIG' — expected claw-brain or mage-brain" >&2; exit 1 ;;
esac

# --- 1. Stem's broker ------------------------------------------------------
# First, before anything that reports to it. Every daemon on this rig holds its own client
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

# --- 2. build both rigs natively on the Pi --------------------------------
step "build claw-brain and mage-brain"
( cd "$REPO" && cargo build --release )

# --- 3. install both binaries ---------------------------------------------
# Both rigs are always built and always installed. Switching rigs means travel and setup, so
# which one is live is a decision made on site — and made explicitly, by naming it.
step "install rig binaries"
sudo install -m 0755 "$REPO/target/release/claw-brain" /usr/local/bin/claw-brain
sudo install -m 0755 "$REPO/target/release/mage-brain" /usr/local/bin/mage-brain

# The symlink is the state — the service execs through it and `readlink` reports which rig is
# live, so there is no second file to go stale. A deploy moves it only when told which rig to
# move it to; otherwise it is left exactly as found, because a deploy that could change rigs
# on its own would be a rig change nobody asked for.
#
# -L, not -e, on the existence test: -e follows the link, so a selector pointing at something
# that is not there would read as absent and get silently repointed at the claw — the same rig
# change, arrived at by accident. A dangling selector is a state to report, not to guess at.
if [ -n "$RIG" ]; then
    sudo ln -sfn "/usr/local/bin/$RIG" /usr/local/bin/brain
    echo "selected $RIG"
elif [ ! -L /usr/local/bin/brain ] && [ ! -e /usr/local/bin/brain ]; then
    sudo ln -sfn /usr/local/bin/claw-brain /usr/local/bin/brain
    echo "created /usr/local/bin/brain → claw-brain"
elif [ ! -e /usr/local/bin/brain ]; then
    echo "WARNING: /usr/local/bin/brain points at $(readlink /usr/local/bin/brain), which does not exist."
    echo "         Re-run with claw-brain or mage-brain; this script will not choose for you."
fi

# --- 4. brain daemon -------------------------------------------------------
step "install and restart brain daemon"
sudo install -m 0644 "$REPO/brain.service" /etc/systemd/system/brain.service
sudo systemctl daemon-reload
sudo systemctl enable brain
sudo systemctl restart brain
echo "brain daemon restarted: $(readlink /usr/local/bin/brain)"

# --- 5. Stem ---------------------------------------------------------------
# rig_mqtt.py installs beside its importers rather than into a site-packages directory: both
# daemons are run by absolute path, which puts /usr/local/bin first on their import path, and
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

# --- 6. eyeball's pose model -----------------------------------------------
# MoveNet SinglePose Lightning, int8, from the TF Hub lite-model store — the same model the
# TensorFlow Hub MoveNet tutorial uses. Fetched once and then left alone: it is a few megabytes
# over a 4G modem, and it never changes.
#
# Not committed to the repository. It is a binary blob with its own licence that is reproducible
# from a URL, which is the definition of something a checkout should not carry.
MODEL_DIR=/usr/local/share/eyeball
MODEL_TFLITE="$MODEL_DIR/movenet.tflite"
# The URL the TF Hub MoveNet tutorial itself uses. tfhub.dev now redirects into Kaggle Models,
# which is why this follows redirects and then checks what actually arrived — a redirect that
# lands on a sign-in page returns HTTP 200 and an HTML document.
#
# Overridable, because this is the one line here most likely to rot: Google has moved this file
# once already and the daemon does not care where it came from.
MODEL_URL="${EYEBALL_MODEL_URL:-https://tfhub.dev/google/lite-model/movenet/singlepose/lightning/tflite/int8/4?lite-format=tflite}"

step "eyeball pose model"
if [ -f "$MODEL_TFLITE" ]; then
    echo "already present: $MODEL_TFLITE ($(stat -c %s "$MODEL_TFLITE") bytes)"
else
    sudo mkdir -p "$MODEL_DIR"
    echo "fetching MoveNet SinglePose Lightning int8"
    sudo curl -fSL --retry 3 --connect-timeout 20 -o "$MODEL_TFLITE.part" "$MODEL_URL"

    # A TFLite flatbuffer carries the identifier "TFL3" at offset 4. Checking it is what turns
    # "that URL now redirects to a sign-in page" into one legible line here, rather than into a
    # daemon that loads an HTML document as a model and fails somewhere less obvious.
    if [ "$(sudo head -c 8 "$MODEL_TFLITE.part" | tail -c 4)" = "TFL3" ]; then
        sudo mv "$MODEL_TFLITE.part" "$MODEL_TFLITE"
        echo "installed: $MODEL_TFLITE ($(stat -c %s "$MODEL_TFLITE") bytes)"
    else
        sudo rm -f "$MODEL_TFLITE.part"
        echo "ERROR: what came back from $MODEL_URL is not a TFLite model." >&2
        echo "       Google moved TF Hub to Kaggle, so this URL may have gone with it." >&2
        echo "       Either point EYEBALL_MODEL_URL at a working one and re-run:" >&2
        echo "         EYEBALL_MODEL_URL=... ./deploy.sh" >&2
        echo "       or fetch MoveNet SinglePose Lightning (int8 tflite) by hand and put it at" >&2
        echo "       $MODEL_TFLITE — the deploy then leaves it alone." >&2
        exit 1
    fi
fi

# --- 7. eyeball daemon -----------------------------------------------------
# Installed and restarted alongside the brain, but never coupled to it: no ordering, no
# dependency, in either direction. A dead eyeball is a staleness timeout the show already
# handles, and a unit dependency would turn that degraded show into a stopped one.
#
# Its restart is therefore also allowed to fail without failing the deploy — a Pi with no
# camera on the link, or without OpenCV yet, is a Pi whose rig still has to come up.
step "install and restart eyeball daemon"
sudo install -m 0755 "$REPO/eyeball/eyeball.py" /usr/local/bin/eyeball.py
sudo install -m 0644 "$REPO/eyeball/eyeball.service" /etc/systemd/system/eyeball.service
# Seeded once, never updated. The camera's whole configuration lives in this file, including the
# password and the crop that gets retuned on site against the live preview — so a deploy that
# copied over it would undo an evening's tuning and put a placeholder password back. A fresh card
# gets a working file; every card after that keeps its own, and changes worth having are carried
# back into the repository by hand.
if [ ! -e /etc/default/eyeball ]; then
    sudo install -m 0600 "$REPO/eyeball/eyeball.default" /etc/default/eyeball
    echo "seeded /etc/default/eyeball — set EYEBALL_CAMERA_PASSWORD before the camera will answer"
fi
python_daemon /opt/eyeball/venv "$REPO/eyeball/requirements.txt"
sudo systemctl daemon-reload
sudo systemctl enable eyeball
if sudo systemctl restart eyeball; then
    echo "eyeball daemon restarted"
else
    echo "WARNING: eyeball did not start — journalctl -u eyeball -n 40"
fi

step "remote deploy complete"
