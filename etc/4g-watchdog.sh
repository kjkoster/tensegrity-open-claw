#!/usr/bin/env bash
#
# Escalating watchdog for the USB 4G modem, the Pi's only uplink: when it wedges, nobody can
# ssh in to fix it, and the rigs stand hours away with nobody on site. Runs once a minute from a
# systemd timer.
#
# The modem is found afresh on every run, starting from the most reliable fact about it — a
# Huawei device on the USB bus — and working up: its network interface and its control port are
# read off that USB device in sysfs, and the ModemManager modem is the one whose physical
# device is that USB device. Nothing here names a port, a hub or an interface, because the
# control cabinet is reshuffled for every show and build, and a hardcoded one is wrong the next
# time.
#
# On a run of failed checks it climbs a ladder of remedies that touch the modem alone, so none
# of them disturbs the show:
#
#   minute FAIL_RECONNECT    NetworkManager disconnects and reconnects the modem
#   minute FAIL_RESET        ModemManager resets the modem
#   minute FAIL_REENUMERATE  the modem is dropped from the USB bus and re-enumerated, a replug
#                            without cutting power; repeated every REENUMERATE_EVERY minutes
#                            for as long as the outage lasts
#
# Each of the first two fires once, on the minute it is reached, and the gap to the next is what
# gives it time to work: a modem needs 30–60 s to register after a reset, so a remedy re-applied
# every minute never lets it finish coming back.
#
# There is deliberately no reboot and no power cut. A reboot breaks the show for a link the show
# does not need, and a warm reboot does not cut USB power anyway. A power cut can only be trusted
# to cut the modem's port alone when the hub really switches ports individually, and a hub's own
# claim to do so is not reliable; on a hub that gangs its ports it would cut the audio interface
# beside the modem too.
#
# Separately from the ladder: when the internet is up but the WireGuard tunnel is not, wg-quick
# is restarted. That covers the tunnel-side failures the uplink ladder cannot see.
#
# Installed at /usr/local/sbin/4g-watchdog.sh by remote-deploy.sh. The configuration below is
# the source of truth for it: edited on the Pi, it is gone at the next deploy.

set -u

# --- configuration ------------------------------------------------------------
# The modem's USB vendor: Huawei. The vendor rather than a product ID, because a Huawei stick
# changes its product ID with its USB mode, and the rig has no other Huawei hardware.
MODEM_VENDOR=12d1

# Connectivity targets, tried in order; the link counts as up when ANY answers.
PING_TARGETS="1.1.1.1 8.8.8.8"

# WireGuard: the server's tunnel IP and the wg-quick unit. When the internet
# is up but this IP does not answer WG_TUNNEL_FAILS times in a row, the unit
# is restarted. Leave WG_TUNNEL_IP empty to disable the tunnel check.
WG_TUNNEL_IP="10.8.0.1"
WG_UNIT="wg-quick@wg0"
WG_TUNNEL_FAILS=2

# Escalation thresholds, in consecutive failed checks (= minutes of outage).
FAIL_RECONNECT=3
FAIL_RESET=5
FAIL_REENUMERATE=8
# The last rung repeats, because past it there is nothing stronger left to try.
REENUMERATE_EVERY=5
# How long the modem stays off the bus: long enough for the kernel and ModemManager to let go
# of it, so it comes back as a new device rather than a resumed one.
REENUMERATE_PAUSE_S=5
# -------------------------------------------------------------------------------

STATE_DIR=/run/4g-watchdog
FAIL_FILE=$STATE_DIR/consecutive-failures
WG_FAIL_FILE=$STATE_DIR/wg-consecutive-failures
mkdir -p "$STATE_DIR"

log() {
    logger -t 4g-watchdog "$*"
}

# The modem's USB device, as its sysfs name (e.g. 1-1.1.3.4), or nothing. Interfaces (names with
# a colon) and root hubs carry no idVendor of their own worth matching, so only devices are read.
find_modem() {
    local device
    for device in /sys/bus/usb/devices/*; do
        case "${device##*/}" in *:*) continue ;; esac
        if [ "$(cat "$device/idVendor" 2> /dev/null)" = "$MODEM_VENDOR" ]; then
            echo "${device##*/}"
            return
        fi
    done
}

# The first entry of `kind` (net, usbmisc) under any of the modem's USB interfaces. Any rather
# than a fixed one, because which interface carries it depends on the USB configuration the
# stick came up in.
modem_child() {
    local usb=$1 kind=$2 entry
    for entry in /sys/bus/usb/devices/"$usb":*/"$kind"/*; do
        [ -e "$entry" ] && { echo "${entry##*/}"; return; }
    done
}

# The ModemManager modem whose physical device is this USB device, as its D-Bus path, or nothing
# when ModemManager does not have it.
mm_modem() {
    local usb=$1 path
    for path in $(mmcli -L -K 2> /dev/null | sed -n 's/^modem-list\.value\[[0-9]*\] *: *//p'); do
        if mmcli -m "$path" -K 2> /dev/null \
            | grep -q "^modem\.generic\.physdev *: .*/$usb\$"; then
            echo "$path"
            return
        fi
    done
}

link_is_up() {
    local target
    [ -n "$IFACE" ] || return 1
    for target in $PING_TARGETS; do
        if ping -I "$IFACE" -c 1 -W 5 "$target" > /dev/null 2>&1; then
            return 0
        fi
    done
    return 1
}

tunnel_is_up() {
    ping -c 1 -W 5 "$WG_TUNNEL_IP" > /dev/null 2>&1
}

# Tunnel repair, independent of the uplink ladder: internet works but the
# WireGuard server's tunnel IP does not answer.
check_tunnel() {
    [ -n "$WG_TUNNEL_IP" ] || return 0
    local wg_failures
    wg_failures=$(cat "$WG_FAIL_FILE" 2> /dev/null || echo 0)
    if tunnel_is_up; then
        if [ "$wg_failures" -gt 0 ]; then
            log "tunnel recovered after $wg_failures failed checks"
        fi
        echo 0 > "$WG_FAIL_FILE"
        return 0
    fi
    wg_failures=$((wg_failures + 1))
    echo "$wg_failures" > "$WG_FAIL_FILE"
    log "internet up but tunnel check failed ($wg_failures consecutive)"
    if [ "$wg_failures" -ge "$WG_TUNNEL_FAILS" ]; then
        log "restarting $WG_UNIT"
        systemctl restart "$WG_UNIT"
        echo 0 > "$WG_FAIL_FILE"
    fi
}

# Rung 1: NetworkManager drops the connection and brings it back. Cheap, and enough when only
# the Linux side lost track. NetworkManager knows a modem by its control port, not its network
# interface.
reconnect() {
    if [ -z "$CONTROL" ]; then
        log "not reconnecting: modem $USB has no control port in sysfs"
        return
    fi
    log "reconnecting $CONTROL via NetworkManager"
    nmcli device disconnect "$CONTROL" > /dev/null 2>&1
    nmcli device connect "$CONTROL" > /dev/null 2>&1 \
        || log "NetworkManager could not reconnect $CONTROL"
}

# Rung 2: ModemManager resets the modem's firmware. Only possible while ModemManager still has
# the modem; when it has lost it, the next rung is the one that brings it back.
reset_modem() {
    local modem
    modem=$(mm_modem "$USB")
    if [ -z "$modem" ]; then
        log "not resetting: ModemManager has no modem on USB $USB"
        return
    fi
    log "resetting $modem via ModemManager"
    mmcli -m "$modem" --reset > /dev/null 2>&1 || log "ModemManager could not reset $modem"
}

# Rung 3: off the bus and back on, as a replug would, without touching power. Acts on the modem's
# own USB device alone, so the hub and everything else on it are left as they are.
reenumerate() {
    log "re-enumerating USB $USB"
    if ! echo 0 > "/sys/bus/usb/devices/$USB/authorized"; then
        log "could not take USB $USB off the bus"
        return
    fi
    sleep "$REENUMERATE_PAUSE_S"
    echo 1 > "/sys/bus/usb/devices/$USB/authorized" || log "could not bring USB $USB back on the bus"
}

USB=$(find_modem)
IFACE=""
CONTROL=""
if [ -n "$USB" ]; then
    IFACE=$(modem_child "$USB" net)
    CONTROL=$(modem_child "$USB" usbmisc)
fi

failures=$(cat "$FAIL_FILE" 2> /dev/null || echo 0)

if link_is_up; then
    if [ "$failures" -gt 0 ]; then
        log "uplink recovered after $failures failed checks"
    fi
    echo 0 > "$FAIL_FILE"
    check_tunnel
    exit 0
fi

failures=$((failures + 1))
echo "$failures" > "$FAIL_FILE"
if [ -z "$USB" ]; then
    # Nothing on the bus to act on, and no remembered port to try: a modem that has gone is
    # logged, and the ladder resumes the moment it is back.
    log "uplink check failed ($failures consecutive): no modem (USB vendor $MODEM_VENDOR) on the bus"
    exit 0
fi
log "uplink check failed ($failures consecutive) on USB $USB, interface ${IFACE:-none}, control port ${CONTROL:-none}"

if [ "$failures" -ge "$FAIL_REENUMERATE" ]; then
    if [ $(((failures - FAIL_REENUMERATE) % REENUMERATE_EVERY)) -eq 0 ]; then
        reenumerate
    fi
elif [ "$failures" -eq "$FAIL_RESET" ]; then
    reset_modem
elif [ "$failures" -eq "$FAIL_RECONNECT" ]; then
    reconnect
fi
