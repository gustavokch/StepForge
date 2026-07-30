# StepForge CLAP/VST3 Plugin — Pure-Rust nih-plug + egui Design

> Sibling design to [`2026-07-26-plugin-port-design.md`](./2026-07-26-plugin-port-design.md).
> That approved design reuses the **SwiftUI** editor inside CLAP/VST3 via a Swift
> framework dylib (its top risk: *"Swift-editor `NSView` embedding from nih_plug has
> no precedent"*). This design takes a **different, pure-Rust** path: the editor is
> rewritten in **egui**, so the desktop plugin contains **no Swift, no FFI, no `NSView`
> bridging**. The two approaches **coexist as alternatives** — a release may ship
> either editor; they have separate bundle ids.

## Context

StepForge's musical-time core (`sequencer_engine`, `#![forbid(unsafe_code)]`) is
**already a pure-Rust, host-callable library**. Phase 0 of the plugin-port effort
(commit `e4e1946`) added `Engine::new_host_driven()` and
`Engine::render_host()` (`engine/crates/core/src/engine.rs:515`), which drive the
reused core from a host audio block with **no self-scheduled RT thread, no CoreMIDI,
no Ableton Link**. The Swift layer's `EngineBridge`, postcard codecs, and C-ABI
round-trip exist only to bridge Rust↔SwiftUI across a process boundary.

**The consequence: the UI is the only thing that is actually Swift.** For a Rust
plugin that links `sequencer_engine` directly, the entire FFI seam collapses to
in-process `Arc<Engine>` access. Replacing the SwiftUI editor with egui yields a
**100% Rust** CLAP/VST3 plugin — fully portable, single-language, and free of the
`NSView`-embedding risk that tops the sibling design.

Brainstorming locked three decisions:

- **Relationship to the approved design:** *coexist as alternatives.* This is a
  standalone design doc; it does not amend `2026-07-26-plugin-port-design.md`. The
  egui plugin ships its own bundle; the Swift-bundle path remains viable.
- **v1 scope:** *full parity* — both Editing and Performance modes plus all sheets.
  The implementation is phased internally (§Phased implementation), but the spec
  targets the complete UI.
- **Formats:** CLAP native in nih-plug; **VST3 via clap-wrapper** (avoids GPLv3
  `vst3-sys` contamination — inherited from the sibling design). A standalone
  `nih_plug_standalone` target is **deferred** (the editor is structured as a
  reusable library crate so standalone is a trivial later addition).

## Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│ DAW host (Bitwig / Reaper / Cubase / Ableton Live)                        │
│ gives: transport (tempo·position·play·time-sig), MIDI in/out, render      │
│ callback at sample rate, project state save/restore, parent window        │
└───────────────────────────────────┬──────────────────────────────────────┘
                                     │ nih_plug wrapper (CLAP) / clap-wrapper (VST3)
                                     ▼
┌─ engine/crates/clap_plugin ── nih_plug::Plugin ───────────────────────────┐
│  • owns Arc<Engine>  (Engine::new_host_driven: state-worker thread only)   │
│  • process()  →  Engine::render_host()  →  context.send_event(NoteEvent)   │
│  • context.transport()  →  HostTransport  (1:1, see §Transport mapping)    │
│  • #[persist] session blob  ↔  Command::LoadSession                        │
│  • editor() spawns the egui editor (Arc<Engine> shared into the closure)   │
└───────────────────────────────────┬───────────────────────────────────────┘
                  spawns editor (nih_plug_egui parents its own window) │
                  shares Arc<Engine> + Arc<RwLock<UiState>>            ▼
┌─ engine/crates/editor_egui ── pure egui library ───────────────────────────┐
│  render(ctx, &UiState, &mut impl CommandSink)                              │
│  • no nih_plug dependency, no Engine handle, no FFI                        │
│  • reads UiState (events + snapshot); emits Command via the sink           │
│  • headless-testable against a fixture UiState                             │
└───────────────────────────────────┬────────────────────────────────────────┘
                                     │ both depend on (unchanged)
                                     ▼
┌─ engine/crates/core ── sequencer_engine (REUSE, untouched) ────────────────┐
│  Engine::new_host_driven / render_host, Command, EngineEvent, models,      │
│  ArcSwap<Session> snapshot, lock-free command/event/MIDI queues            │
└────────────────────────────────────────────────────────────────────────────┘
```

Three layers, each with one job, each independently testable. The reused core is
**untouched**.

## Crate structure

Three new workspace members under `engine/crates/` (siblings of `core` and `ffi`):

| Crate | Type | Depends on | Purpose |
|---|---|---|---|
| `editor_egui` | lib | `sequencer_engine` (types only), `egui` (0.31, matching `nih_plug_egui`), `parking_lot` | Pure egui UI. `render(ctx, &UiState, &mut impl CommandSink)`. **No `nih_plug`, no `Engine` handle.** |
| `clap_plugin` | lib + cdylib | `sequencer_engine`, `editor_egui`, `nih_plug`, `nih_plug_egui` (pinned git rev) | The `Plugin`. Owns `Arc<Engine>`, hosts the editor, wires transport/MIDI/state. `nih_export_clap!(StepForge)`. |
| `xtask` | bin | `nih_plug_xtask` | Bundler shim: `fn main() { nih_plug_xtask::main() }`. Add `.cargo/config` alias `xtask = "run --package xtask --release --"`. |

**Why split `editor_egui` from `clap_plugin`:**

1. **Reuse** — host-framework-agnostic; a future `nih_plug_standalone` target, or a
   swap to `nih_plug_iced`/the codeberg nih-plug fork, changes only `clap_plugin`'s
   `Editor` factory, never the UI logic.
2. **Testability** — `render(&headless_ctx, &fixture, &mut recorder)` is unit-tested
   with no window, no engine, no nih_plug.
3. **Build boundary** — the iOS xcframework build (`sequencer_engine` + `ffi`) never
   pulls nih_plug. `cargo build --target aarch64-apple-ios -p sequencer_engine`
   stays green.
4. **Compile time** — UI iteration recompiles `editor_egui` only; the large nih_plug
   dep rebuilds only when the glue changes.

**egui version alignment:** `editor_egui` must use the *same* egui version as
`nih_plug_egui` (0.31 at the pinned rev). To guarantee this without a duplicate
egui, depend on `nih_plug_egui`'s re-export (`pub use egui;`) initially; switch to a
direct `egui = "0.31"` dep once the version is stable.

## Editor↔engine data flow

`Arc<Engine>` is cloned in `Plugin::editor()` into the move closure; one copy stays
on the `StepForge` struct for `process()`. `Engine` is `Send + Sync` (lock-free
`MpMcQueue`s, `ArcSwap<Session>` snapshot, state-worker thread behind a `JoinHandle`).

The editor holds `Arc<parking_lot::RwLock<UiState>>` — a Rust port of the Swift
`SessionMirror` (§Editor design). Each frame (nih_plug_egui redraws continuously at
~60 Hz while the editor is open — it calls `request_repaint()` unconditionally):

```text
update closure(ctx, _setter, _state):
  1. drain RT→GUI event channels into UiState (bounded per frame):
       hot_events  (≤256/frame): UiState::apply_hot(slot)
       large_events(≤16 /frame): UiState::apply_large(ev)   // Serialized/FullSnapshot/Error
     every 4th frame: UiState.session = engine.snapshot.load().clone()  // ArcSwap, wait-free
  2. editor_egui::render(ctx, &ui_state, &EngineCommandSink{engine})
       widgets read UiState; interactions call sink.push(Command)
            → push_drop_oldest(&engine.commands, cmd)   // the existing RT-safe path
```

```rust
trait CommandSink { fn push(&self, cmd: Command); }

struct EngineCommandSink { engine: Arc<Engine> }
impl CommandSink for EngineCommandSink {
    fn push(&self, cmd: Command) {
        let _ = sequencer_engine::midi_out::push_drop_oldest(&self.engine.commands, cmd);
    }
}
```

**Hard Rule 1 (RT thread is sacred) preserved.** The GUI thread's only writes to
RT-touched data are pushes to `engine.commands` (lock-free, drop-oldest — exactly
the path Swift uses today) and `engine.snapshot.load()` reads (ArcSwap, wait-free).
`process()` never touches `UiState`. The hot/large queues are drained, not locked.

## nih-plug integration

All API claims verified against `robbert-vdh/nih-plug` (master, via zread) and
bound to specific source files.

### The `Plugin` impl

```rust
pub struct StepForge {
    engine: Arc<Engine>,                 // Engine::new_host_driven() in Default
    host_render_state: HostRenderState,  // &mut across blocks; render_host needs it
    sample_rate: f32,                    // cached from BufferConfig in initialize()
    params: Arc<StepForgeParams>,        // #[persist] session + editor_state only
    ui_state: Arc<RwLock<UiState>>,      // GUI-only; cloned into the editor closure
}

#[derive(Default, Params)]
pub struct StepForgeParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,
    #[persist = "session"]
    session: Arc<RwLock<SessionBytes>>,  // postcard SessionEnvelope, base64-over-JSON
}
```

Required `Plugin` items: `NAME/VENDOR/URL/EMAIL/VERSION`, `AUDIO_IO_LAYOUTS`,
`type SysExMessage = ()`, `type BackgroundTask = ()`, `params()`, `process()`.
Constants:

- `MIDI_INPUT  = MidiConfig::Basic` — `render_host` already maps incoming note-ons
  in the command octave (60–67) to `QueuePattern` (engine.rs:540-551). The host routes
  note input to us from the start; Phase 0's `process()` simply passes `&[]` until the
  UI exposes pattern-select, at which point `next_event()` populates the `midi_in`
  slice.
- `MIDI_OUTPUT = MidiConfig::Basic` — we emit `NoteOn`/`NoteOff`.
- `AUDIO_IO_LAYOUTS` = a **no-op 2×2 stereo passthrough** (we never read/write
  `buffer`). Ableton Live rejects MIDI-only plugins (`AUDIO_IO_LAYOUTS = &[]`,
  per the `Plugin` trait doc note); the dummy bus makes every host accept us. The
  `midi_inverter` example uses `&[]`, which we deliberately do **not** copy.
- `SAMPLE_ACCURATE_AUTOMATION = false` (no params to automate). `HARD_REALTIME_ONLY =
  false`.

`Default for StepForge` constructs `Engine::new_host_driven()` (spawns the
state-worker only) on the main thread (nih-plug calls `Default::default()` once
before `initialize()`).

### `process()` — the RT path

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers,
           context: &mut impl ProcessContext<Self>) -> ProcessStatus {
    let tr = context.transport();
    let transport = HostTransport {
        tempo_bpm:        tr.tempo.unwrap_or(120.0),
        sample_rate:      self.sample_rate as f64,
        block_samples:    buffer.samples() as u32,
        block_start_beat: tr.pos_beats().unwrap_or(0.0),          // quarter-notes — direct
        bar_start_beat:   tr.bar_start_pos_beats().unwrap_or(0.0), // quarter-notes — direct
        is_playing:       tr.playing,
        beats_per_bar:    4.0, // engine assumes 4/4 today; unused field (host.rs:28-32)
    };

    let midi_in: &[MidiEvent] = &[];   // Phase 0; later: drain context.next_event() into a slice
    let mut midi_out = [MidiEvent::zero(); 1024];          // stack, fixed; MidiEvent: Copy

    let n = self.engine.render_host(&mut self.host_render_state, &transport, midi_in, &mut midi_out);

    for ev in &midi_out[..n] {
        if let Ok(note) = NoteEvent::<()>::from_midi(ev.sample_offset, &[ev.status, ev.data1, ev.data2]) {
            context.send_event(note);
        }
    }
    ProcessStatus::Normal
}
```

**Verified facts this relies on:**

- `ProcessContext::send_event(PluginNoteEvent)` is RT-safe and is the **output** path
  (gated on `MIDI_OUTPUT != None`). `next_event()` is the input path
  (`src/context/process.rs`).
- `NoteEvent::timing` is a **sample offset within the current block**, clamped to the
  buffer — 1:1 with `MidiEvent.sample_offset`. `NoteEvent::<()>::from_midi(timing,
  &[u8])` parses the 3-byte message, normalizes velocity to `[0,1]`, and treats
  note-on-velocity-0 as `NoteOff` (`src/midi.rs`). No hand-conversion.
- `HostRenderState` is `&mut self.host_render_state`; nih-plug guarantees `process()`
  is the sole audio-thread mutator of `&mut self`, so a struct field is safe.
  `reset()` (called after `initialize()` and on host resets) re-initializes it to
  clear envelopes/playheads.

### Transport mapping — **verified 1:1, no unit conversion**

`Transport` public fields: `playing`, `recording`, `sample_rate: f32`,
`tempo: Option<f64>`, `time_sig_numerator: Option<i32>`, `time_sig_denominator:
Option<i32>`. Position is exposed via accessor methods (not public fields):
`pos_beats()`, `bar_start_pos_beats()` — both **quarter-notes** despite the "beats"
name.

StepForge's engine also uses quarter-notes: `render_host` computes
`samples_per_beat = sample_rate / (tempo_bpm/60)` and treats each 16th as `0.25`
beats (`sixteenths = into_bar * 4.0`, `next_step_beat += 0.25`, engine.rs:553-635;
"four 16ths per beat", host.rs:28). **So nih-plug quarter-notes == StepForge beats,
directly.**

| `HostTransport` field | nih-plug source |
|---|---|
| `tempo_bpm` | `transport.tempo.unwrap_or(120.0)` |
| `sample_rate` | stored `buffer_config.sample_rate as f64` (nih-plug is f32) |
| `block_samples` | `buffer.samples() as u32` |
| `block_start_beat` | `transport.pos_beats().unwrap_or(0.0)` |
| `bar_start_beat` | `transport.bar_start_pos_beats().unwrap_or(0.0)` |
| `is_playing` | `transport.playing` |
| `beats_per_bar` | `4.0` (engine unused today; future: `num as f64 * 4.0 / den as f64`) |

### Parameters — **zero automation params**

Drive everything via `Command`. The `Command` enum (~35 variants, many with payload)
is not representable in the CLAP/VST3 parameter model, and host-automation would add
round-trip latency + snapshot fragility for no gain. The existing RT-safe command
path (`MpMcQueue<Command, 64>`, `push_drop_oldest`) is the right GUI→engine channel.
`Params` therefore carries only the two `#[persist]` fields above (this mirrors the
`midi_inverter` example's empty params). An optional `BoolParam::bypass()` may be
added later if host bypass should suspend rendering.

### State persistence — `#[persist]`, not a `serialize_state` method

There is **no** `Plugin::serialize_state()`; nih-plug persists state entirely through
`Params` `#[persist = "key"]` fields, serialized as JSON (optionally zstd-compressed)
into a `PluginState { version, params, fields }` blob (`src/wrapper/state.rs`).
`PersistentField` is blanket-impl'd for `Arc<RwLock<T: Serialize+Deserialize>>`, so
the session blob needs no manual code:

```rust
#[derive(Serialize, Deserialize)]
struct SessionBytes(#[serde(with = "serde_bytes_base64")] Vec<u8>);  // or bare Vec<u8> (~3.6× larger)
```

- **Restore:** deserialization runs **before** `initialize()`. In `initialize()`, if
  `params.session` is non-empty, push `Command::LoadSession { bytes }` — the engine's
  existing postcard restore path runs on the state-worker.
- **Save:** nih-plug has no pre-save hook. Keep the field fresh by writing the latest
  `Serialized`/snapshot bytes back into `params.session` on the GUI thread whenever
  the engine emits them. Worst case the saved snapshot is a few frames stale —
  acceptable.

`SESSION_FORMAT_VERSION` is embedded in the envelope, so the engine handles its own
versioning.

### Version pinning

nih-plug has **no released version** (`0.0.0`); the framework is in maintenance mode.
Pin all three crates to **one git rev ≥ 2025-02-23** (the egui-0.31 update) of
`robbert-vdh/nih-plug`:

```toml
nih_plug       = { git = "https://github.com/robbert-vdh/nih-plug.git", rev = "<SHA>" }
nih_plug_egui  = { git = "https://github.com/robbert-vdh/nih-plug.git", rev = "<SHA>" }
nih_plug_xtask = { git = "https://github.com/robbert-vdh/nih-plug.git", rev = "<SHA>" }
```

Record the SHA in `engine/Cargo.lock`. If a baseview/egui-baseview bug is hit, the
actively-maintained codeberg `BillyDM/nih-plug` fork is a one-line-per-dep swap
(the `editor_egui` split means the UI code is untouched by such a swap). Pick the
SHA in Phase 0 and never float `branch = "master"`.

## Editor design (`editor_egui`)

### `UiState` — Rust port of `SessionMirror`

```rust
pub struct UiState {
    // Transient (hot event channel)
    pub playing: bool,
    pub queued_pattern: Option<usize>,
    pub queued_pattern_quantize: Option<QuantizeGrain>,
    pub pattern_loop_count: [u32; 9],
    pub playheads: [u8; MAX_TRACKS],      // per-track current step index
    pub track_activity: [f32; MAX_TRACKS],// LED brightness decay (for Performance LEDs)
    pub undo_available: [bool; MAX_TRACKS],
    pub last_error: Option<String>,
    pub last_overflow: Option<u32>,
    // Authoritative (snapshot, throttled ~15 Hz refresh)
    pub session: Option<Arc<Session>>,
    frame_counter: u64,
}

impl UiState {
    pub fn apply_hot(&mut self, slot: HotEventSlot) { /* port SessionMirror.apply for hot variants */ }
    pub fn apply_large(&mut self, ev: EngineEvent)   { /* Serialized / FullSnapshot / Error */ }
}
```

`apply_hot` / `apply_large` are a direct port of the Swift `SessionMirror.apply(_:)`
per-variant matching (`app/StepForge/Engine/SessionMirror.swift`). The dual-source
model is preserved: **events for transient liveness** (playheads, queued pattern,
loop count) + **snapshot for ground truth** (tracks, steps, lengths, mutes, notes).

### Widgets

Set visuals once in `create_egui_editor`'s `build` closure; palette as module
constants. Theme: dark graphite tiers (`0x0E0E0E`→`0x353535`), orange `#FF7F00`
active only, velocity-zone colors (accent=`#FF7F00`, mid=`#FFB688`, low=`#98CBFF`),
SF-Pro/Mono-equivalent typography.

- **Step grid** (EditingView `TrackList`): pinned track-header column + horizontal
  `ScrollArea` of step columns. Each `StepCell` via
  `ui.allocate_exact_size(cell, Sense::click_and_drag())` + `ui.painter()`:
  velocity-zone fill, length-window dimming (alpha for cols ≥ `track.length`),
  2 px playhead bar when `playheads[t] == col`, ratchet markers (X2/X3/X4). Zoom 8/16
  via a local `Zoom` enum (cols 0..8 doubled-width).
- **TransportBar**: play/stop (best-effort `Command::Play/Stop`; reflect actual state
  from `playing` which is driven by `transport.playing`), BPM (read from snapshot;
  edit → `SetBpm`), sync badge (read-only — host is transport), zoom toggle.
- **FeelBar**: swing slider, humanize, quantize-grain selector, pattern switcher →
  `SetGlobalSwing/SetHumanize/SetQuantizeGrain/QueuePattern`.
- **TrackManagementBar**: add (`AddTrack`), remove (`RemoveTrack`).
- **ActionDrawer**: Roll/Vary/Cut/Copy/Paste/Trash/Undo + strength slider.
- **NotePickerSheet** (`egui::Window`): GM-drums grid + piano roll → `SetTrackNote`.
- **PerformanceView**: large PLAY/STOP, 3×3 pattern grid (`EMPTY/FILLED/PLAYING/
  QUEUED` with time-driven glow), track activity LEDs + mute toggles, quantize
  selector. **PatternOptionsSheet** (`egui::Window`): follow-action → `SetFollowAction`.
- **SettingsSheet** (`egui::Window`): global MIDI channel (`SetGlobalMidiChannel`),
  swing (`SetGlobalSwing`), sync source (read-only).

### Touch → mouse/keyboard gesture adaptation

The iOS gestures (pinch-zoom, long-press, drag-up/down) have no direct desktop
equivalent. Mapping:

| iOS gesture | Desktop |
|---|---|
| tap empty / tap filled | left-click set Mid / left-click filled cycle Mid→Accent→Low→off |
| double-tap delete | **right-click** delete (off) |
| drag up / drag down (zone) | vertical **drag** up=Accent, down=Low |
| long-press ratchet | modifier+click or right-hold → ratchet popover (`egui::Area`, X2/X3/X4) |
| pinch zoom 8↔16 | scroll-wheel / `1`·`2` keys + toolbar toggle |
| transport / pattern | hotkeys: space play/stop, `[`·`]` prev/next pattern |

Animations (playhead pulse, queued-pattern glow, LED decay) driven by
`ctx.input(|i| i.time)`; nih_plug_egui's unconditional `request_repaint()` keeps them
live while open.

## Threading & RT (Hard Rule 1)

- `process()` runs on the **host RT thread**: `transport` read (host struct),
  `HostTransport` arithmetic (scalars), `render_host` (given RT-safe), `NoteEvent::
  from_midi` (pure parse), `send_event` (lock-free output queue). No alloc, no lock,
  no FFI, no `unsafe` in our glue.
- `process()` must **not** touch `UiState`/`params.session` (lock/alloc), call
  `snapshot.load()` (not needed on RT — `host_render_state` is authoritative), or
  push `Command`s (that's the GUI→RT direction). Enable nih_plug's
  `assert_process_allocs` feature in debug builds to abort-on-alloc.
- The editor runs on the GUI thread; its only RT-adjacent writes are lock-free
  command pushes + wait-free snapshot reads.

## Build / packaging

- **CLAP:** `cargo xtask bundle stepforge --release` → `.clap`. `xtask` produces a
  universal (lipo) binary. Links `sequencer_engine` + `editor_egui`; embeds no Swift.
- **VST3:** clap-wrapper wraps the CLAP build (Phase 4). License-safe (no `vst3-sys`
  GPLv3).
- **macOS:** codesign + notarization gates for both formats; CI on the universal
  build.
- **iOS:** unaffected — `cargo build --target aarch64-apple-ios -p sequencer_engine`
  excludes the new desktop crates (CI guard).

## Phased implementation

Each phase is independently shippable. The spec targets full parity; the plan phases it.

0. **Scaffold + RT wiring.** Three crates; workspace wiring; empty
   `editor_egui::render`. `StepForge` `Plugin` with `AUDIO_IO_LAYOUTS` (dummy 2×2),
   `MIDI_OUTPUT = Basic`, `process()`→`render_host`+`send_event`, `#[persist]`
   session + `LoadSession` in `initialize()`. Editor shows BPM/play only.
   `cargo xtask bundle stepforge --release`; validate in `clap-validator` + Bitwig.
   *Lands the whole engine↔plugin loop before any UI.*
1. **EditingView core.** Port `UiState` + `apply_hot`/`apply_large`. Step-grid widget
   (pinned headers, 8/16 zoom, gestures), TransportBar, FeelBar, TrackManagementBar.
2. **ActionDrawer + NotePickerSheet.** Roll/Vary/Cut/Copy/Paste/Trash/Undo + strength;
   GM-drums grid + piano roll.
3. **PerformanceView + PatternOptionsSheet.** 3×3 pattern grid, track LEDs/mutes,
   quantize selector, follow-action sheet.
4. **SettingsSheet + theme polish + VST3 + signing.** SettingsSheet, theme/typography
   polish, clap-wrapper VST3 output, codesign + notarization, CI.

## Testing & verification

- **Unit (pure Rust, run on the iOS target too):**
  - `editor_egui`: headless `egui::Context` + `ctx.input_mut(|i| i.events.push(...))`
    → assert the `CommandSink` received the expected `Command` sequence per gesture.
  - `map_transport(&Transport, sample_rate, block_samples) -> HostTransport` as a pure
    function — `proptest` over all `Option` fields (None must fall back gracefully).
  - `MidiEvent`↔`NoteEvent` round-trip via `from_midi`/`as_midi` (NoteOn/NoteOff
    byte-for-byte; velocity within 1/127).
  - `UiState::apply_*`: scripted `EngineEvent`/`HotEventSlot` sequences → assert
    `UiState` equals hand-computed expected (the `SessionMirror`-port correctness test).
- **Integration (macOS):** reuse the engine's `host_render` test harness; drive
  `render_host` over synthetic `HostTransport` blocks; assert `MidiEvent` invariants
  (sample offset `< block_samples`; note-off follows note-on per track; ratchet
  counts). Needs no nih_plug.
- **Manual:** `clap-validator`; load the `.clap` in Bitwig (primary) + Reaper; check
  transport sync (start/stop/tempo/time-sig), MIDI-out to an instrument track, state
  save/restore round-trip, `assert_process_allocs` clean, no glitches. Phase 4:
  VST3 in Cubase/Reaper.
- **Build boundaries:** iOS xcframework still builds; macOS host
  `cargo check -p clap_plugin --target {x86_64,aarch64}-apple-darwin`.

## Risks / open items

- **nih_plug_egui maturity / maintenance mode** — its README "encourages iced/vizia".
  Mitigation: pin a known-good rev; the `editor_egui`/`clap_plugin` split means a swap
  to iced/vizia or the codeberg fork touches only the `Editor` factory. egui's
  immediate-mode painter is actually a good fit for the dynamic step grid (vizia would
  be more painful) — proceed with egui.
- **Continuous ~60 Hz redraw while open** — nih_plug_egui calls `request_repaint()`
  every frame. Acceptable for v1; optimize later with the `needs_redraw` gate the
  editor's TODO describes.
- **Resizable window** — use the `ResizableWindow` widget (host-cooperative); hosts
  may reject (`context.request_resize() == false`) → fall back to fixed/scrollable.
- **clap-wrapper (VST3) window parenting** — forwards the host parent handle; per-host
  edge cases possible (Phase 4 test matrix).
- **Ableton Live** — mitigated by the dummy 2×2 audio bus (else MIDI-only is refused).
- **`MIDI_OUTPUT != None` consumes input** — in most hosts the plugin eats upstream
  note/CC input. StepForge generates notes (doesn't forward), so this is fine, but a
  MIDI clip upstream won't pass through. Document it.
- **No pre-save hook** — keep `params.session` fresh from GUI-thread engine events
  (§State persistence).
- **Cross-platform (Windows/Linux)** — deferred (sibling design tracks it). The pure-
  Rust + egui path makes it materially easier later than the Swift-bundle path would.
- **`beats_per_bar` / non-4/4** — engine assumes 4/4 today; full time-signature
  support is a future engine change (host.rs:28-32), independent of this plugin.
- **Known upstream: nih-plug crashes on garbage CLAP state (fuzz-only).** nih-plug's
  `ext_state_load` (`src/wrapper/clap/wrapper.rs`) reads an 8-byte LE length prefix
  from the stream and calls `Vec::with_capacity(length as usize)` with no bound check;
  a state blob whose prefix decodes to a huge value causes OOM → SIGABRT. Confirmed at
  the pinned rev `f36931f` (also master's tip — upstream has not fixed it). This is in
  the framework wrapper, before the plugin's `deserialize_fields`, so no in-plugin
  workaround exists; it affects every nih-plug CLAP plugin. `clap-validator`'s
  `state-invalid-random` (3×1 MB random bytes) hits it. **Real DAWs are unaffected**
  (they send the exact bytes nih-plug wrote: valid length + valid JSON). Accepted for
  Phase 0; track an upstream issue/PR. Mitigation if later required: vendor a fork that
  clamps the length (`.min(MAX_STATE_SIZE)`) behind a cargo `[patch]`.
- **Known upstream: `clap-validator param-conversions` divides by zero on zero-param
  plugins.** StepForge deliberately exposes zero automation params (driven via
  `Command`), so the validator's own `param-count` divisor is zero → it crashes itself
  ("This is a bug in the validator"). Not a plugin defect.
