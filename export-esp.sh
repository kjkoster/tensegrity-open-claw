LIBCLANG_DIR="${HOME}/.rustup/toolchains/esp/xtensa-esp32-elf-clang/esp-20.1.1_20250829/esp-clang/lib"
ESP_BIN_DIR="${HOME}/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/xtensa-esp-elf/bin"

if [ ! -d "${LIBCLANG_DIR}" ]; then
    echo "Error: LIBCLANG directory not found: ${LIBCLANG_DIR}" >&2
    return 1
fi

if [ ! -d "${ESP_BIN_DIR}" ]; then
    echo "Error: ESP toolchain bin directory not found: ${ESP_BIN_DIR}" >&2
    return 1
fi

export LIBCLANG_PATH="${LIBCLANG_DIR}"
export PATH="${ESP_BIN_DIR}:$PATH"
