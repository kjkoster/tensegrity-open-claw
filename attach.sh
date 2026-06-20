#!/usr/bin/env bash
#
# attach.sh -- attach probe-rs to the esp32s3 target, choosing among the
# connected JTAG probes.
#
#   ./attach.sh            no selector -> probe-rs default (errors if >1 probe)
#   ./attach.sh 0          pick the probe at that index in `probe-rs list`
#   ./attach.sh AC:A7      pick the probe whose selector contains that substring
#
set -euo pipefail

CHIP="esp32s3"
BINARY="/home/kjkoster/binaries/ponytail"

usage() {
    cat >&2 <<EOF
usage: $(basename "$0") [PROBE]

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
