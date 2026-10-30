#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON claw-pi (pushed there by deploy.sh).
# Installs what the Pi runs beside the brain, then builds the brain natively, installs it and
# restarts it.
#
set -euo pipefail

. $HOME/.cargo/env

# Where the sources landed, asked of this script's own location rather than spelled out. A
# second copy of the path here could disagree with the one deploy.sh rsyncs to, and the
# result would be a clean deploy that rebuilt and reinstalled the previous sources.
REPO="$(cd "$(dirname "$0")/.." && pwd)"

. "$REPO/etc/deploy-lib.sh"

# --- 4G uplink watchdog ----------------------------------------------------
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

# --- WiFi access point and wired segment -----------------------------------
# NetworkManager's, like every other connection on the rig's Pis. The versioned profiles
# replace whatever is on the Pi, so a setting changed there by hand does not survive a deploy.
# The access point's is a template filled from claw-wifi.env; the wired segment's shares
# nothing with another file and installs as it is.
#
# Both brought up on every deploy, changed or not, so the profiles in the repo are the ones
# running. That drops every station for a few seconds. A deploy over 4G arrives through the
# tunnel, not through either of them, so it is not cut off.
step "install and restart WiFi access point and wired segment"
wifi_profile "$REPO/etc/claw-ap.nmconnection" claw-ap
sudo install -m 0600 "$REPO/etc/claw-eth0.nmconnection" \
    /etc/NetworkManager/system-connections/claw-eth0.nmconnection
# The wired segment's reservations, for NetworkManager's own DHCP server, which reads them when
# the connection comes up below.
sudo install -d -m 0755 /etc/NetworkManager/dnsmasq-shared.d
sudo install -m 0644 "$REPO/etc/claw-eth0.dnsmasq.conf" \
    /etc/NetworkManager/dnsmasq-shared.d/claw-eth0.conf
sudo nmcli connection reload
sudo nmcli connection up claw-ap
sudo nmcli connection up claw-eth0

# Stem's broker is not installed here: it runs on memory-pi, deployed by deploy-memory-pi.sh.

# --- Build brain natively on the Pi ---------------------------------------
step "build brain"
( cd "$REPO" && cargo build --release )

# --- Brain daemon ----------------------------------------------------------
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

# --- Stem ------------------------------------------------------------------
# rig_mqtt.py and rig_board.py install beside their importer rather than into a site-packages
# directory: the daemon is run by absolute path, which puts /usr/local/bin first on its import
# path, and a file copy beats owning a package layout for two shared modules.
step "install and restart Stem"
sudo install -m 0755 "$REPO/stem/stem.py" /usr/local/bin/stem.py
sudo install -m 0644 "$REPO/stem/rig_mqtt.py" /usr/local/bin/rig_mqtt.py
sudo install -m 0644 "$REPO/stem/rig_board.py" /usr/local/bin/rig_board.py
sudo install -m 0644 "$REPO/stem/stem.service" /etc/systemd/system/stem.service
python_daemon /opt/stem/venv "$REPO/stem/requirements.txt"
sudo systemctl daemon-reload
sudo systemctl enable stem
if sudo systemctl restart stem; then
    echo "stem daemon restarted"
else
    echo "WARNING: stem did not start — journalctl -u stem -n 40"
fi

step "remote deploy complete"
