#!/usr/bin/env bash
#
# Build and flash a binary to a XIAO nRF52840 over USB DFU.
#
# Usage, from the repo root:
#   ./scripts/flash.sh
#
# The board ships with Adafruit's UF2/DFU bootloader, so no debug probe is
# needed. Double-tapping reset enters the bootloader, which exposes a serial
# port that adafruit-nrfutil talks to.
#
# Pipeline: cargo build -> ELF -> .hex (rust-objcopy) -> .zip DFU package
# (adafruit-nrfutil) -> serial DFU upload.
#
# set -e: stop on first error, -u: error on undefined vars,
# -o pipefail: a failure anywhere in a pipe fails the whole pipe.
set -euo pipefail

TARGET=thumbv7em-none-eabihf
# Installed into a venv rather than system-wide; setup command is printed below if it is missing.
NRFUTIL="scripts/.venv/bin/adafruit-nrfutil"

BIN_NAME="xiao-firmware"
CARGO_BUILD_ARGS=(-p "$BIN_NAME")

ELF="target/${TARGET}/release/${BIN_NAME}"
HEX="target/${TARGET}/release/${BIN_NAME}.hex"
ZIP="target/${TARGET}/release/${BIN_NAME}.zip"

cargo build --release --target "$TARGET" "${CARGO_BUILD_ARGS[@]}"

if ! command -v rust-objcopy &>/dev/null; then
	cargo install cargo-binutils
	rustup component add llvm-tools
fi

if [ ! -x "$NRFUTIL" ]; then
	echo "adafruit-nrfutil not found at $NRFUTIL — set it up with:"
	echo "  python3 -m venv scripts/.venv && scripts/.venv/bin/pip install adafruit-nrfutil"
	exit 1
fi

rust-objcopy -O ihex "$ELF" "$HEX"

# --dev-type 0x0052 is required by Seeed's own Arduino board definition for
# this board; the DFU package format nrfutil expects.
"$NRFUTIL" dfu genpkg --dev-type 0x0052 --application "$HEX" "$ZIP"

# Double-tapping reset within ~half a second enters the bootloader. The board
# re-enumerates as a different serial device, which is what we flash over.
echo "Double-tap reset on the XIAO now, then press enter."
read -r

# Find the board by USB vendor ID rather than assuming ttyACM0 — with two
# boards plugged in, the numbering is not stable. 2886 is Seeed Studio.
PORT=""
for dev in /dev/ttyACM*; do
	[ -e "$dev" ] || continue
	if udevadm info -q property -n "$dev" 2>/dev/null | grep -q "ID_VENDOR_ID=2886"; then
		PORT="$dev"
		break
	fi
done

if [ -z "$PORT" ]; then
	echo "Couldn't find the XIAO's serial port — check 'ls /dev/ttyACM*' and set PORT manually."
	exit 1
fi

echo "Flashing over $PORT..."
"$NRFUTIL" --verbose dfu serial -pkg "$ZIP" -p "$PORT" -b 115200 --singlebank
echo "Flashed."
