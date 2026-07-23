#!/usr/bin/env bash
# Build the StepForge engine as an xcframework and generate the C header.
# Outputs:
#   engine/include/sequencer_engine.h        (cbindgen; committed)
#   engine/dist/SequencerEngine.xcframework  (gitignored)
set -euo pipefail

# Ensure rustup-managed cargo/cbindgen are found even when Homebrew rust shadows
# rustup, or the script runs under Xcode's minimal preBuildScript PATH.
export PATH="$HOME/.cargo/bin:$PATH"

ENGINE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ENGINE_DIR"

DIST_DIR="$ENGINE_DIR/dist"
INCLUDE_DIR="$ENGINE_DIR/include"
HEADER="$INCLUDE_DIR/sequencer_engine.h"
mkdir -p "$DIST_DIR" "$INCLUDE_DIR"

echo "[build] generating C header (cbindgen) ..."
cbindgen --crate sequencer_engine_ffi --config cbindgen.toml --output "$HEADER" --lang c

echo "[build] compiling iOS & macOS slices (release) ..."
cargo build --release --target aarch64-apple-ios
cargo build --release --target aarch64-apple-ios-sim
cargo build --release --target x86_64-apple-ios
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

LIB_DEVICE="target/aarch64-apple-ios/release/libsequencer_engine_ffi.a"
LIB_SIM_ARM="target/aarch64-apple-ios-sim/release/libsequencer_engine_ffi.a"
LIB_SIM_X86="target/x86_64-apple-ios/release/libsequencer_engine_ffi.a"
LIB_MAC_ARM="target/aarch64-apple-darwin/release/libsequencer_engine_ffi.a"
LIB_MAC_X86="target/x86_64-apple-darwin/release/libsequencer_engine_ffi.a"

# Modern Xcode rejects separate arch slices for the same platform variant as
# "equivalent library definitions". Merge simulator and macOS slices into universal archives.
echo "[build] merging simulator and macOS slices (lipo) ..."
mkdir -p target/xcframework-staging
LIB_SIM_UNIVERSAL="target/xcframework-staging/libsequencer_engine_ffi_sim.a"
LIB_MAC_UNIVERSAL="target/xcframework-staging/libsequencer_engine_ffi_mac.a"

lipo -create "$LIB_SIM_ARM" "$LIB_SIM_X86" -output "$LIB_SIM_UNIVERSAL"
lipo -create "$LIB_MAC_ARM" "$LIB_MAC_X86" -output "$LIB_MAC_UNIVERSAL"
lipo -info "$LIB_SIM_UNIVERSAL"
lipo -info "$LIB_MAC_UNIVERSAL"

echo "[build] assembling xcframework ..."
rm -rf "$DIST_DIR/SequencerEngine.xcframework"
xcodebuild -create-xcframework \
  -library "$LIB_DEVICE"        -headers "$INCLUDE_DIR" \
  -library "$LIB_SIM_UNIVERSAL" -headers "$INCLUDE_DIR" \
  -library "$LIB_MAC_UNIVERSAL" -headers "$INCLUDE_DIR" \
  -output "$DIST_DIR/SequencerEngine.xcframework"

echo "[build] done -> $DIST_DIR/SequencerEngine.xcframework"
