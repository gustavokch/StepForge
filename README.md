# StepForge

StepForge is a MIDI step sequencer built on a hard two-layer boundary: a Rust
musical-time core (`sequencer_engine`) wrapped by a SwiftUI shell. It ships as a
standalone app on **iOS and macOS**; an audio-plugin edition
(**AUv3 / VST3 / CLAP**) is in design.

## What it is

A real-time, gesture-driven drum sequencer. The Rust core owns everything musical —
timing, transport, pattern state, MIDI dispatch — and the SwiftUI shell owns everything
non-musical: UI, gestures, CoreMIDI discovery, haptics, and persistence. The two layers
talk only through a byte-serialized command/event channel across a C ABI. No data-carrying
struct crosses that boundary.

## Features

- **Musical-time core** — `Session` / `Pattern` / `Track` / `Step` model with per-track
  swing, velocity humanize, ratchets, velocity zones, a pattern queue with follow-actions,
  and quantize grains.
- **Step algorithms** — Roll, Vary, Cut, Trash, Paste, covered by `proptest` invariants
  (`length`, `midi_note`, `speed_ratio` are preserved; accents stay locked for Vary).
- **MIDI I/O** — CoreMIDI destination discovery and note output.
- **Sync** — inbound MIDI Clock (24-PPQN) on both platforms; Ableton Link session sync on
  macOS (see [Platforms](#platforms)).
- **Persistence** — versioned session save/load (`SessionEnvelope` /
  `SESSION_FORMAT_VERSION`) with a snapshot round-trip test.
- **Experience** — multi-touch step gestures, haptics (iOS), and adapted iPad + Mac layouts.

## Platforms

| Surface | Status |
| --- | --- |
| iOS 17 (iPhone + iPad) | Standalone app |
| macOS 14 | Standalone app (`StepForge-macOS`) |
| AUv3 / VST3 / CLAP (macOS) | In design — not yet shipped |

> **Sync note:** MIDI Clock works on iOS and macOS. Ableton Link is real on macOS and
> intentionally dormant on iOS today (the Link runtime is `cfg(not(target_os = "ios"))`).

## Architecture

- **Two-layer split.** `sequencer_engine` is pure Rust and `#![forbid(unsafe_code)]`;
  `sequencer_engine_ffi` is the *only* crate with `unsafe` (the 8 `extern "C"` entry points
  + CoreMIDI bindings). The unsafe boundary is compiler-enforced.
- **The FFI seam is bytes, not structs.** Commands and events cross the C ABI as
  postcard-serialized bytes via total codecs (never panic). Every `extern "C"` body is
  wrapped in `catch_unwind` and returns a `#[repr(C)] EngineResult` — malformed bytes yield
  `ErrDecode`, never an abort.
- **The mirror pattern.** Swift never holds a pointer into engine memory. `EngineBridge`
  drains events into a value-type `SessionMirror` on the MainActor; SwiftUI reads only the
  mirror. Gestures become commands; the UI never mutates engine state directly.
- **The RT thread is sacred.** Self-scheduled from a Rust clock, it never crosses FFI,
  never locks, never allocates on the hot path (fixed-slot channels; bounded queues drop on
  overflow). `MIDISend` runs only on the off-RT CoreMIDI worker, never on RT.

The seven hard rules and the full working agreement live in [`CLAUDE.md`](CLAUDE.md); see
[`docs/specs/architecture-spec.md`](docs/specs/architecture-spec.md) for the detailed
architecture.

## Repository layout

```
engine/
  crates/core/    # sequencer_engine  — musical-time logic, state, models (#![forbid(unsafe_code)])
  crates/ffi/     # sequencer_engine_ffi — the only `unsafe`: C ABI + CoreMIDI
  include/        # sequencer_engine.h — committed, single source of truth for Swift
  scripts/        # setup.sh, build_engine.sh
app/
  StepForge/      # SwiftUI shell: Engine/, Features/, Components/, Theme/, Gestures/, Persistence/
  project.yml     # XcodeGen spec (iOS + macOS targets)
docs/             # specs/, plans/, superpowers/{specs,plans,audits}/
```

## Status & roadmap

**Shipped**

- iOS + macOS standalone apps (shared SwiftUI + `EngineBridge` tree).
- Full musical-time core with property-tested step algorithms and versioned persistence.
- MIDI Clock sync (iOS + macOS) and Ableton Link (macOS).

**In progress / designed**

- **Host-driven rendering.** `Engine::render_host` and the `HostRenderState` types have
  landed in the core; the C-ABI `engine_render` export and host drivers are pending.
- **Plugin edition — AUv3 + VST3 + CLAP** (macOS-only, host-driven MIDI effect). Designed,
  Phase 0 in progress; **not yet shipped.**

**Known limits**

- Ableton Link is dormant on iOS.
- No CI and no signed release builds.

## Getting started

One-time setup (requires a rustup-managed toolchain — Homebrew rust cannot cross-compile to
iOS — plus Xcode and the Command Line Tools):

```bash
brew install xcodegen                 # generates the Xcode project from app/project.yml
engine/scripts/setup.sh               # rustup stable + iOS targets + cbindgen
```

Build the engine (the app's prebuild script runs this too):

```bash
engine/scripts/build_engine.sh        # -> engine/dist/SequencerEngine.xcframework
                                      #    engine/include/sequencer_engine.h
```

Generate the (gitignored) Xcode project and build either target:

```bash
cd app && xcodegen generate           # -> app/StepForge.xcodeproj

# iOS (simulator)
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge \
  -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build

# macOS
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge-macOS \
  -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO build
```

## Tests

```bash
cd engine
cargo test                                            # core + FFI (incl. C-ABI garbage-bytes safety)
cargo test -p sequencer_engine_ffi --test ffi_api     # the FFI integration test
cargo fmt && cargo clippy --all-targets -- -D warnings
```

The step algorithms (Roll/Vary/…) are covered by `proptest` invariants, and sessions
round-trip through the versioned snapshot format.

## Documentation

- [`docs/specs/`](docs/specs/) — governing architecture and UI/UX specs, plus
  [`amendments.md`](docs/specs/amendments.md) (resolved contradictions + open issues).
- [`docs/superpowers/specs/`](docs/superpowers/specs/) — design docs (foundation,
  engine-plan; macOS-target and plugin-port designs land with their feature branches).
- [`CLAUDE.md`](CLAUDE.md) — the engineering guide for contributors.

## Contributing

Contributions follow the working agreement in [`CLAUDE.md`](CLAUDE.md): cross-layer changes
are symmetric — add the Rust variant + codec + a C-ABI round-trip test, then the Swift
mirror + `SessionMirror.apply` (no orphans). The RT path stays allocation-free, and
algorithm changes get `proptest`.

## License

Dual-licensed under **MIT or Apache-2.0**, at your option (SPDX: `MIT OR Apache-2.0`). See
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).
