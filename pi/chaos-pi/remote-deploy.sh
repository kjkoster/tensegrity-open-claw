#!/usr/bin/env bash
#
# remote-deploy.sh — runs ON chaos-pi (pushed there by deploy-chaos-pi.sh, which built the brain).
# Installs the brain and what the Pi runs beside it, flashes the boards on its USB, then
# installs the telemetry: the SSD, Stem's broker, GreptimeDB and Memory. Every
# service is restarted only when one of its files changed (`install_changed`), so a deploy that
# touches only the brain restarts only the brain.
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

# --- Boards on USB ---------------------------------------------------------
# Last, so a board that stops the deploy does so with the rig already back up. deploy-chaos-pi.sh
# built the firmware into binaries/<device>.bin and the board table, `<mac> <device> <chip>` per
# line, into rendered/boards, from the MACs and chips in clockwork.dot. Every board on USB is
# known by the MAC its chip reports (boards.sh), so a board the dot does not name stops the
# deploy: firmware is never written to a board on a guess. esptool is told the chip rather than
# left to detect it, and refuses a board whose chip is another.
#
# A board is flashed only when its image differs from the one last written to that MAC, so a
# deploy that changed no firmware leaves every board's flash alone. Each board still restarts,
# because asking a chip its MAC does that.
#
# esptool from Debian's package, which also brings the pyserial board-log.py reads with.
step "flash boards on USB"
if ! command -v esptool >/dev/null; then
    sudo apt-get install -y esptool
fi
FLASHED=/var/lib/clockwork/flashed
sudo mkdir -p "$FLASHED"
boards="$(bash "$REPO/pi/chaos-pi/boards.sh")"
if [ -z "$boards" ]; then
    echo "no boards on USB"
fi
while read -r mac port; do
    [ -n "$mac" ] || continue
    # `read` fails on a MAC the table does not have, leaving both empty; the check below says why.
    read -r device chip < <(awk -v mac="$mac" '$1 == mac { print $2, $3 }' "$REPO/rendered/boards") || true
    if [ -z "$device" ]; then
        echo "ERROR: the board on $port has MAC $mac, which no device in clockwork.dot has." >&2
        echo "       Add \`mac $mac chip <chip>\` to its device's address in clockwork.dot, the" >&2
        echo "       chip as esptool names it (esp8266, esp32, …), and deploy again." >&2
        exit 1
    fi
    image="$REPO/binaries/$device.bin"
    if [ ! -f "$image" ]; then
        echo "ERROR: the board on $port is $device's, and no firmware for $device was shipped." >&2
        exit 1
    fi
    sum="$(sha256sum "$image" | cut -d ' ' -f 1)"
    if [ "$(sudo cat "$FLASHED/$mac" 2>/dev/null)" = "$sum" ]; then
        echo "$device ($mac) unchanged"
        continue
    fi
    # The image Arduino's ESP8266 core builds is the whole flash from 0x0, its bootloader
    # included. esptool resets the board into it once written. Its stdin is not the board list
    # this loop reads.
    esptool --chip "$chip" --port "$port" --baud 460800 write_flash 0x0 "$image" </dev/null
    echo "$sum" | sudo tee "$FLASHED/$mac" >/dev/null
    echo "flashed $device ($mac) on $port"
done <<<"$boards"

# --- Telemetry -----------------------------------------------------------------
# After everything the show needs, so a missing SSD or a failed download stops the deploy with
# the rig already up. None of it is part of the show: losing it costs the recording and never
# a frame.

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
tank_line="$(grep -v -e '^#' -e '^$' "$REPO/pi/chaos-pi/fstab-tank")"
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
# Debian's Mosquitto, installed when missing, with the rig's settings as an overlay in its
# include directory, so an upgrade cannot revert them. The clients come with it, for
# `mosquitto_sub` on the Pi itself. Nothing waits for it: Stem and the brain, installed above,
# keep running and publish nothing while it is not up.
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
# installed version differs from greptime.version — the tarball is close to 200 MB over the 4G
# bundle — and installed only once its checksum matches. VERSION beside the binary records
# what was installed, so an unchanged pin downloads nothing.
#
# The data directory is made only on a mounted /tank: the mount step above has already failed
# the deploy otherwise, and a directory made on the empty mount point would sit on the SD card.
step "install GreptimeDB"
CHANGED=
. "$REPO/pi/chaos-pi/greptime.version"
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
install_changed 0644 "$REPO/pi/chaos-pi/greptimedb.toml" /etc/greptimedb/greptimedb.toml
install_changed 0644 "$REPO/pi/chaos-pi/greptimedb.service" /etc/systemd/system/greptimedb.service
sudo systemctl enable greptimedb
if [ -n "$CHANGED" ]; then
    sudo systemctl daemon-reload
    restart greptimedb
else
    echo "GreptimeDB unchanged"
fi

# --- Memory --------------------------------------------------------------------
# After the broker and GreptimeDB, which it relays and writes to. Built on the Mac with the
# brain by deploy-chaos-pi.sh. Cargo relinks only when something it builds from changed, so an
# unchanged binary is a deploy that changed nothing Memory runs.
step "install Memory"
CHANGED=
install_changed 0755 "$REPO/memory/memory" /usr/local/bin/memory
install_changed 0644 "$REPO/pi/chaos-pi/memory.service" /etc/systemd/system/memory.service
sudo systemctl enable memory
if [ -n "$CHANGED" ]; then
    sudo systemctl daemon-reload
    restart memory
else
    echo "Memory unchanged"
fi

step "remote deploy complete"
