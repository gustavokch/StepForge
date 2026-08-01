#!/usr/bin/env bash
# Bundle the StepForge CLAP plugin (Release) and install it into
# ~/Library/Audio/Plug-Ins/CLAP/ so a host can load it.
#
# Runs `cargo xtask bundle -p stepforge_clap --release` (cargo-incremental,
# fast on repeat runs), then ditto-copies the produced .clap over any previous
# install and strips the quarantine attribute (the bundle is unsigned locally).
#
# Usage:
#   engine/scripts/install_clap.sh
set -euo pipefail

# Homebrew rust shadows rustup; cargo must resolve to the rustup toolchain.
# (Mirrors setup.sh / build_engine.sh / build_install_macos.sh.)
export PATH="$HOME/.cargo/bin:$PATH"

ENGINE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLAP_PKG="stepforge_clap"
BUNDLE_NAME="${CLAP_PKG}.clap"
BUILT_BUNDLE="$ENGINE_DIR/target/bundled/$BUNDLE_NAME"

CLAP_DIR="$HOME/Library/Audio/Plug-Ins/CLAP"
INSTALLED_BUNDLE="$CLAP_DIR/$BUNDLE_NAME"

log() { echo "[$1] $2"; }
start=$SECONDS

# -----------------------------------------------------------------------------
# 0. Preflight
# -----------------------------------------------------------------------------
command -v cargo >/dev/null 2>&1 || {
  echo "[error] cargo not found (install rustup from https://rustup.rs and put ~/.cargo/bin on PATH)" >&2
  exit 1
}

mkdir -p "$CLAP_DIR"

# -----------------------------------------------------------------------------
# 1. Bundle the plugin (Release)
# -----------------------------------------------------------------------------
log bundle "cargo xtask bundle -p $CLAP_PKG --release ..."
( cd "$ENGINE_DIR" && cargo xtask bundle -p "$CLAP_PKG" --release )

[ -d "$BUILT_BUNDLE" ] || {
  echo "[error] bundle produced no .clap at $BUILT_BUNDLE" >&2
  exit 1
}
log bundle "done -> $BUILT_BUNDLE"

# -----------------------------------------------------------------------------
# 2. Install into ~/Library/Audio/Plug-Ins/CLAP/
# -----------------------------------------------------------------------------
log install "copying to $INSTALLED_BUNDLE ..."
rm -rf "$INSTALLED_BUNDLE"
ditto "$BUILT_BUNDLE" "$INSTALLED_BUNDLE"

# Locally built and unsigned; strip any quarantine attribute so the host loads it.
log install "stripping quarantine xattr ..."
xattr -cr "$INSTALLED_BUNDLE" 2>/dev/null || true

elapsed=$(( SECONDS - start ))
echo
log done "installed -> $INSTALLED_BUNDLE  (${elapsed}s)"
echo "Restart or rescan your host to load it."
