---
description: Diagnose StepForge xcframework / cargo / cbindgen / Xcode build-chain failures
---

Diagnose the StepForge build failure. **$ARGUMENTS** is the failing command or the pasted error text. Walk the chain, find the root cause, and give the minimal fix.

- **Cargo** (`engine/`): check `Cargo.toml` workspace members and `crates/*/Cargo.toml` deps + feature flags (some crates need `no_std`/iOS-friendly features); confirm the target triple (`aarch64-apple-ios` / `aarch64-apple-ios-sim` / `x86_64-apple-ios`) is installed. If `rustc --version` prints `(Homebrew)`, that is the cause — run `rustup default stable`, put `~/.cargo/bin` before Homebrew on `PATH`, and rely on `rust-toolchain.toml`.
- **cbindgen** (`build_engine.sh` step): check `cbindgen.toml` (`language = "c"`, `parse_deps`), that `sequencer_engine_ffi` actually has the `#[no_mangle] extern "C"` symbols, and that the generated `engine/include/sequencer_engine.h` declares the symbols Swift uses.
- **xcframework** (`build_engine.sh`): confirm all three `.a` slices exist at `target/<triple>/release/libsequencer_engine_ffi.a` and the `-headers` dir contains `sequencer_engine.h`; recreate with `rm -rf engine/dist/SequencerEngine.xcframework` then re-run.
- **Xcode** (`app/`): `FRAMEWORK_SEARCH_PATHS` includes `$(SRCROOT)/../engine/dist`; the xcframework is **linked, not embedded** (`embed: false` in `app/project.yml`); `SWIFT_OBJC_BRIDGING_HEADER` points at `$(SRCROOT)/../engine/include/sequencer_engine.h`; `CoreMIDI.framework` is linked; if the project is stale, regenerate with `cd app && xcodegen generate`.

Inspect the relevant files (`Cargo.toml`, `cbindgen.toml`, `build_engine.sh`, `app/project.yml`, the header) to confirm. State the single most likely root cause first, then secondary checks. Apply the fix if asked.
