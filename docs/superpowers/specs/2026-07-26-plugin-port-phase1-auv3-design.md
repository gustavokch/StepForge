# StepForge Plugin — Phase 1 (AUv3) Design

> Approved design for the **macOS AUv3 extension** — the first working StepForge
> plugin. Wraps the Phase-0 host-driven engine, reuses the existing SwiftUI editor,
> and validates with `auval -v aumi` and in real hosts (Logic / Reaper / AUM).
> Phase 1 of the plugin port; parent design:
> [`2026-07-26-plugin-port-design.md`](./2026-07-26-plugin-port-design.md).
> **macOS only.** The standalone iOS + macOS apps build and run unchanged — this
> phase is strictly additive.

## Context

Phase 0 (PR #8, `feat/plugin-port-phase0-iso`) shipped the host-adapter: a runtime
`host_driven` construction mode, the `engine_render` C entry, a per-instance
`RenderStateHandle`, the sample→step accumulator, sample-offset MIDI out + a
fixed-size scheduled note-off/note-on queue, and the note-on→pattern-select
mapping — pure Rust, host-testable, with `proptest` + `/audit-rt` coverage. No
plugin wrapper yet.

Phase 1 wraps that surface in a **Swift `AUAudioUnit`** (`'aumi'`, MIDI-FX) whose
`internalRenderBlock` calls `engine_render` on the host's real-time thread, reuses
the standalone SwiftUI editor inside an `AUViewController`, and persists session
state with the DAW project. The result is a loadable, editable, sample-accurate
MIDI drum-sequencer plugin.

Three load-bearing facts carried from exploration:

1. **The engine's event-drain path works unchanged in host-driven mode.**
   `engine_drain_events` has no host-driven guard; the state worker (still spawned
   by host-driven `engine_start`) emits `FullSnapshot`/snapshot events on the large
   channel, and `render_host → process_one → process()` emits playhead events on
   the hot channel at the same tempo-bound rate as the standalone RT loop (~8
   16ths/s @120 BPM, drained at ~120 Hz). So the editor reuses `EngineBridge`'s
   drain loop verbatim — **zero engine changes for the editor.** (The Phase-0
   docstring note "host-driven mode has no event drain … until Phase 2" describes
   the pure-Rust Phase-0 *test* context, where no GUI drainer is wired — not a
   fundamental limitation. The parent design spec, lines 104 / 227–228, states the
   drain stays live.)
2. **The real `engine_render` ABI has a per-instance `RenderStateHandle` and a
   unified `MidiEvent`** (Phase 0 intentional deviations from the parent spec's
   sketch). This design targets the shipped ABI, not the sketch.
3. **The Swift reuse gap is handle ownership, not events.** `EngineBridge` today
   constructs its own standalone `engine_new()` handle and inseparably couples
   `engine_start` with the ~120 Hz drain timer. The AU must call `engine_render` on
   the *same* handle from the host RT thread — so Phase 1 adds a host-driven
   construction + handle-sharing path (Swift-only, additive).

## Decisions locked in brainstorming

- **Event drain — reuse `engine_drain_events` as-is.** No new host-side event
  mechanism; validate empirically and revisit only if the playhead proves
  unreliable. (Fallbacks considered and rejected: a dedicated observation FFI
  entry, or deriving playhead from AU transport — both unnecessary.)
- **Handle ownership — the AU owns the engine + render-state + lifecycle;
  `EngineBridge` borrows.** `EngineBridge` gains a borrowed-handle initializer and
  an `ownsLifecycle` flag; in borrowed mode it arms **only** the drain timer (no
  `engine_start`/`stop`/`free`) and submits/drains against the AU's handle. The
  standalone path is bit-for-bit unchanged.
- **Editor scope — full editor, trimmed.** Editing + Performance + Settings (the
  MIDI-routing section hidden) + a plugin-mode `TransportBar`. New trimmed editor
  host; drops `MidiManager`/destinations + app-shell/`EngineLifecycle`.
- **MIDI-input scope — Phase-0 default only.** Incoming note-ons in the command
  octave (C4 = 60+) → `QueuePattern` (NextStep), already implemented in
  `render_host`. No Thru, no mode-config UI, **no engine change.** Thru and the
  configurable input mode defer to a later phase.
- **State persistence — both `fullState` and `fullStateForDocument`, same session
  bytes.** `fullStateForDocument` is the primary path (project-scoped save);
  `fullState` covers instance restore / broad host compatibility.
- **Manufacturer/subtype — `'SFor'` / `'DrmS'`** (defaults; trivially changed).
- **Carry-forward — Link quiescence stays deferred to Phase 3** (host-driven
  `engine_new` constructs an idle Link; the poller is not spawned — acceptable in a
  DAW for Phase 1). **Plan against `feat/plugin-port-phase0-iso`** (ABI stable &
  committed; the plan is a document, trivially updated if PR #8 review tweaks it).
  Implementation branches off iso, or off main once #8 merges.

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│ DAW host (Logic / Reaper / AUM)                                   │
│  audio render callback (RT) · musical/transport context · MIDI    │
│  I/O EventLists · project state save/restore                      │
└───────────────────────────┬──────────────────────────────────────┘
                            │ AUv3 SDK
                            ▼
┌──────────────────────────────────────────────────────────────────┐
│ StepForgeAU.appex  (macOS Audio Unit Extension)                   │
│                                                                    │
│  StepForgeAudioUnit (AUAudioUnit, 'aumi')                          │
│   ├─ owns: engine handle (engine_new_host_driven)                  │
│   │         render-state (engine_render_state_new)                 │
│   │         lifecycle (engine_start → stop → free)                 │
│   ├─ allocateRenderResources: capture context/transport/MIDI-out   │
│   │                            blocks; build internalRenderBlock   │
│   ├─ internalRenderBlock (host RT):                                │
│   │     HostTransportBuilder → HostTransport                       │
│   │     MIDIMarshaler.in(AUMIDIEventList) → [MidiEvent]            │
│   │     engine_render(engine, rs, transport, in, out)              │
│   │     MIDIMarshaler.out([MidiEvent]) → host MIDI-output block    │
│   │     zero-fill dummy stereo audio bus                           │
│   └─ fullState / fullStateForDocument ↔ engine_serialize           │
│                                                                    │
│  EngineBridge (REUSED, +borrowed-handle mode)                      │
│   └─ ~120 Hz drain timer → SessionMirror (off-RT, MainActor)       │
│                                                                    │
│  StepForgeEditorViewController (AUViewController)                  │
│   └─ NSHostingView(PluginEditorView)  ← reused SwiftUI editor      │
└───────────────────────────┬──────────────────────────────────────┘
                            │ C ABI (bytes; Rules 3–4)
                            ▼
┌──────────────────────────────────────────────────────────────────┐
│ sequencer_engine  (Phase 0, REUSE unchanged)                      │
│  engine_render → render_host: transport→step, sample-offset MIDI,  │
│  note-on→pattern-select, seek/loop re-align, stop→CC123            │
└──────────────────────────────────────────────────────────────────┘
```

One engine instance per AU. The AU is the RT owner and lifecycle owner;
`EngineBridge` is a pure command/event view-model over the borrowed handle (mirror
pattern, Rule 2). The engine itself is untouched.

## Components (new Swift)

All new files live under `app/StepForge/AudioUnit/` (macOS-only target sources).
Each has one job and is independently testable where it is a pure mapping.

### `StepForgeAudioUnit` (`AUAudioUnit` subclass, `'aumi'`)

- **`init(componentDescription:options:)`** — store the description; create the
  host-driven engine (`engine_new_host_driven`) and call `engine_start` (spawns
  **only** the state worker). Construct `EngineBridge` in **borrowed-handle mode**
  and call `start()` (borrowed mode arms the ~120 Hz drain timer and skips
  `engine_start`, since the AU already started the worker). Submit
  `RequestFullSnapshot` to seed the mirror.
- **Bus config:** one dummy **stereo (2-ch) audio output** bus (declared via
  `setBusTypes` / `outputBusFormat`); MIDI I/O is via the render block, not audio
  busses.
- **`allocateRenderResources`** — create the `RenderStateHandle`
  (`engine_render_state_new`); capture `musicalContextBlock`,
  `transportStateBlock`, and the MIDI-output block (`AUMIDIOutputEventListBlock`);
  assemble `internalRenderBlock` closing over the engine handle, render-state, and
  captured blocks. (Re-called on config changes — e.g. sample-rate change — so the
  render-state is allocate/deallocate-scoped while the engine handle + worker
  persist.)
- **`deallocateRenderResources`** — free the render-state
  (`engine_render_state_free`).
- **`dealloc`** — `bridge.stop()` (cancel timer) → `engine_stop` (join worker) →
  `engine_free`. Free the render-state if it was allocated.
- **`fullState` / `fullStateForDocument`** — both backed by the same session bytes
  (§Persistence).
- **Factory** — discovered via `Info.plist` (`NSExtensionPointIdentifier =
  com.apple.AudioUnit-UI`, `NSExtensionPrincipalClass =
  StepForgeEditorViewController`, component description in `NSExtensionAttributes`).

### `HostTransportBuilder` (pure, unit-tested)

Maps captured AU context/transport values into the `HostTransport` C struct:

| `HostTransport` field | source |
|---|---|
| `tempo_bpm` | `musicalContextBlock.tempo` |
| `sample_rate` | output bus `format.sampleRate` (from `allocateRenderResources`) |
| `block_samples` | render frame count (`inNumberFrames`) |
| `block_start_beat` | `musicalContextBlock.beat` |
| `bar_start_beat` | `musicalContextBlock.currentDownBeat` |
| `is_playing` | `transportStateBlock` `.playing` flag |
| `beats_per_bar` | `musicalContextBlock.timeSignatureUpper` (passed through; Phase-0 accumulator still assumes 4/4) |

Pure function of inputs → no handle, no side effects → property-testable.

### `MIDIMarshaler` (pure, unit-tested)

- **`in(_:into:)`** — iterate the incoming `AUMIDIEventList`, copy channel-voice
  messages into a fixed-size `[MidiEvent]` buffer (cap, e.g. 64) with their sample
  offsets. **Drop-tail on overflow** (bounded, RT-safe).
- **`out(_:into:)`** — write returned `MidiEvent`s to the host MIDI-output block,
  preserving sample offsets, into a fixed-size output budget.
- Pure translation logic (sample-offset fidelity, ordering, overflow behavior) →
  property-testable without a host.

### `EngineBridge` (REUSED, additive change)

Add a borrowed-handle initializer and an `ownsLifecycle` flag (default `true`):

- `init(handle:)` sets `ownsLifecycle = false`.
- `start()` — if `ownsLifecycle`, call `engine_start`; **always** arm the ~120 Hz
  drain timer.
- `stop()` / `deinit` — cancel the timer; only call `engine_stop`/`engine_free`
  when `ownsLifecycle`.

The standalone path (`ownsLifecycle == true`) is bit-for-bit unchanged;
`EngineBridgeTests` must still pass. No change to `submit`/`drainOnce`/`serialize`/
`load`.

### `PluginEditorView` + plugin `TransportBar`

- **`PluginEditorView`** — trimmed `RootView`: mounts `EditingView` +
  `PerformanceView` + `SettingsSheet`, bound to the borrowed `EngineBridge` via
  `.environmentObject`. No app-shell, no `MidiManager`, no `EngineLifecycle`/
  `ScenePhase` wiring.
- **Plugin `TransportBar`** — read-only "Following host — BPM · bar · step" readout
  (BPM + step from the drained mirror; bar from musical context) **+ keep the 8/16
  zoom toggle**. Drops the play/stop / BPM-input / sync-source controls (they write
  transport the host owns).
- **`SettingsSheet`** — conditionally hide the MIDI-routing/destinations section
  (depends on the dropped `MidiManager`); keep swing/humanize/etc.

### `StepForgeEditorViewController` (`AUViewController`)

`viewDidLoad` hosts `PluginEditorView` in an `NSHostingView`, and is the
`NSExtensionPrincipalClass` that vends the `AUAudioUnit` via
`createAudioUnit(with:)`.

## Data flow

**Per audio block (RT):** build `HostTransport` → marshal `midi_in` →
`engine_render` → marshal `midi_out` → zero-fill audio. All fixed-buffer, no alloc.

**Command/event (editor ↔ engine):** gesture → `bridge.submit(Command)` →
`engine_submit_command` → MPSC → state worker applies (off-RT) → events on hot/large
channels → `bridge.drainOnce` (~120 Hz, off-RT) → one MainActor hop →
`SessionMirror`. Identical to standalone.

**MIDI output:** `engine_render` writes `MidiEvent`s (sample offsets within the
block, deferred note-offs/note-ons carried across blocks by `PendingMidiQueue`);
the AU forwards them to the host MIDI-output block.

**Persistence:** `fullState`/`fullStateForDocument` getter → `engine_serialize` →
postcard `SessionEnvelope` bytes → `["session": Data]`; setter →
`bridge.load(Data)` → `engine_submit_command(LoadSession)` (round-trips the running
state-worker queue).

## Threading & RT (Hard Rule 1 preserved)

- `internalRenderBlock` runs on the **host's RT thread**: no allocation, no locks,
  no `Vec`/`String`/`format!`, no FFI-out, no CoreMIDI, no Link. Only `engine_render`
  (already RT-safe, Phase-0-audited) + fixed-size MIDI buffers.
- The engine handle is **shared**: `engine_render` on RT, `engine_submit_command`/
  `engine_drain_events` on `drainQueue` — the same lock-free concurrency model as
  standalone (RT + worker + GUI touch disjoint structures: command MPSC, hot/large
  event channels, MIDI ring, COW snapshot). `engine_start`/`stop`/`free` are
  serialized by the AU's lifecycle (Rule 5: `stop` returns before `free`).
- The `RenderStateHandle` is **single-owner**: `internalRenderBlock` is the only
  accessor, always on the host RT thread. No synchronization needed.
- The editor drain is **off-RT**: the `drainQueue` timer + one MainActor hop per
  batch — unchanged from standalone.

## MIDI I/O

- **Input:** the render block receives an `AUMIDIEventList`; `MIDIMarshaler.in`
  copies channel-voice messages into `midi_in` with sample offsets.
  `render_host` maps note-ons in the command octave → `QueuePattern` (Phase 0).
- **Output:** `engine_render` writes `MidiEvent`s; `MIDIMarshaler.out` forwards them
  to the host MIDI-output block with sample offsets.
- **Dummy audio bus:** `internalRenderBlock` zero-fills the stereo output (the
  engine emits no audio). MIDI rides the MIDI-output block, not the audio bus.
- **CoreMIDI boundary (Rule 7):** in AUv3, MIDI flows through the host's
  `*EventList` blocks, **not** CoreMIDI directly. The AU never touches
  `MIDIClientRef`/endpoints; `MidiManager` is dropped from the extension entirely.

> The exact AUv3 MIDI I/O block symbols (`AUMIDIOutputEventListBlock`, render-block
> input event list) and `AudioComponentDescription` constants are verified against
> Apple's AudioToolbox docs via Context7 **during plan-writing**. The *mechanism*
> above is settled; only the exact symbol names are TBD.

## State persistence

Reuses `engine_serialize` (versioned postcard `SessionEnvelope`,
`SESSION_FORMAT_VERSION` unchanged — no model ripple). Both `fullState` and
`fullStateForDocument` are implemented, returning/accepting the same
`["session": Data]`:

- `fullStateForDocument` — project-scoped (the primary path; session persists with
  the DAW project).
- `fullState` — instance restore / broad host compatibility.

## Build / packaging

- **New `StepForgeAU` target** in `app/project.yml` (xcodegen): Audio Unit
  Extension (`.appex`), `platform: macOS`, `deploymentTarget macOS 14.0`,
  **embedded in `StepForge-macOS`** (the container app). `bundleIdPrefix` already
  `com.stepforge`.
- **Sources — reused** (shared with the app targets): `Engine/*` **minus**
  `MidiManager.swift`, `EngineLifecycle.swift`, `StepForgeApp.swift`,
  `RootView.swift` (and **plus** the additive borrowed-handle change to
  `EngineBridge.swift`); `Features/*`, `Theme/*`, `Components/*`, `Postcard/*`,
  `Models.swift`, `Command.swift`, `EngineEvent.swift`, `SessionMirror.swift`.
- **Sources — new** (`app/StepForge/AudioUnit/`): `StepForgeAudioUnit`,
  `StepForgeEditorViewController`, `HostTransportBuilder`, `MIDIMarshaler`,
  `PluginEditorView`, plugin `TransportBar`.
- **Link** the engine xcframework (`embed: false` — static `.a`, same as the app
  targets; the macOS slice already exists). Bridging header →
  `engine/include/sequencer_engine.h`; matching search paths. `build_engine.sh`
  `preBuildScript`.
- **`Haptics.swift`** already macOS-clean (`#if os(macOS)` branch). **iOS app target
  untouched.**
- **`Info.plist`:** `NSExtensionPointIdentifier = com.apple.AudioUnit-UI`;
  `NSExtensionPrincipalClass = StepForgeEditorViewController`;
  `NSExtensionAttributes` carrying the `AudioComponentDescription` (`type = 'aumi'`,
  `subtype = 'DrmS'`, `manufacturer = 'SFor'`, `name`, `version`, `tags`).
- **Signing:** flip `CODE_SIGNING_ALLOWED` to `YES` for the `.appex` target. Phase 1
  local validation uses **ad-hoc / development signing** (`auval` and Logic/Reaper
  load ad-hoc-signed AUs locally). Notarization is post-Phase-1.

## Validation

- **Swift unit tests (XCTest, no host)** — the pure mappings, which are the bulk of
  the new logic: `HostTransportBuilder` (context/transport → struct, incl. stopped
  & seek cases); `MIDIMarshaler` both directions (sample-offset fidelity, ordering,
  drop-tail overflow, fixed-buffer bounds); `fullState`/`fullStateForDocument`
  encode → set → restore → encode round-trip equality.
- **RT invariants (covered by Phase 0)** — step-boundary firing, sample offsets in
  `[0, block_samples)`, stop-freeze + all-notes-off, deterministic reseed are
  already `proptest`'d against `engine_render`. The Swift wrapper supplies correct
  inputs; it does not re-implement dispatch.
- **Host-validatable only** — `auval -v aumi com.stepforge.app.mac.StepForgeAU`;
  manual play/stop/seek/tempo/pattern-select + project save/restore in **Logic,
  Reaper, AUM**. (`internalRenderBlock` cannot be exercised end-to-end without a
  host — this bar is intentional and matches the parent spec.)

## Standalone regression (additive-only)

iOS `StepForge` + macOS `StepForge-macOS` build and run unchanged. The single
shared-file change — `EngineBridge.swift`'s additive borrowed-handle init +
`ownsLifecycle` flag — leaves the standalone path (default `true`) bit-for-bit
identical. `EngineBridgeTests` (and the iOS test target) must still pass. No
engine-side change of any kind.

## Deviations from the parent design spec

None that change intent. The parent spec's Phase-0 *prose* sketch of `engine_render`
(separate `IncomingMidi`/`OutgoingMidi`, accumulator in `RtState`, a cargo
`host-driven` feature) was already superseded by Phase 0's shipped ABI (unified
`MidiEvent`, accumulator in `HostRenderState`, runtime `host_driven` flag). Phase 1
targets the **shipped** ABI. Phase-1-specific refinements to the parent spec are the
locked decisions above (handle ownership, editor scope, MIDI-input default,
dual-key persistence) — all consistent with the parent spec's direction.

## Scope boundary (explicitly deferred)

- **Thru + configurable MIDI-input mode** → later phase (spec: "product detail
  refined later").
- **Ableton Link quiescence** → Phase 3 (idle Link acceptable for Phase 1).
- **`stepforge_create_editor_view` seam / Swift editor framework bundle** → Phase 2
  (Phase 1 instantiates the editor directly in the same Swift module — no `dlopen`).
- **CLAP / VST3** → Phases 3–4.
- **Notarization / distribution signing** → post-Phase-1.

## Risks / open items

- **`'aumi'` host compatibility** — mitigated by the dummy stereo audio bus.
- **AU MIDI I/O exact API** — mechanism settled; exact symbols verified at
  plan-writing via Apple docs.
- **Headless testability** — pure mappings unit-tested; integration is
  host-validatable (`auval` + manual). No end-to-end `internalRenderBlock` unit test
  is possible without a host.
- **Building on unmerged PR #8** — plan against `feat/plugin-port-phase0-iso`; the
  `engine_render` ABI is stable and committed, and the plan is trivially updated if
  review changes it.
- **Editor event-drain reliability** — reuse is evidence-backed (spec + code); if
  the playhead proves unreliable in a real host, the fallback is a minimal
  host-side observation path (deferred unless observed).
