#!/usr/bin/env bash
#
# build.sh — generate Wrist's generated.h from the rig's source files, then compile the
# firmware for the NodeMCU with the core pinned in sketch.yaml.
#
# Run from anywhere:  wrist/build.sh
# The image lands in wrist/build/; both it and generated.h are git-ignored.
#
set -euo pipefail
cd "$(dirname "$0")"

python3 -I generate.py ../clockwork.dot ../clockwork.qxw ../fixtures/wrist.qxf \
    ../fixtures/wrist.oscquery.json generated.h
arduino-cli compile --profile nodemcu --warnings all --output-dir build .
