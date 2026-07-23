#!/usr/bin/env bash
# One-time setup for the StepForge Rust engine.
# Ensures a rustup-managed stable toolchain + iOS targets + cbindgen.
# (iOS targets are also declared in rust-toolchain.toml; the explicit add below
#  is belt-and-suspenders for environments where Homebrew rust shadows rustup.)
set -euo pipefail

# Ensure rustup-managed cargo/cbindgen are found even when Homebrew rust shadows rustup
# or the script runs under Xcode's minimal preBuildScript PATH.
export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v rustup >/dev/null 2>&1; then
  echo "ERROR: rustup is required (Homebrew rust cannot cross-compile to iOS)." >&2
  echo "Install from https://rustup.rs and put ~/.cargo/bin before Homebrew on PATH." >&2
  exit 1
fi

echo "[setup] rustup default stable ..."
rustup default stable >/dev/null

echo "[setup] adding iOS and macOS targets ..."
rustup target add --toolchain stable \
  aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios \
  aarch64-apple-darwin x86_64-apple-darwin

echo "[setup] installing cbindgen (if missing) ..."
if ! command -v cbindgen >/dev/null 2>&1; then
  cargo install cbindgen --locked
fi

echo "[setup] verifying targets ..."
for t in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target list --installed | grep -q "$t" || { echo "ERROR: target $t missing" >&2; exit 1; }
done

echo "[setup] done."
