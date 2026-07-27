# Phase 1 AUv3 — Validation Report

**Branch:** `feat/auv3-port-impl` (off `origin/main`, with the Phase-1 design + plan cherry-picked)
**Date:** 2026-07-27
**Plan:** `docs/superpowers/plans/2026-07-26-plugin-port-phase1-auv3.md`

## Validation bar — met

| Check | Result |
|---|---|
| `/audit-rt` (RT path) | ✅ Clean — Phase 1 touched no `engine/` files; `core/src/lib.rs` still `#![forbid(unsafe_code)]`; the Rust RT files (`clock.rs`/`scheduler.rs`/`midi.rs`/`event_codec.rs`) are allocation/lock-free; the Swift `internalRenderBlock` was audited line-by-line (Hard Rule 1) during the Task 5 review (no alloc/lock/FFI-other-than-`engine_render`/CoreMIDI/Link). |
| `auval -v aumi DrmS SFor` | ✅ `AU VALIDATION SUCCEEDED` (exit 0), reproducible. Component discovered via `auval -l`. |
| Standalone iOS — build + tests | ✅ `BUILD SUCCEEDED`; `TEST SUCCEEDED`, 48 tests (44 prior + 4 new Phase-1 state tests). |
| Standalone macOS — build | ✅ `BUILD SUCCEEDED`; `StepForgeAU.appex` embedded in `StepForge-macOS.app/Contents/PlugIns/`. |
| Engine regression (no Rust changes) | ✅ `cargo test` green; `cargo clippy --all-targets -- -D warnings` clean; `cargo check --target aarch64-apple-ios` green. |

## Deferred (host-validatable only)

- **Host smoke matrix (Logic / Reaper / AUM):** MIDI-in-time at 120 + 140 BPM, play/stop/seek/loop re-align to the bar, editor opens + edits apply, project save/restore persists the session. `internalRenderBlock` cannot be exercised end-to-end without a host; record pass/fail per host when run manually.
- **Notarization / distribution signing:** post-Phase-1.

## AU-glue reconciliations made during implementation

The plan flagged the AU-SDK call sites as "VERIFY / reconcile against the compiler + auval." Resolutions actually applied (verified against `MacOSX.sdk` AudioToolbox headers, the Swift compiler, and `auval`):

**Task 4 (AU shell):** `AudioComponents` codes as 4-char ASCII strings (not `0x`-hex); `AUInternalRenderBlock` 7-param arity (added `outputData`); silence via `noErr` (`mutableRawAudioData` is not a public accessor); `factoryFunction` removed for pure-v3; `NSExtensionPrincipalClass = $(PRODUCT_MODULE_NAME).StepForgeEditorViewController`; App Sandbox mandatory (entitlements); `outputBusses` override + bus built in `init` (else `-10875`); `SettingsSheet` excluded from the AU target (references `MidiManager`).

**Task 5 (engine_render wiring):** classic MIDI end-to-end (NOT UMP) — input walks `AURenderEvent.MIDI.data[3]`; output via `MIDIOutputEventBlock` (`AUMIDIOutputEventListBlock` does not exist); `AUHostMusicalContextBlock` 6 args (downbeat = param 6 `currentMeasureDownbeatPosition`); `AUHostTransportStateBlock` 4 args; `isPlaying = flags.contains(.moving)` (no `.playing`); `renderState` as `UnsafeMutablePointer<RenderStateHandle>?`; Swift-renamed `midiOutputEventBlock`/`midiOutputNames`; cached render block; allocation-free `midiIn` walk (directly into the fixed buffer — no `[RawMIDI]` on the RT thread); stack-tuple `midiOut` emit.

**Task 6 (editor):** `AppMode` extracted to a shared file (the AU target excludes `Root/`); `bridgeForEditor` is `internal` (cross-file); transport swapped via an additive `EnvironmentValues.usePluginTransport` flag (no duplicate bar); `SettingsSheet` dropped from the plugin editor (not in the AU target).

**Task 7 (state persistence):** `AUState` in its own iOS-safe file (not inside the macOS-guarded `StepForgeAudioUnit.swift`); `bridge.load(recovered!)` (brief had `Data?` vs `Data` type error).

## Bug found + fixed during validation (Task 8)

`auval -v` initially FAILED at `VERIFYING CLASS INFO` with `kAudioUnitErr_InvalidPropertyValue` (-10851). Root cause: Task 7's `fullState` *replaced* `super.fullState` with a bespoke `["session": Data]` dict, stripping the standard ClassInfo keys (`<type>`/`<subtype>`/`<manufacturer>`, version, parameter tree) the v2 bridge needs to synthesize `kAudioUnitProperty_ClassInfo`. Fix (`9a1b4fe`): `get` starts from `super.fullState` and adds the session; `set` pulls `session` out for the engine and forwards the rest to `super`. Persistence round-trip unchanged; auval now passes. (Confirmed via controlled experiments: `[:]` → "missing required field `<type>`"; `super.fullState` alone → pass; merge → pass.)

## Minor hardening items (deferred to a follow-up; non-blocking)

Recorded in `.superpowers/sdd/progress.md`. Notable: Task 5 RT-path cosmetics (clamp `UInt32` sample-offset; cache `sampleRate` in the render block; clamp `outCount` to `outCap`); Task 6 redundant env-flag injection in the VC + `bridgeForEditor` defensive fallback; Task 7 round-trip test loads identical bytes (partially vacuous). The Task 1 dead `stolenByDeinit` test scaffolding is plan-mandated verbatim.
