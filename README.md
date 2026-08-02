# StepForge

StepForge is a MIDI step sequencer built on a hard two-layer boundary: a Rust
musical-time core (`sequencer_engine`) wrapped by a SwiftUI shell. It ships as a
standalone app on **iOS and macOS**, and as **AUv3** and **CLAP** plugin editions on
macOS; a **VST3** edition is in design. AUv3 crosses the byte-FFI seam (host-driven
via `engine_render`); CLAP is a separate pure-Rust surface that consumes the core
in-process — no Swift, no C ABI.

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
| AUv3 (macOS) | Shipped — `StepForgeAU` app-extension, host-driven MIDI effect |
| CLAP (macOS) | Shipped — pure-Rust `nih-plug` + `egui` plugin |
| VST3 (macOS) | In design — not yet shipped |

> **Sync note:** MIDI Clock works on iOS and macOS. Ableton Link is real on macOS and
> intentionally dormant on iOS today (the Link runtime is `cfg(not(target_os = "ios"))`).
> AUv3 is macOS-only: the iOS app target excludes the `AudioUnit/` sources.

## Architecture

- **Two-layer split.** `sequencer_engine` is pure Rust and `#![forbid(unsafe_code)]`;
  `sequencer_engine_ffi` is the *only* crate with `unsafe` (the 8 `extern "C"` entry points
  + CoreMIDI bindings). The unsafe boundary is compiler-enforced.
- **Two plugin surfaces, one core.** The AUv3 edition reuses the SwiftUI editor inside
  an `AudioUnit` app-extension and crosses the byte-FFI seam host-driven via
  `engine_render`. The CLAP edition is a separate, pure-Rust surface: `crates/editor_egui`
  (`stepforge_editor_egui`) is the testable egui editor UI (no `nih_plug` dep), and
  `crates/clap_plugin` (`stepforge_clap`, `nih_plug` + `nih_plug_egui`) wraps it and calls
  `core` directly in-process — no Swift, no C ABI.
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
  crates/core/         # sequencer_engine     — musical-time logic, state, models (#![forbid(unsafe_code)])
  crates/ffi/          # sequencer_engine_ffi — only `unsafe`: C ABI + CoreMIDI
  crates/editor_egui/  # stepforge_editor_egui — pure-egui editor UI (no nih_plug dep), host-free tests
  crates/clap_plugin/  # stepforge_clap        — nih_plug + nih_plug_egui wrapper, calls core in-process
  crates/xtask/        # nih_plug_xtask bundler -> engine/target/bundled/stepforge_clap.clap
  include/             # sequencer_engine.h — committed, single source of truth for Swift
  scripts/             # setup.sh, build_engine.sh
  dist/                # SequencerEngine.xcframework (built, gitignored)
app/
  StepForge/           # SwiftUI shell: Engine/, Features/, Components/, Theme/, Gestures/, Persistence/
    AudioUnit/         # AUv3 app-extension glue (StepForgeAU, macOS-only; excluded from the iOS target)
  project.yml          # XcodeGen spec: StepForge (iOS), StepForge-macOS, StepForgeAU appex, StepForgeTests
docs/                  # specs/, plans/, superpowers/{specs,plans,audits}/
build_install_macos.sh # clean build + install the macOS app into ~/Applications
```

## Status & roadmap

**Shipped**

- iOS + macOS standalone apps (shared SwiftUI + `EngineBridge` tree).
- Full musical-time core with property-tested step algorithms and versioned persistence.
- **Host-driven rendering** — `engine_render` advances the engine one host audio block on
  the RT thread (C-ABI export + `HostRenderState` live in `engine/include/sequencer_engine.h`).
- **AUv3** (macOS) — `StepForgeAU` app-extension, host-driven MIDI effect.
- **CLAP** (macOS) — pure-Rust `nih-plug` + `egui` plugin; editing view + transport ported.
- MIDI Clock sync (iOS + macOS) and Ableton Link (macOS).

**In progress / designed**

- **VST3 edition** (macOS) — designed, not yet shipped.
- **CLAP feature parity** with the Swift app — editor surface is landing incrementally.

**Known limits**

- Ableton Link is dormant on iOS.
- AUv3 is macOS-only (the iOS app target excludes `AudioUnit/`).
- No CI and no signed release builds.

## Getting started

StepForge ships as two independent surfaces that build separately. Pick one:

- **Swift app + AUv3 (iOS + macOS)** — SwiftUI/Rust standalone apps plus the
  AUv3 host-driven MIDI effect. Needs Xcode + XcodeGen + a rustup toolchain with
  iOS targets. See [Swift app + AUv3](#swift-app--auv3-ios--macos) below.
- **CLAP plugin (macOS)** — pure-Rust `nih-plug` + `egui`, no Swift, no C ABI.
  Needs only a rustup toolchain. See [CLAP plugin](#clap-plugin-macos) below.

### Swift app + AUv3 (iOS + macOS)

One-time setup (rustup-managed toolchain required — Homebrew rust can't
cross-compile to iOS — plus Xcode + Command Line Tools):

```bash
brew install xcodegen                 # generates the Xcode project from app/project.yml
engine/scripts/setup.sh               # rustup stable + iOS targets + cbindgen
```

Build the engine (the app's prebuild script runs this too):

```bash
engine/scripts/build_engine.sh        # -> engine/dist/SequencerEngine.xcframework
                                      #    engine/include/sequencer_engine.h
```

Generate the (gitignored) Xcode project and build a target:

```bash
cd app && xcodegen generate           # -> app/StepForge.xcodeproj

# iOS (simulator)
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge \
  -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build

# macOS
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge-macOS \
  -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO build
```

Install the standalone macOS app into `~/Applications` (clean build + ad-hoc sign
+ quarantine strip):

```bash
./build_install_macos.sh              # SKIP_RUST_CLEAN=1 keeps engine/target for fast Swift-only cycles
```

> **AUv3:** `StepForgeAU` is an app-extension built automatically inside the
> macOS target — no separate build or install command. The host registers it
> when the macOS app launches. AUv3 is macOS-only (the iOS target excludes
> `AudioUnit/`).

### CLAP plugin (macOS)

Pure Rust — no Xcode, xcodegen, cbindgen, or iOS targets. One-time setup is just
the rustup toolchain + macOS targets:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

Build and test the editor UI, then bundle the plugin:

```bash
cd engine
cargo test -p stepforge_editor_egui                 # editor UI tests (pure-egui, host-free)
cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings
cargo xtask bundle -p stepforge_clap --release      # -> engine/target/bundled/stepforge_clap.clap
```

Install it into `~/Library/Audio/Plug-Ins/CLAP/` (bundles, copies, strips
quarantine):

```bash
engine/scripts/install_clap.sh        # -> ~/Library/Audio/Plug-Ins/CLAP/stepforge_clap.clap
```

Restart or rescan your host to load it.

## Tests

```bash
cd engine
cargo test                                            # core + FFI (incl. C-ABI garbage-bytes safety)
cargo test -p sequencer_engine_ffi --test ffi_api     # the FFI integration test
cargo test -p stepforge_editor_egui                   # the CLAP editor UI (pure-egui, host-free)
cargo fmt && cargo clippy --all-targets -- -D warnings
```

The step algorithms (Roll/Vary/…) are covered by `proptest` invariants, and sessions
round-trip through the versioned snapshot format.

## Documentation

- [`docs/specs/`](docs/specs/) — governing architecture and UI/UX specs, plus
  [`amendments.md`](docs/specs/amendments.md) (resolved contradictions + open issues).
- [`docs/superpowers/specs/`](docs/superpowers/specs/) — design docs (foundation,
  engine-plan; macOS-target and plugin-port designs land with their feature branches).
- [`SPEC.md`](SPEC.md) — the spec-driven-development spec for the in-progress work.
- [`CLAUDE.md`](CLAUDE.md) — the engineering guide for contributors.

## Contributing

Contributions follow the working agreement in [`CLAUDE.md`](CLAUDE.md): cross-layer changes
are symmetric — add the Rust variant + codec + a C-ABI round-trip test, then the Swift
mirror + `SessionMirror.apply` (no orphans). The RT path stays allocation-free, and
algorithm changes get `proptest`.

## License

Dual-licensed under **MIT or Apache-2.0**, at your option (SPDX: `MIT OR Apache-2.0`). See
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).
