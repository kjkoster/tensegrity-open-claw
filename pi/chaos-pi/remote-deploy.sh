#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON claw-pi (pushed there by deploy-chaos-pi.sh, which built the brain).
# Installs the brain and what the Pi runs beside it. Every service is restarted only when one
# of its files changed (`install_changed`), so a deploy that touches only the brain restarts
# only the brain.
#
set -euo pipefail

# Where deploy-chaos-pi.sh put the files, asked of this script's own location rather than
# spelled out. A second copy of the path here could disagree with the one deploy-chaos-pi.sh
# rsyncs to, and the result would be a clean deploy that reinstalled the previous files.
REPO="$(cd "$(dirname "$0")/../.." && pwd)"

. "$REPO/pi/deploy-lib.sh"

# --- 4G uplink watchdog ----------------------------------------------------
# Ahead of everything else: this is what gets the Pi back onto the network after the modem
# wedges, and the rig is a long way from the person who would otherwise have to walk to it. A
# deploy that dies further down should still have installed the way back in.
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
CHANGED=
install_changed 0755 "$REPO/pi/chaos-pi/4g-watchdog.sh" /usr/local/sbin/4g-watchdog.sh
install_changed 0644 "$REPO/pi/chaos-pi/4g-watchdog.service" /etc/systemd/system/4g-watchdog.service
install_changed 0644 "$REPO/pi/chaos-pi/4g-watchdog.timer" /etc/systemd/system/4g-watchdog.timer
sudo systemctl enable 4g-watchdog.timer
# Restarted and not merely enabled, so an edited schedule takes effect on this deploy rather
# than at whatever reboot happens next. The timer only: the service is the check itself, and
# firing it from here would have it judge an uplink in the middle of being deployed over.
if [ -n "$CHANGED" ]; then
    sudo systemctl daemon-reload
    restart 4g-watchdog.timer
else
    echo "4G watchdog unchanged"
fi

# --- WiFi access point and wired segment -----------------------------------
# NetworkManager's, like every other connection on the rig's Pis. The versioned profiles
# replace whatever is on the Pi, so a setting changed there by hand does not survive a deploy.
# Both were rendered from clockwork.dot by deploy-chaos-pi.sh, into rendered/. 0600, root's, as
# NetworkManager requires of a profile holding a secret.
#
# Each is brought up again only when its profile changed, because that drops every station on
# it for a few seconds. A profile edited by hand on the Pi differs from the repo's, so it is
# replaced and brought up like any other change. A deploy over 4G arrives through the tunnel,
# not through either of them, so it is not cut off.
step "install WiFi access point and wired segment"
CHANGED=
install_changed 0600 "$REPO/rendered/claw-ap.nmconnection" \
    /etc/NetworkManager/system-connections/claw-ap.nmconnection
ap_changed="$CHANGED"
CHANGED=
install_changed 0600 "$REPO/rendered/claw-eth0.nmconnection" \
    /etc/NetworkManager/system-connections/claw-eth0.nmconnection
eth0_changed="$CHANGED"
if [ -n "$ap_changed$eth0_changed" ]; then
    sudo nmcli connection reload
fi
if [ -n "$ap_changed" ]; then
    sudo nmcli connection up claw-ap
else
    echo "access point unchanged"
fi
if [ -n "$eth0_changed" ]; then
    sudo nmcli connection up claw-eth0
else
    echo "wired segment unchanged"
fi

# Stem's broker is not installed here: it runs on memory-pi, deployed by deploy-memory-pi.sh.

# --- Brain daemon ----------------------------------------------------------
step "install brain daemon"
# Cargo relinks only when something it builds from changed, so an unchanged binary is a deploy
# that changed nothing the brain runs, the workspace and the show table included.
CHANGED=
install_changed 0755 "$REPO/brain/brain" /usr/local/bin/brain
install_changed 0644 "$REPO/brain/brain.service" /etc/systemd/system/brain.service
sudo systemctl enable brain
if [ -n "$CHANGED" ]; then
    sudo systemctl daemon-reload
    restart brain
else
    echo "brain unchanged"
fi

# --- Stem ------------------------------------------------------------------
# rig_mqtt.py and rig_board.py install beside their importer rather than into a site-packages
# directory: the daemon is run by absolute path, which puts /usr/local/bin first on its import
# path, and a file copy beats owning a package layout for two shared modules. The unit is
# deploy-chaos-pi.sh's rendering, with the broker's address filled in from clockwork.dot.
step "install Stem"
CHANGED=
install_changed 0755 "$REPO/stem/stem.py" /usr/local/bin/stem.py
install_changed 0644 "$REPO/stem/rig_mqtt.py" /usr/local/bin/rig_mqtt.py
install_changed 0644 "$REPO/stem/rig_board.py" /usr/local/bin/rig_board.py
install_changed 0644 "$REPO/rendered/stem.service" /etc/systemd/system/stem.service
python_daemon /opt/stem/venv "$REPO/stem/requirements.txt"
sudo systemctl enable stem
if [ -n "$CHANGED" ]; then
    sudo systemctl daemon-reload
    restart stem
else
    echo "Stem unchanged"
fi

step "remote deploy complete"
