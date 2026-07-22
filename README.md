# StepForge

iOS MIDI drum sequencer: a Rust musical-time core (`sequencer_engine`) wrapped by
a SwiftUI shell (`StepForge`). See `docs/specs/` for the full specs and
`docs/specs/amendments.md` for resolved/open issues.

## One-time setup

```bash
brew install xcodegen                 # generates the Xcode project from app/project.yml
engine/scripts/setup.sh               # rustup stable + iOS targets + cbindgen
```

Requires a rustup-managed toolchain (Homebrew rust cannot cross-compile to iOS),
Xcode, and the Command Line Tools.

## Build & run

```bash
engine/scripts/build_engine.sh        # -> engine/dist/SequencerEngine.xcframework + engine/include/sequencer_engine.h
cd app && xcodegen generate           # -> app/StepForge.xcodeproj (gitignored)
open app/StepForge.xcodeproj          # Run on a simulator/device
```

## Tests

```bash
cd engine && cargo test               # core + FFI (incl. C-ABI garbage-bytes safety)
```
