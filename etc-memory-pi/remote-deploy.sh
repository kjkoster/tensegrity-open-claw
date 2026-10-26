#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON memory-pi (pushed there by deploy-memory-pi.sh).
# Installs memory-pi's configuration.
#
set -euo pipefail

# Where the sources landed, asked of this script's own location rather than spelled out, so it
# cannot disagree with the path deploy-memory-pi.sh rsyncs to.
REPO="$(cd "$(dirname "$0")/.." && pwd)"

. "$REPO/etc/deploy-lib.sh"

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

# --- Health daemon -----------------------------------------------------------
# rig_mqtt.py and rig_board.py are Stem's, shipped here by deploy-memory-pi.sh. They install
# beside their importer, as on claw-pi: the daemon is run by absolute path, which puts
# /usr/local/bin first on its import path.
step "install and restart the health daemon"
sudo install -m 0755 "$REPO/etc-memory-pi/memory.py" /usr/local/bin/memory.py
sudo install -m 0644 "$REPO/stem/rig_mqtt.py" /usr/local/bin/rig_mqtt.py
sudo install -m 0644 "$REPO/stem/rig_board.py" /usr/local/bin/rig_board.py
sudo install -m 0644 "$REPO/etc-memory-pi/memory.service" /etc/systemd/system/memory.service
python_daemon /opt/memory/venv "$REPO/etc-memory-pi/requirements.txt"
sudo systemctl daemon-reload
sudo systemctl enable memory
sudo systemctl restart memory
echo "memory daemon restarted"

# --- GreptimeDB ----------------------------------------------------------------
# From a pinned release rather than apt, which carries no GreptimeDB. Downloaded only when the
# installed version differs from greptime.version — the tarball is close to 200 MB over claw-pi's
# 4G bundle — and installed only once its checksum matches. VERSION beside the binary records
# what was installed, so an unchanged pin downloads nothing.
#
# The data directory is made only on a mounted /tank: the mount step above has already failed
# the deploy otherwise, and a directory made on the empty mount point would sit on the SD card.
step "install and restart GreptimeDB"
. "$REPO/etc-memory-pi/greptime.version"
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
fi
if ! id greptime >/dev/null 2>&1; then
    sudo useradd --system --no-create-home --shell /usr/sbin/nologin greptime
fi
sudo install -d -o greptime -g greptime -m 0750 /tank/greptimedb
sudo install -d -m 0755 /etc/greptimedb
sudo install -m 0644 "$REPO/etc-memory-pi/greptimedb.toml" /etc/greptimedb/greptimedb.toml
sudo install -m 0644 "$REPO/etc-memory-pi/greptimedb.service" /etc/systemd/system/greptimedb.service
sudo systemctl daemon-reload
sudo systemctl enable greptimedb
sudo systemctl restart greptimedb
echo "GreptimeDB restarted"

# --- Recorder ----------------------------------------------------------------
# After Greptime, which it writes to. rig_mqtt.py is already in /usr/local/bin from the health
# daemon's step.
step "install and restart the recorder"
sudo install -m 0755 "$REPO/etc-memory-pi/recorder.py" /usr/local/bin/recorder.py
sudo install -m 0644 "$REPO/etc-memory-pi/recorder.service" /etc/systemd/system/recorder.service
python_daemon /opt/recorder/venv "$REPO/etc-memory-pi/recorder-requirements.txt"
sudo systemctl daemon-reload
sudo systemctl enable recorder
sudo systemctl restart recorder
echo "recorder restarted"

# --- Grafana -------------------------------------------------------------------
# From Grafana's apt repository. Its signing key is fetched once, and the package installed only
# when it is missing — the package is large, and claw-pi's 4G bundle carries it — after which it
# upgrades with the rest of the system like any other package.
#
# The settings are a drop-in for its unit and the data source is provisioning, so both are
# the repo's on every deploy and the package's own files are never edited.
step "install and restart Grafana"
sudo install -d -m 0755 /etc/apt/keyrings
if [ ! -f /etc/apt/keyrings/grafana.asc ]; then
    curl -fsSL https://apt.grafana.com/gpg.key |
        sudo install -m 0644 /dev/stdin /etc/apt/keyrings/grafana.asc
fi
sudo install -m 0644 "$REPO/etc-memory-pi/grafana/grafana.sources" \
    /etc/apt/sources.list.d/grafana.sources
if dpkg-query -W -f='${Status}' grafana 2>/dev/null | grep -q "install ok installed"; then
    echo "grafana already installed"
else
    sudo apt-get update
    sudo apt-get install -y grafana
fi
sudo install -d -m 0755 /etc/systemd/system/grafana-server.service.d
sudo install -m 0644 "$REPO/etc-memory-pi/grafana/grafana-server.conf" \
    /etc/systemd/system/grafana-server.service.d/claw.conf
sudo install -o root -g grafana -m 0640 "$REPO/etc-memory-pi/grafana/datasources.yaml" \
    /etc/grafana/provisioning/datasources/greptime.yaml
sudo systemctl daemon-reload
sudo systemctl enable grafana-server
sudo systemctl restart grafana-server
echo "Grafana restarted"

step "remote deploy complete"
