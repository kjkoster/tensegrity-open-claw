#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON memory-pi (pushed there by deploy-memory-pi.sh).
# Installs memory-pi's configuration. As on chaos-pi, each service is restarted only when one
# of its files changed (`install_changed`) or it was installed just now, so a deploy that
# touches only Memory restarts only Memory.
#
set -euo pipefail

# Where the sources landed, asked of this script's own location rather than spelled out, so it
# cannot disagree with the path deploy-memory-pi.sh rsyncs to.
REPO="$(cd "$(dirname "$0")/../.." && pwd)"

. "$REPO/pi/deploy-lib.sh"

# --- WiFi ------------------------------------------------------------------
# chaos-pi's access point is memory-pi's only network, at a fixed address. Loaded but not
# brought up: this deploy arrives over that very link, and switching it now would cut the
# deploy off halfway. The profile takes over at the next boot. Rendered from clockwork.dot by
# deploy-memory-pi.sh; 0600, root's, as NetworkManager requires of a profile holding a secret.
step "install WiFi connection"
CHANGED=
install_changed 0600 "$REPO/rendered/claw.nmconnection" \
    /etc/NetworkManager/system-connections/claw.nmconnection
if [ -n "$CHANGED" ]; then
    sudo nmcli connection reload
    echo "reboot memory-pi for the changed WiFi connection to take effect"
else
    echo "WiFi connection unchanged"
fi

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
tank_line="$(grep -v -e '^#' -e '^$' "$REPO/pi/memory-pi/fstab-tank")"
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

# --- Stem's broker -------------------------------------------------------------
# Before anything that reports to it: every daemon on the rig holds its own client connection
# and announces itself on `health/`. Debian's Mosquitto, installed when missing, with the rig's
# settings as an overlay in its include directory, so an upgrade cannot revert them. The
# clients come with it, for `mosquitto_sub` on the Pi itself.
#
# A restart drops every client and, with no persistence, every retained value, so it happens
# only when the settings changed or the broker was installed just now.
step "install Stem's broker"
CHANGED=
if dpkg-query -W -f='${Status}' mosquitto 2>/dev/null | grep -q "install ok installed"; then
    echo "mosquitto already installed"
else
    sudo apt-get update
    sudo apt-get install -y mosquitto mosquitto-clients
    CHANGED=1
fi
install_changed 0644 "$REPO/stem/stem.conf" /etc/mosquitto/conf.d/stem.conf
sudo systemctl enable mosquitto
if [ -n "$CHANGED" ]; then
    restart mosquitto
else
    echo "broker unchanged"
fi

# --- GreptimeDB ----------------------------------------------------------------
# From a pinned release rather than apt, which carries no GreptimeDB. Downloaded only when the
# installed version differs from greptime.version — the tarball is close to 200 MB over chaos-pi's
# 4G bundle — and installed only once its checksum matches. VERSION beside the binary records
# what was installed, so an unchanged pin downloads nothing.
#
# The data directory is made only on a mounted /tank: the mount step above has already failed
# the deploy otherwise, and a directory made on the empty mount point would sit on the SD card.
step "install GreptimeDB"
CHANGED=
. "$REPO/pi/memory-pi/greptime.version"
if [ "$(cat /opt/greptimedb/VERSION 2>/dev/null)" = "$GREPTIME_VERSION" ]; then
    echo "GreptimeDB $GREPTIME_VERSION already installed"
else
    # /var/tmp, on the SD card, not /tmp: Debian keeps /tmp in RAM, capped at half of it, and the
    # unpacked binary alone is larger than that on a 1 GB Pi.
    download="$(mktemp -d -p /var/tmp)"
    tarball="greptime-linux-arm64-$GREPTIME_VERSION.tar.gz"
    curl -fL --retry 3 -o "$download/$tarball" \
        "https://github.com/GreptimeTeam/greptimedb/releases/download/$GREPTIME_VERSION/$tarball"
    echo "$GREPTIME_SHA256  $download/$tarball" | sha256sum -c -
    tar -xzf "$download/$tarball" -C "$download"
    sudo mkdir -p /opt/greptimedb
    sudo install -m 0755 "$download/greptime-linux-arm64-$GREPTIME_VERSION/greptime" \
        /opt/greptimedb/greptime
    echo "$GREPTIME_VERSION" | sudo tee /opt/greptimedb/VERSION >/dev/null
    rm -rf "$download"
    echo "GreptimeDB $GREPTIME_VERSION installed"
    CHANGED=1
fi
if ! id greptime >/dev/null 2>&1; then
    sudo useradd --system --no-create-home --shell /usr/sbin/nologin greptime
fi
sudo install -d -o greptime -g greptime -m 0750 /tank/greptimedb
sudo install -d -m 0755 /etc/greptimedb
install_changed 0644 "$REPO/pi/memory-pi/greptimedb.toml" /etc/greptimedb/greptimedb.toml
install_changed 0644 "$REPO/pi/memory-pi/greptimedb.service" /etc/systemd/system/greptimedb.service
sudo systemctl enable greptimedb
if [ -n "$CHANGED" ]; then
    sudo systemctl daemon-reload
    restart greptimedb
else
    echo "GreptimeDB unchanged"
fi

# --- Memory --------------------------------------------------------------------
# After the broker and GreptimeDB, which it relays and writes to. Built on the Mac and shipped
# by deploy-memory-pi.sh. Cargo relinks only when something it builds from changed, so an
# unchanged binary is a deploy that changed nothing Memory runs.
step "install Memory"
CHANGED=
install_changed 0755 "$REPO/memory/memory" /usr/local/bin/memory
install_changed 0644 "$REPO/pi/memory-pi/memory.service" /etc/systemd/system/memory.service
sudo systemctl enable memory
if [ -n "$CHANGED" ]; then
    sudo systemctl daemon-reload
    restart memory
else
    echo "Memory unchanged"
fi

# --- Grafana -------------------------------------------------------------------
# From Grafana's apt repository. Its signing key is fetched once, and the package installed only
# when it is missing — the package is large, and chaos-pi's 4G bundle carries it — after which it
# upgrades with the rest of the system like any other package.
#
# The settings are a drop-in for its unit and the data source is provisioning, so both are
# the repo's on every deploy and the package's own files are never edited.
step "install Grafana"
CHANGED=
sudo install -d -m 0755 /etc/apt/keyrings
if [ ! -f /etc/apt/keyrings/grafana.asc ]; then
    curl -fsSL https://apt.grafana.com/gpg.key |
        sudo install -m 0644 /dev/stdin /etc/apt/keyrings/grafana.asc
fi
sudo install -m 0644 "$REPO/pi/memory-pi/grafana/grafana.sources" \
    /etc/apt/sources.list.d/grafana.sources
if dpkg-query -W -f='${Status}' grafana 2>/dev/null | grep -q "install ok installed"; then
    echo "grafana already installed"
else
    sudo apt-get update
    sudo apt-get install -y grafana
    CHANGED=1
fi
sudo install -d -m 0755 /etc/systemd/system/grafana-server.service.d
install_changed 0644 "$REPO/pi/memory-pi/grafana/grafana-server.conf" \
    /etc/systemd/system/grafana-server.service.d/claw.conf
# Grafana's own group must read the data source, and install_changed installs as root alone.
grafana_datasource=/etc/grafana/provisioning/datasources/greptime.yaml
install_changed 0640 "$REPO/pi/memory-pi/grafana/datasources.yaml" "$grafana_datasource"
sudo chgrp grafana "$grafana_datasource"
sudo systemctl enable grafana-server
if [ -n "$CHANGED" ]; then
    sudo systemctl daemon-reload
    restart grafana-server
else
    echo "Grafana unchanged"
fi

step "remote deploy complete"
