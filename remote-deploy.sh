#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON claw-pi (pushed there by deploy.sh).
# Builds brain natively, then installs and restarts the brain daemon.
#
set -euo pipefail

. $HOME/.cargo/env

step() { printf '\n\033[1m--- %s ---\033[0m\n' "$1"; }

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

step "remote deploy complete"
