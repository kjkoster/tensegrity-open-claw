#!/usr/bin/env bash
#
# build.sh — generate Wrist's generated.h from the rig's source files, then compile the
# firmware for the NodeMCU with the core pinned in sketch.yaml.
#
# Run from anywhere:  wrist/build.sh
# The image lands in $WRIST_BUILD_DIR, by default target/arduino/ under the git-ignored
# workspace target/, beside the Cargo builds. generated.h is git-ignored too.
#
set -euo pipefail
WRIST_BUILD_DIR="${WRIST_BUILD_DIR:-$(cd "$(dirname "$0")/.." && pwd)/target/arduino}"
cd "$(dirname "$0")"

python3 -I generate.py ../clockwork.dot ../clockwork.qxw ../fixtures/wrist.qxf \
    ../fixtures/wrist.oscquery.json generated.h
arduino-cli compile --profile nodemcu --warnings all --output-dir "$WRIST_BUILD_DIR" .
