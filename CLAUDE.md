# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## StepForge

iOS MIDI drum sequencer with a hard two-layer boundary: a **Rust musical-time core** (`sequencer_engine`) compiled to a static `.a` / xcframework, and a **SwiftUI shell** (`StepForge`) that owns everything non-musical (UI, gestures, Ableton Link, CoreMIDI discovery, haptics, persistence). The two layers communicate only through a byte-serialized command/event channel across a C ABI.

- Source of truth: `docs/specs/ui-ux-spec.md` + `docs/specs/architecture-spec.md`; resolved contradictions + open issues in `docs/specs/amendments.md`.
- Foundation design: `docs/superpowers/specs/2026-07-22-project-foundation-design.md`. Implementation plan: `docs/plans/2026-07-22-project-foundation.md`.
- Status: the Rust workspace and app are scaffolded by the plan; the commands below apply once scaffolded.

## Commands

One-time setup (requires a rustup-managed toolchain — Homebrew `rustc` cannot cross-compile to iOS — plus Xcode + Command Line Tools):

```bash
brew install xcodegen
engine/scripts/setup.sh        # rustup stable + iOS targets + cbindgen
```

Engine (Rust) — run from `engine/`:

```bash
cargo check                                          # typecheck both crates
cargo test                                           # all Rust tests (core + FFI, incl. C-ABI safety)
cargo test -p sequencer_engine_ffi --test ffi_api    # one integration test file
cargo test commands_roundtrip                        # filter tests by name
cargo fmt                                            # format
cargo clippy --all-targets -- -D warnings            # lint
```

Build the xcframework + C header (the app's preBuildScript runs this too):

```bash
engine/scripts/build_engine.sh
# -> engine/dist/SequencerEngine.xcframework
# -> engine/include/sequencer_engine.h (committed; single source of truth for Swift)
```

App (Swift):

```bash
cd app && xcodegen generate      # regenerate StepForge.xcodeproj from project.yml (gitignored)
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge \
  -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build
```

The xcframework is **linked** (not embedded) into the app; `CoreMIDI.framework` is also linked. The Swift bridging header points directly at `engine/include/sequencer_engine.h`.

## Architecture

**Workspace split (`engine/`).** `crates/core` (`sequencer_engine`, `#![forbid(unsafe_code)]`) holds all musical-time logic, state, and models — pure Rust, host-testable, no platform I/O. `crates/ffi` (`sequencer_engine_ffi`, `#![allow(unsafe_code)]`) is the *only* crate with `unsafe`: the 8 `extern "C"` entry points + CoreMIDI bindings. The unsafe boundary is compiler-enforced, not conventional. `core/src/midi.rs` is pure dispatch math; all CoreMIDI `unsafe` lives in `ffi/src/coremidi.rs`.

**The FFI seam is bytes, not structs.** Commands (Swift → Rust) and events (Rust → Swift) cross the C ABI as postcard-serialized bytes via `command_codec` / `event_codec`; both sides have matching encoders/decoders. No data-carrying `#[repr(C)]` enum is ever passed across. `engine_submit_command(ptr, len)` enqueues to a lock-free **MPSC** queue; `engine_drain_events` pulls. Every `extern "C"` body is wrapped in `catch_unwind` and returns a `#[repr(C)] EngineResult` (malformed bytes → `ErrDecode`, never an abort). Rust-allocated buffers are freed only by `engine_free_bytes`.

**Threading.** The engine's real-time thread is self-scheduled from a Rust clock, advances step dispatch, and hands MIDI off via a fixed-slot ring to a CoreMIDI worker thread (`MIDISend` never runs on RT). RT emits small events on a fixed-slot channel (`[u8; MAX_EVENT_BYTES]`, encoded via `encode_event_into` into a caller-provided buffer — never `Vec`); large payloads (`Serialized`/`Error`) are produced on an off-RT worker. Swift drains both channels on a dedicated `DispatchQueue` (~120 Hz), coalesces playhead events, and makes one MainActor hop per batch.

**The mirror pattern.** Swift never holds a pointer into engine memory. `EngineBridge` drains events into a value-type `SessionMirror` on the MainActor; SwiftUI reads only the mirror. Gestures become commands; the UI never mutates engine state directly.

## Hard rules (do not violate)

1. **RT thread is sacred** — never crosses FFI, never calls Swift, never locks, never allocates on the hot path. Self-scheduled from a Rust clock (external Link / MIDI-Clock timing arrives as commands). Event and MIDI channels use fixed-size slots. Bounded queues drop on overflow (never block). Workers reading RT-mutated state use non-blocking reads (seqlock/COW), never a mutex the RT thread could block on.
2. **UI holds no long-lived pointer into engine state** — only the value-type `SessionMirror` on MainActor. Transient buffers from `drain`/`serialize` are borrowed and freed via `engine_free_bytes` immediately.
3. **All FFI is non-blocking and panic-safe** — `catch_unwind` + `EngineResult`; codecs are total (never panic); no push callback from the engine into Swift.
4. **Buffer ownership** — Rust-allocated buffers are freed only by `engine_free_bytes`, exactly once; command bytes are Swift-owned and borrowed by Rust for the call only.
5. **Handle lifecycle** — `engine_stop` returns before `engine_free`; no concurrent `engine_*` calls on a handle; `EngineBridge` stops from scene-phase teardown before `deinit` frees.
6. **Unsafe isolation** — `sequencer_engine` stays `#![forbid(unsafe_code)]`; all `unsafe` in `sequencer_engine_ffi`, reviewed line-by-line.
7. **Swift owns the CoreMIDI lifecycle** — `MIDIClientRef` / endpoint discovery lives in Swift; the engine stores only integer endpoint IDs.

## Working agreement

- **Cross-layer changes are symmetric — no orphans.** Adding a `Command` / `EngineEvent`: add the Rust variant + codec + a C-ABI round-trip test, then the Swift mirror + encoder/decoder + `SessionMirror.apply`. (Use `/add-feature`.)
- **The RT path stays allocation-free** — fixed buffers/slots; never `Vec` / `String` / `format!` on RT. Audit RT files before merging. (Use `/audit-rt`.)
- **Algorithm changes get property tests** — Roll/Vary/etc. must preserve invariants (`length` / `midi_note` / `speed_ratio` unchanged; accents locked for Vary). Use `proptest`.
- **Model changes ripple** — update `serde_ext` (+ bump `SESSION_FORMAT_VERSION`), the Swift mirror, and the snapshot round-trip test.
- **Non-destructive length** — `Track.length` is a window over a fixed `[Step; 16]`; Roll/Vary/Cut/Trash never touch `length` / `midi_note` / `speed_ratio`; Paste carries `length` + `speed_ratio` but never `midi_note`.

## Where things live

Specs `docs/specs/` · amendments `docs/specs/amendments.md` · design `docs/superpowers/specs/` · plans `docs/plans/` · engine `engine/crates/{core,ffi}` · app `app/StepForge/`.
