#!/usr/bin/env bash
#
# attach.sh -- attach probe-rs to the esp32s3 target, choosing among the
# connected JTAG probes.
#
# This is one file living in two places: the source tree and, rsynced, on the
# probe host (claw-pi). On claw-pi it attaches locally. Anywhere else it
# re-execs the rsynced copy on claw-pi over ssh, so the probe-selection logic
# below runs in exactly one place -- on the machine that actually holds the
# probes -- and the dev-tree invocation is just a thin ssh wrapper.
#
#   ./attach.sh            no selector -> probe-rs default (errors if >1 probe)
#   ./attach.sh 0          pick the probe at that index in `probe-rs list`
#   ./attach.sh AC:A7      pick the probe whose selector contains that substring
#
set -euo pipefail

CHIP="esp32s3"
BINARY="/home/kjkoster/binaries/ponytail"

# Probe host. Off this host we hand off to the copy of this script that lives
# there; on it, we attach locally. REMOTE_SCRIPT's ~ is left unexpanded locally
# (it is single-quoted and used inside double quotes) so it expands in the
# remote login shell instead.
REMOTE_HOST="claw-pi"
REMOTE_USER="kjkoster"
REMOTE_SCRIPT='~/attach.sh'

usage() {
    cat >&2 <<EOF
usage: $(basename "$0") [PROBE]

  On $REMOTE_HOST this attaches locally; anywhere else it re-execs this same
  (rsynced) script on $REMOTE_HOST over ssh, forwarding any argument.

  (no arg)   attach using probe-rs's own probe selection
  <number>   attach to the probe at that index in 'probe-rs list'
  <string>   attach to the probe whose VID:PID:Serial selector contains
             <string> (any unique substring; case-insensitive)
EOF
}

# Emit the "VID:PID:Serial" selector of every probe, in `probe-rs list` order.
# A list line looks like:
#   0: ESP JTAG -- 303a:1001:DC:B4:D9:3B:B1:A4 (EspJtag)
# We take the field between " -- " and " (", which is exactly what --probe wants.
list_selectors() {
    probe-rs list | sed -n 's/.* -- \(.*\) (.*/\1/p'
}

main() {
    case "${1-}" in
        -h|--help) usage; exit 0 ;;
    esac

    # If we're not on the probe host, hand off to the rsynced copy there.
    #
    # ssh runs a remote command under a *non-login, non-interactive* shell,
    # which does NOT read the profile files (.profile/.bash_profile/.bashrc)
    # that put cargo's bin dir -- where probe-rs lives -- on PATH. Running it
    # bare therefore fails with "probe-rs: command not found". Re-exec under a
    # login shell (bash -lc) so the remote environment matches an interactive
    # session on the Pi. -t gives probe-rs a TTY so live RTT output and Ctrl-C
    # behave; ~ in REMOTE_SCRIPT expands in that remote shell.
    local here
    here="$(hostname)"
    here="${here%%.*}"   # tolerate a FQDN like claw-pi.local
    if [ "$here" != "$REMOTE_HOST" ]; then
        local remote_cmd="$REMOTE_SCRIPT"
        [ "$#" -gt 0 ] && remote_cmd+=" $(printf '%q ' "$@")"
        exec ssh -t "${REMOTE_USER}@${REMOTE_HOST}" "bash -lc $(printf '%q' "$remote_cmd")"
    fi

    # ── From here down we are on the probe host: attach locally. ─────────────

    # No argument: original behaviour, let probe-rs pick.
    if [ "$#" -eq 0 ]; then
        exec probe-rs attach --chip "$CHIP" "$BINARY"
    fi

    local arg="$1"
    mapfile -t selectors < <(list_selectors)
    if [ "${#selectors[@]}" -eq 0 ]; then
        echo "attach.sh: no probes reported by 'probe-rs list'" >&2
        exit 1
    fi

    local selector=""
    if [[ "$arg" =~ ^[0-9]+$ ]]; then
        # Numeric -> index.
        if [ "$arg" -ge "${#selectors[@]}" ]; then
            echo "attach.sh: no probe at index $arg (have ${#selectors[@]}: 0..$(( ${#selectors[@]} - 1 )))" >&2
            exit 1
        fi
        selector="${selectors[$arg]}"
    else
        # String -> unique case-insensitive substring match.
        local matches=() s
        for s in "${selectors[@]}"; do
            [[ "${s,,}" == *"${arg,,}"* ]] && matches+=("$s")
        done
        case "${#matches[@]}" in
            0) echo "attach.sh: no probe selector contains '$arg'" >&2; exit 1 ;;
            1) selector="${matches[0]}" ;;
            *) echo "attach.sh: '$arg' is ambiguous, matches:" >&2
               printf '  %s\n' "${matches[@]}" >&2; exit 1 ;;
        esac
    fi

    echo "attach.sh: using probe $selector" >&2
    exec probe-rs attach --probe "$selector" --chip "$CHIP" "$BINARY"
}

main "$@"
