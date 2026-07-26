# StepForge Plugin Edition — Design (AUv3 + VST3 + CLAP, macOS)

> Approved design for porting StepForge from a standalone app into DAW plugins.
> Cross-platform (Windows/Linux) is **deferred** — tracked in a separate repo
> issue. Implementation is phased (Phase 0 → 4); see "Phased implementation".

## Context

StepForge is a MIDI drum sequencer with a clean two-layer split: a Rust
musical-time core (`sequencer_engine`) behind a byte-serialized C ABI, and a
SwiftUI shell. Today it ships as a standalone app (iOS + a `StepForge-macOS`
target already exists and builds). The goal: turn it into a **plugin** that
runs inside DAWs (Logic, Reaper, Bitwig, Cubase, Ableton Live, AUM), targeting
**AUv3 + VST3 + CLAP on macOS**, with **maximal reuse** of the existing Rust
core and SwiftUI editor.

Exploration established three load-bearing facts that shape this design:

1. **MIDI-only.** The engine has no audio path — it emits MIDI. So the plugin is
   a **MIDI source / MIDI-FX** (AU `'aumi'`, CLAP note-output, VST3 MIDI
   effect), never an instrument.
2. **The dispatch core is already host-friendly.** `process()`
   (`engine/crates/core/src/engine.rs:1067`) is a pure, allocation-free free
   function that advances exactly one 16th-note step per call, and a
   `SteppableClock::advance_to()` already exists
   (`engine/crates/core/src/clock.rs:12-34`). The self-scheduled RT loop is a
   thin wrapper around it — so re-hosting behind a host render callback is
   feasible, not a rewrite.
3. **The Swift layer is already multiplatform.** `EngineBridge`,
   `SessionMirror`, the postcard codecs, `Models`, and the entire SwiftUI view
   tree already compile for macOS. The throwaway surface is ~3 small files.

**Decisions locked in brainstorming:**
- **Scope:** AU + VST3 + CLAP, **macOS only**. Cross-platform deferred (repo issue).
- **Transport:** **Host-driven from day one** — the host's render callback
  drives the engine; sample-accurate; follows host transport. Ableton Link
  disabled (the host *is* the transport).
- **I/O:** **MIDI generator + MIDI input** (incoming notes select patterns /
  audition; merged to output). No audio path.
- **Wrapper tech:** **Pure Rust + Swift, no C++.** AUv3 = Swift app extension;
  VST3 + CLAP = nih_plug (Rust). The SwiftUI editor is reused identically by all
  three via a Swift bundle exposing an `NSView` factory.
- **VST3 licensing:** build **CLAP native in nih_plug**, produce **VST3 via
  clap-wrapper** — avoids the GPLv3 `vst3-sys` contamination that would
  otherwise conflict with StepForge's MIT/Apache-2.0 license.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│ DAW host  (Logic / Reaper / Bitwig / Cubase / Live / AUM)     │
│ gives: transport (tempo·position·play), MIDI in/out, render   │
│ callback at sample rate, project state save/restore           │
└──────────────────────┬───────────────────────────────────────┘
                       │ host SDK (AUv3 / VST3 / CLAP)
        ┌──────────────┼──────────────┬─────────────────┐
        ▼              ▼              ▼
   ┌─────────┐   ┌───────────┐   ┌────────────┐
   │  AUv3   │   │   VST3    │   │    CLAP    │   ← 3 thin wrappers
   │ .appex  │   │ clap-wrap │   │  nih_plug  │     (Swift / Rust)
   │ Swift   │   │  per of ↓ │   │   Rust     │
   └────┬────┘   └─────┬─────┘   └─────┬──────┘
        │              │               │
        └──────────────┴───────────────┤  each calls / hosts:
                                        ▼
        ┌───────────────────────────────────────────────┐
        │  Rust host-adapter  (NEW)                      │
        │  engine_render(): transport→step, sample-offset│
        │  MIDI out, incoming-MIDI→commands              │
        │  ┌─────────────────────────────────────────┐   │
        │  │ sequencer_engine core  (REUSE ~unchanged)│  │
        │  │ process(), models, scheduler, codecs     │  │
        │  └─────────────────────────────────────────┘   │
        └───────────────────────▲───────────────────────┘
                                │ stepforge_create_editor_view(handle)->NSView
        ┌───────────────────────┴───────────────────────┐
        │  Swift editor bundle  (NEW shell, ~95% reused) │
        │  SwiftUI views + EngineBridge + SessionMirror  │
        └───────────────────────────────────────────────┘
```

Three layers, each with one clear job, each independently testable:

- **Rust host-adapter** — the only substantial new Rust. Drives the reused core
  from a host render callback; no threads of its own on the RT path.
- **Swift editor bundle** — the reused SwiftUI editor + `EngineBridge`, exposed
  as an `NSView` factory so any wrapper can embed it.
- **Three thin wrappers** — each implements one host SDK, calls
  `engine_render`, routes MIDI via host note ports, embeds the editor `NSView`,
  persists state with the project.

## The Rust host-adapter (the heart of the new work)

**Problem.** Today `engine_start` (`engine/crates/ffi/src/lib.rs:142-172`)
spawns four threads: an RT thread self-scheduled by `InstantClock`
(`engine/crates/ffi/src/coremidi.rs:405-454`), a state worker, a CoreMIDI
worker, and a Link poller. A plugin host drives its **own** RT thread and
expects the plugin to render inside its `process()` callback — so the
self-scheduled RT thread, the CoreMIDI worker (which `MIDISend`s against
wall-clock `Instant`), and the Link poller must all be removed in plugin mode.

**Solution: a host-driven mode, selected at construction.** Construct the
engine *without* spawning the RT/CoreMIDI/Link threads; the host calls a new
render entry per audio block. The state-worker thread is **kept** — it drains
the command MPSC and writes the COW `Session` snapshot off-RT, exactly as today,
so the render path only does a non-blocking seqlock/COW read (Hard Rule 1
preserved). A cargo feature (e.g. `host-driven`) or a runtime construction flag
gates the two modes; the standalone app keeps the self-scheduled mode unchanged.

**New C ABI entry** (alongside the existing 8; in
`engine/crates/ffi/src/lib.rs`, mirrored in
`engine/include/sequencer_engine.h`):

```c
// Advance the engine by one host audio block, on the host's RT thread.
EngineResult engine_render(
    struct EngineHandle *engine,
    const HostTransport *transport,            // tempo, is_playing, sample_rate,
                                               // block_samples, block_start_beat,
                                               // beats_per_bar, bar_start_beat
    const IncomingMidi *midi_in, uintptr_t midi_in_count,   // host MIDI in, sample-offset
    OutgoingMidi *midi_out, uintptr_t midi_out_cap,         // caller buffer (fixed)
    uintptr_t *midi_out_count);                              // written
```

`HostTransport`, `IncomingMidi`, `OutgoingMidi` are plain `#[repr(C)]` POD
structs of scalars (MIDI bytes + sample offsets) — **not** data-carrying enums,
so they obey the FFI rules. Per block, `engine_render`:

1. Reads transport; if `is_playing` transitioned true → seed RNG (`begin_play`,
   `engine.rs:329-339`) and snap `global_step` to the host bar (`bar_start_beat`)
   so step 0 aligns with the downbeat.
2. Computes 16th boundaries crossing the block from `tempo`/`sample_rate`/`beats_per_bar`,
   and for each boundary calls `process_one` (`engine.rs:482-489`) → `process()`
   (`engine.rs:1067`, pure, RT-safe) → `check_scheduler` (`engine.rs:494-585`).
   This is the **sample→step accumulator** (new fractional-16th state in `RtState`).
3. Maps incoming MIDI (`midi_in`) to engine actions — pattern select / audition
   — and/or merges pass-through into the output (see MIDI-in mapping below).
4. Drains `engine.midi` (`MidiOutRing`, `engine/crates/core/src/midi_out.rs:36`,
   lock-free, drop-oldest), assigns each message a **sample offset within this
   block**, writes to `midi_out`. Note-offs whose gate spans past the block end
   are held in a small fixed-size scheduled-events queue keyed by absolute
   sample time and emitted in later blocks.

**`MidiMsg` timing-semantics change.** Today `send_at_offset_micros` /
`gate_micros` are wall-clock `Instant` deltas resolved by the CoreMIDI worker
(`engine/crates/ffi/src/coremidi.rs:201-219`). In host-driven mode these become
**sample offsets**. Field is repurposed (or a `sample_offset` added); the host
thread both produces and drains the ring, so timing is self-consistent.

**RT compliance.** `engine_render` runs on the host RT thread and is audited
with `/audit-rt`: no allocations, no locks, no `Vec`/`String`/`format!`, no FFI
out, no CoreMIDI, no Link. It reuses `process()` (already RT-safe) + the lock-free
ring + a non-blocking snapshot read. New property tests (proptest) assert:
step boundaries fire exactly when host-beat crosses them; every emitted MIDI
sample offset is within `[0, block_samples)`; transport stop freezes advancement
and fires all-notes-off; play-start reseeds deterministically.

**Transport source.** Each wrapper fills `HostTransport` from its host:
- **AUv3:** `musicalContextBlock` (`tempo`, `beat`, `timeSignatureUpper/Lower`,
  `currentDownBeat`, `sampleTime`) + `transportStateBlock` (`transportStateFlags`).
- **VST3/CLAP (nih_plug):** the unified `Transport` struct (`playing`, `tempo`,
  `pos_beats()`, `bar_start_pos_beats()`, `sample_rate`) — nih_plug already
  normalizes VST3 `ProcessContext` and CLAP `clap_event_transport` into it.

## MIDI-input mapping (new engine behavior)

Incoming notes don't exist in the standalone (touch-driven) app, so this is net-new.
A configurable input **mode** (new `Command`/setting), defaulting to:

- **Pattern Select:** Note-Ons in a mapped "command octave" select the active
  pattern / trigger follow-actions (maps to the existing pattern-switch path in
  `engine/crates/core/src/scheduler.rs`).
- **Thru:** notes outside the command range pass through to the output, merged.
- **Off.**

Exact note→pattern mapping is a product detail refined later; the spec fixes the
*mechanism* (incoming MIDI → RT action → output/selection) and a sensible default.

## Swift editor bundle (the reuse vehicle)

Extract the editor + `EngineBridge` + `SessionMirror` + postcard codecs into a
**Swift framework target** that exposes:

```swift
@_cdecl("stepforge_create_editor_view")
func stepforgeCreateEditorView(_ handle: OpaquePointer) -> NSView
```

returning an `NSHostingView` of the editor with a bound `EngineBridge(handle)`.

- **Reused as-is** (`app/StepForge/Engine/` except `EngineLifecycle.swift`,
  `MidiManager.swift`): `EngineBridge`, `SessionMirror`, `Command`,
  `EngineEvent`, `Models`, `Postcard/*`, and the whole `Features/`/`Theme/`/
  `Components/` view tree.
- **Adapted:** `TransportBar` — host drives transport now, so show a
  "Following host: BPM · bar · step" readout instead of play/stop/BPM controls.
  `GridMetrics` — add a pixel-width branch for resizable plugin windows (today
  it keys off compact/regular size classes).
- **Removed in plugin:** destination-selection UI (MIDI routes out the plugin's
  note port, not CoreMIDI destinations); `StepForgeApp.swift`/`RootView.swift`
  app-shell + scene-phase wiring (replaced by the wrapper).

For **AUv3**, the `AUViewController` instantiates the editor directly (same Swift
module — no `dlopen`). For **VST3/CLAP**, the framework is packaged as a dylib
in the `.clap`/`.vst3` `Contents/Frameworks/` and `@rpath`-loaded by the nih_plug
`Editor::spawn`, which receives the host's parent `NSView`
(`ParentWindowHandle::AppKitNsView(*mut c_void)`) and `addSubview:`s the editor
view returned by `stepforge_create_editor_view`.

## State persistence (reuses existing serialize)

`engine_serialize` already yields a versioned `SessionEnvelope` (postcard) with
`SESSION_FORMAT_VERSION`; restore round-trips it. Each wrapper wires this to its
host:

- **AUv3:** `fullState["session"] = <Data>` (postcard bytes); restore on set
  (use `fullStateForDocument` for project-scoped state).
- **VST3/CLAP (nih_plug):** `Plugin` `serialize_state`/`setState` ↔ postcard blob.

## Threading & RT (Hard Rule 1 preserved)

- Host-driven render runs on the **host's RT thread** — no self-scheduled RT
  thread, no CoreMIDI worker, no Link poller in plugin mode.
- `process()` + the MIDI ring + non-blocking snapshot read are RT-safe; the new
  accumulator + scheduled note-off queue are fixed-size, allocation-free.
- `EngineBridge` draining (for the editor `SessionMirror`) still happens off-RT
  on a GUI timer (~120 Hz), unchanged from today.

## Build / packaging

- **AUv3:** add an "Audio Unit Extension" target (`.appex`) to `app/project.yml`
  (xcodegen), embedded in the existing StepForge app as container. Links the
  xcframework + Swift editor sources. Info.plist `NSExtensionPointIdentifier =
  com.apple.AudioUnit-UI`, type code `'aumi'`. **Allocate a dummy 2-channel
  output bus** in `allocateRenderResources()` (some hosts reject a bus-less
  `'aumi'`).
- **CLAP:** new nih_plug plugin crate `engine/crates/clap_plugin`; `cargo xtask
  bundle` → `.clap`. Links the xcframework; `process()` → `engine_render`;
  `NoteEvent` in/out (`MIDI_INPUT`/`MIDI_OUTPUT = Basic`); embeds the editor
  dylib. Pin nih_plug by git SHA (no tagged release).
- **VST3:** produced by **clap-wrapper** wrapping the CLAP build (license-safe).
  If, in future, GPLv3 is acceptable, VST3 can instead be built natively in
  nih_plug and clap-wrapper dropped.
- A build script copies the Swift editor dylib into the `.clap`/`.vst3`
  `Contents/Frameworks/` and sets `@rpath`. Signing + notarization for all three.

## Phased implementation

Each phase is independently shippable.

0. **Rust host-adapter.** Host-driven construction mode; `engine_render` C entry
   + header; transport→step accumulator; sample-offset MIDI out + scheduled
   note-off queue; incoming-MIDI→command mapping; cargo feature disabling the
   self-scheduled RT/CoreMIDI/Link threads. Property tests + `/audit-rt`. *Pure
   Rust, host-testable, no plugin yet.*
1. **AUv3 extension.** Swift `AUAudioUnit` (`'aumi'`), `internalRenderBlock` →
   `engine_render`, MIDI I/O via `*EventList` blocks, musical/transport context,
   `fullState`, `AUViewController` + reused SwiftUI editor, dummy audio bus.
   Validate with `auval -v aumi` and in Logic/Reaper/AUM. *First working plugin.*
2. **Editor bundle extraction.** Swift framework + `stepforge_create_editor_view`
   C entry; adapt `TransportBar`/destinations/`GridMetrics`. *Enables nih_plug GUI.*
3. **CLAP (nih_plug).** Plugin crate; `process()`→`engine_render`; `NoteEvent`
   I/O; `Transport`; editor-dylib FFI + `NSView` embedding; `setState`. Bundle +
   sign. Validate with `clap-validator` + Bitwig/Reaper.
4. **VST3 (clap-wrapper).** Wrap the Phase-3 CLAP; bundle + sign. Validate in
   Cubase/Reaper.

## Verification

- **Rust:** `cargo test` (existing + new property tests for transport→step
  mapping and render invariants); `cargo clippy --all-targets -- -D warnings`;
  `cargo check --target aarch64-apple-ios` (iOS still builds, standalone
  unchanged); `/audit-rt` on the render path and any touched RT files.
- **FFI:** add a C-ABI round-trip test for `engine_render` (mirrors the existing
  `engine_submit_command`/`engine_drain_events` tests).
- **AUv3:** `auval -v aumi <bundleid>`; manual play/stop/tempo/pattern-select in
  Logic + Reaper + AUM; confirm project save/restore persists the session.
- **CLAP/VST3:** `clap-validator`; manual transport + MIDI-I/O + editor in
  Bitwig/Reaper/Cubase; confirm state persistence.
- **Cross-cutting:** standalone app (iOS + macOS) still builds and runs
  unchanged — the host-driven mode is additive.

## Risks / open items

- **nih_plug has no tagged release** → pin a git SHA; decide upstream
  (`robbert-vdh/nih-plug`) vs the active `nice-plug` fork.
- **Swift-editor `NSView` embedding from nih_plug has no precedent** (the API
  allows it; no example exists) → spike early in Phase 3.
- **`'aumi'` host compatibility** → mitigated by the dummy audio bus.
- **MIDI-input note→action mapping is a product decision** → fixed mechanism +
  sensible default in spec; refine after dogfooding.
- **Note-offs spanning block boundaries** → handled by the scheduled-events
  queue; covered by property tests.
- **Cross-platform (Windows/Linux)** → deferred; tracked in a separate repo issue.
