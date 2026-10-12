#!/usr/bin/env bash
#
# Escalating watchdog for the 4G USB dongle. A brownout (or plain firmware
# mood) can wedge the dongle so badly that only cutting its power brings it
# back — and since the dongle is the Pi's uplink, nobody can ssh in to do
# that by hand. Runs once a minute from a systemd timer and walks a ladder:
#
#   minute FAIL_RESTART  restart the connection (nmcli / mmcli / ip link)
#   minute FAIL_HILINK   ask the HiLink dongle to reboot itself via its API
#   minute FAIL_USB      cut USB power with uhubctl (sysfs unbind as fallback)
#   minute FAIL_REBOOT   reboot the Pi (guarded against reboot loops)
#
# Each of the first three rungs fires ONCE, on the minute it is reached, and
# then the ladder waits for the next one. Repeating them every minute was
# worse than doing nothing: a dongle needs 30-60s to register after a reset,
# so a remedy re-applied each minute is a remedy that never lets the dongle
# finish coming back.
#
# NOTE a warm Pi reboot does NOT cut USB VBUS — a wedged dongle rides
# through it. The reboot rung only fixes Linux-side problems; the rung that
# clears a wedged dongle is the uhubctl one, so its hub/port MUST be
# verified by eye: run the cycle command and confirm the dongle LED goes
# dark. On a Pi 4 the dongle is usually under hub 1-1 (ports ganged), NOT
# hub 2 (that is the USB3 root hub — cycling it exits 0 without touching
# the dongle).
#
# The reboot rung is the exception, because it can be deferred: it is tried
# on every minute past its threshold, and refuses itself when the Pi has just
# booted, when the interface is not there to be fixed, and when it already
# rebooted recently. A show that reboots every quarter of an hour because the
# campsite has no coverage is worse than a show with no telemetry.
#
# Separately from the ladder: when the internet is up but the WireGuard
# tunnel is not, wg-quick is restarted. That covers the tunnel-side
# failures the uplink ladder can't see.
#
# Installed at /usr/local/sbin/4g-watchdog.sh by remote-deploy.sh. The configuration below
# is the source of truth for it: edited on the Pi, it is gone at the next deploy.

set -u

# --- configuration ------------------------------------------------------------
# The dongle's network interface. HiLink dongles present as an ethernet
# interface (eth1 or an enx... name), QMI/MBIM modems as wwan0, serial ones
# as ppp0. Check with: ip -br link
IFACE=eth1

# Connectivity targets, tried in order; the link counts as up when ANY answers.
PING_TARGETS="1.1.1.1 8.8.8.8"

# WireGuard: the server's tunnel IP and the wg-quick unit. When the internet
# is up but this IP does not answer WG_TUNNEL_FAILS times in a row, the unit
# is restarted. Leave WG_TUNNEL_IP empty to disable the tunnel check.
WG_TUNNEL_IP="10.8.0.1"
WG_UNIT="wg-quick@wg0"
WG_TUNNEL_FAILS=2

# HiLink dongle web API (192.168.8.1 on stock firmware). Used to ask the
# dongle to reboot itself — gentler than cutting power and works even when
# uhubctl is not an option. Leave empty to skip this rung. If the dongle's
# web UI is password-protected this best-effort call will fail and the
# ladder simply continues to the USB rung.
HILINK_BASE="http://192.168.8.1"

# uhubctl hub location and port for the USB power cycle. Find them with
# `sudo uhubctl` (the hub whose device listing shows the Huawei), then
# VERIFY by running the cycle and watching the dongle LED go dark:
#   sudo uhubctl -l <loc> -p <port> -a cycle -d 5
# Pi 4: usually -l 1-1 with ports ganged. Pi 3B+: often -l 1-1.1 -p 2.
# Leave LOCATION empty to skip straight to the sysfs unbind/rebind fallback.
#
# Port 2 rather than 1, because the sysfs id read off this Pi by matching
# idVendor:idProduct was 1-1.2 — which is this hub's port 2. The two used to
# disagree and it did not show, because a Pi 4 gangs its ports: cutting port
# 1 cuts the dongle anyway, so a wrong port number here works right up until
# the day it is a Pi whose ports are switched individually.
UHUBCTL_LOCATION="1-1"
UHUBCTL_PORT="2"
POWER_OFF_SECONDS=5

# sysfs device id for the unbind/rebind fallback. Weaker than a real power
# cut (resets the USB side only), but better than nothing.
#
# Empty means "whatever port uhubctl is aimed at", which is the only setting
# that cannot quietly disagree with the rung above — and disagreeing is the
# failure that hides, because both rungs report success while one of them
# resets a device the other never touched. Set it only to name a device that
# is genuinely somewhere else: compare `lsusb` against
# `ls /sys/bus/usb/devices` and pick the id whose .../idVendor:idProduct
# matches the dongle. Setting it to "-" skips the fallback altogether.
USB_DEV=""

# Escalation thresholds, in consecutive failed checks (= minutes of outage).
# Each rung fires once, on the minute it is reached, and the gap to the next
# is what gives that remedy time to work — a dongle reboot alone takes 30-60s.
FAIL_RESTART=3
FAIL_HILINK=5
FAIL_USB=8
FAIL_REBOOT=13

# Never reboot within this many seconds of boot: whatever is wrong, a Pi that
# has only just come up has not yet had time to fix it.
MIN_UPTIME_FOR_REBOOT=900

# And never twice inside this many seconds. The failure counter lives in /run
# and a reboot is precisely the event that clears it, so without a stamp that
# outlives the boot, a dead SIM or a campsite with no coverage reboots the rig
# every quarter of an hour, for as long as the outage lasts, in front of
# whoever is watching the show.
MIN_SECONDS_BETWEEN_REBOOTS=3600
# -------------------------------------------------------------------------------

STATE_DIR=/run/4g-watchdog
FAIL_FILE=$STATE_DIR/consecutive-failures
WG_FAIL_FILE=$STATE_DIR/wg-consecutive-failures
mkdir -p "$STATE_DIR"

# The one piece of state that has to survive the reboot it records. /var/lib
# rather than /run for exactly that reason.
STAMP_DIR=/var/lib/4g-watchdog
LAST_REBOOT_FILE=$STAMP_DIR/last-reboot
mkdir -p "$STAMP_DIR"

log() {
    logger -t 4g-watchdog "$*"
}

# Asked separately from the ping, because the two failures want different
# remedies: no interface is a dongle that has fallen off the bus or an IFACE
# that names something this Pi has never had, and neither of those is fixed by
# a reboot. The ladder still climbs — a vanished dongle is exactly what the
# power cut is for — but the reboot rung refuses itself while this is false.
iface_exists() {
    ip link show "$IFACE" > /dev/null 2>&1
}

link_is_up() {
    local target
    iface_exists || return 1
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

# Ladder step 1: ask whatever network manager is present to bring the
# connection back up. Cheap, and enough when only the Linux side lost track.
restart_connection() {
    if command -v nmcli > /dev/null 2>&1; then
        log "restarting connection on $IFACE via NetworkManager"
        nmcli device disconnect "$IFACE" > /dev/null 2>&1
        nmcli device connect "$IFACE" > /dev/null 2>&1 && return
    fi
    if command -v mmcli > /dev/null 2>&1; then
        log "resetting modem via ModemManager"
        mmcli -m any --reset > /dev/null 2>&1 && return
    fi
    log "bouncing $IFACE via ip link"
    ip link set "$IFACE" down 2> /dev/null
    sleep 2
    ip link set "$IFACE" up 2> /dev/null
}

# Ladder step 2: ask the HiLink firmware to reboot the dongle. Best-effort:
# needs the stock unauthenticated API; a password-protected or thoroughly
# wedged firmware will refuse, and the ladder moves on to cutting power.
hilink_reboot() {
    [ -n "$HILINK_BASE" ] || { log "no HILINK_BASE configured, skipping"; return 1; }
    command -v curl > /dev/null 2>&1 || { log "curl not installed, skipping HiLink rung"; return 1; }
    local ses_tok ses tok reply
    ses_tok=$(curl -s -m 5 "$HILINK_BASE/api/webserver/SesTokInfo") || {
        log "HiLink API unreachable"; return 1; }
    ses=$(printf '%s' "$ses_tok" | sed -n 's/.*<SesInfo>\(.*\)<\/SesInfo>.*/\1/p')
    tok=$(printf '%s' "$ses_tok" | sed -n 's/.*<TokInfo>\(.*\)<\/TokInfo>.*/\1/p')
    reply=$(curl -s -m 5 -X POST "$HILINK_BASE/api/device/control" \
        -H "Cookie: $ses" \
        -H "__RequestVerificationToken: $tok" \
        -H "Content-Type: application/xml" \
        --data '<?xml version="1.0" encoding="UTF-8"?><request><Control>1</Control></request>')
    if printf '%s' "$reply" | grep -q "<response>OK</response>"; then
        log "HiLink dongle accepted reboot request"
        return 0
    fi
    log "HiLink reboot refused: $reply"
    return 1
}

# Ladder step 3: the equivalent of pulling the dongle out and plugging it
# back in. uhubctl actually cuts VBUS; the sysfs unbind/rebind fallback only
# resets the USB side, which is weaker but better than nothing.
power_cycle_usb() {
    if [ -n "$UHUBCTL_LOCATION" ] && command -v uhubctl > /dev/null 2>&1; then
        log "power-cycling USB hub $UHUBCTL_LOCATION port $UHUBCTL_PORT"
        uhubctl -l "$UHUBCTL_LOCATION" -p "$UHUBCTL_PORT" -a cycle \
            -d "$POWER_OFF_SECONDS" > /dev/null 2>&1 && return
        log "uhubctl failed, falling back to driver unbind/rebind"
    fi
    local device="$USB_DEV"
    if [ -z "$device" ] && [ -n "$UHUBCTL_LOCATION" ]; then
        device="$UHUBCTL_LOCATION.$UHUBCTL_PORT"
    fi
    if [ "$device" = "-" ]; then
        log "sysfs fallback disabled (USB_DEV=-)"
    elif [ -n "$device" ] && [ -e "/sys/bus/usb/devices/$device" ]; then
        log "unbinding and rebinding USB device $device"
        echo "$device" > /sys/bus/usb/drivers/usb/unbind 2> /dev/null
        sleep "$POWER_OFF_SECONDS"
        echo "$device" > /sys/bus/usb/drivers/usb/bind 2> /dev/null
    elif [ -n "$device" ]; then
        # Named, and not there. Worth saying out loud with the neighbours
        # listed: this is the rung that silently does nothing when the id is
        # stale, and the answer is almost always in that listing.
        log "USB device $device is not in /sys/bus/usb/devices — present: $(ls /sys/bus/usb/devices 2> /dev/null | tr '\n' ' ')"
    else
        log "no usable USB recovery configured (UHUBCTL_LOCATION/USB_DEV)"
    fi
}

# Ladder step 4: reboot. Fixes anything wrong on the Linux side, but note:
# it does NOT cut USB power, so a dongle the USB rung failed to revive will
# still be wedged afterwards. Last resort, not a guarantee.
reboot_pi() {
    local uptime now last
    uptime=$(cut -d. -f1 /proc/uptime 2> /dev/null || echo 0)
    if [ "$uptime" -lt "$MIN_UPTIME_FOR_REBOOT" ]; then
        log "uplink still down but uptime ${uptime}s < ${MIN_UPTIME_FOR_REBOOT}s, holding off reboot"
        return
    fi
    # A reboot does not cut USB power and cannot invent an interface that is
    # not there, so against a dongle that has fallen off the bus — or an IFACE
    # naming something this Pi has never had — it buys nothing and costs the
    # show. The USB rung is the one that could still bring it back, and it has
    # already had its turn by the time this is reached.
    if ! iface_exists; then
        log "not rebooting: interface $IFACE does not exist, which no reboot fixes — check ip -br link against IFACE"
        return
    fi
    now=$(date +%s)
    last=$(cat "$LAST_REBOOT_FILE" 2> /dev/null || echo 0)
    if [ "$((now - last))" -lt "$MIN_SECONDS_BETWEEN_REBOOTS" ]; then
        log "not rebooting: last watchdog reboot was $((now - last))s ago, under ${MIN_SECONDS_BETWEEN_REBOOTS}s — treating this as an outage to sit out"
        return
    fi
    echo "$now" > "$LAST_REBOOT_FILE"
    log "uplink down for $failures checks, rebooting"
    systemctl reboot
}

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
if iface_exists; then
    log "uplink check failed ($failures consecutive)"
else
    log "uplink check failed ($failures consecutive): interface $IFACE does not exist (see ip -br link)"
fi

# `-eq` on the three remedies and `-ge` only on the reboot. The counter climbs
# by exactly one per run, so equality fires each rung once and the gap to the
# next rung is what lets the remedy work. The reboot keeps `-ge` because it is
# the one rung that refuses itself and therefore has to be able to come back:
# hit at a five-minute uptime it declines, and asks again a minute later.
if [ "$failures" -ge "$FAIL_REBOOT" ]; then
    reboot_pi
elif [ "$failures" -eq "$FAIL_USB" ]; then
    power_cycle_usb
elif [ "$failures" -eq "$FAIL_HILINK" ]; then
    hilink_reboot
elif [ "$failures" -eq "$FAIL_RESTART" ]; then
    restart_connection
fi
