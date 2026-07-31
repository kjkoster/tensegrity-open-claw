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

case "$RIG" in
    ""|claw-brain|mage-brain) ;;
    *) echo "unknown rig '$RIG' — expected claw-brain or mage-brain" >&2; exit 1 ;;
esac

# --- 1. build both rigs natively on the Pi --------------------------------
step "build claw-brain and mage-brain"
( cd "$REPO" && cargo build --release )

# --- 2. install both binaries ---------------------------------------------
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

# --- 3. brain daemon -------------------------------------------------------
step "install and restart brain daemon"
sudo install -m 0644 "$REPO/brain.service" /etc/systemd/system/brain.service
sudo systemctl daemon-reload
sudo systemctl enable brain
sudo systemctl restart brain
echo "brain daemon restarted: $(readlink /usr/local/bin/brain)"

step "remote deploy complete"
