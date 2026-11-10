#!/usr/bin/env bash
#
# boards.sh — runs ON chaos-pi. Finds the boards on its USB and the MAC each one's chip reports,
# so a board is known by its chip whatever port or cable it is on, and two boards whose USB
# serial chips carry the same name are still told apart.
#
#   boards.sh          every board, one line each: `<mac> <port>`
#   boards.sh <mac>    the port of the board with that MAC; fails if none has it
#
# A board is a serial device in /dev/serial/by-id/ behind one of the USB serial chips ESP boards
# carry; anything else on the bus, the 4G modem among it, is left alone. Asking a chip its MAC
# puts it in its bootloader and restarts it, so every board found here restarts. A board that
# does not answer fails the run, rather than be skipped as if it were not there.
#
set -euo pipefail

# Silicon Labs CP210x, WCH CH34x, and Espressif's own USB.
ESP_VENDORS=(10c4 1a86 303a)

shopt -s nullglob
want="${1-}"
for port in /dev/serial/by-id/*; do
    vendor="$(udevadm info --query=property --name="$port" | sed -n 's/^ID_\(USB_\)\{0,1\}VENDOR_ID=//p' | head -n 1)"
    case " ${ESP_VENDORS[*]} " in
        *" $vendor "*) ;;
        *) continue ;;
    esac
    if ! out="$(esptool --port "$port" read_mac 2>&1)"; then
        echo "boards.sh: $port is an ESP board's USB serial chip, and its chip does not answer:" >&2
        echo "$out" >&2
        exit 1
    fi
    mac="$(sed -n 's/^MAC: *//p' <<<"$out" | head -n 1 | tr 'A-F' 'a-f')"
    if [ -z "$mac" ]; then
        echo "boards.sh: esptool reported no MAC for $port:" >&2
        echo "$out" >&2
        exit 1
    fi
    if [ -z "$want" ]; then
        echo "$mac $port"
    elif [ "$mac" = "$want" ]; then
        echo "$port"
        exit 0
    fi
done

if [ -n "$want" ]; then
    echo "boards.sh: no board on USB has MAC $want" >&2
    exit 1
fi
