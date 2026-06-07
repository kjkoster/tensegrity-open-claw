#!/bin/sh

exec probe-rs attach --chip=esp32s3 target/xtensa-esp32s3-none-elf/debug/ponytail
