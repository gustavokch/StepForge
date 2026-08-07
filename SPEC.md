# SPEC

## §G GOAL
Pure-Rust CLAP/VST3 drum-sequencer plugin. `nih_plug` drives the reused
`sequencer_engine` (host-driven); an egui editor replaces SwiftUI — no Swift,
no FFI, no `NSView`. Full UI parity with the iOS app, shipped phased.
Phase 0 (skeleton + RT loop + minimal editor) shipped PR #12; Phase 1+ is the
real UI.

## §C CONSTRAINTS
- **macOS desktop** (Phase 0 shipped PR #12; Phase 1+ continues macOS). The iOS build MUST stay green on every task: `cargo check -p sequencer_engine --target aarch64-apple-ios` PASS.
- The `sequencer_engine` core stays `#![forbid(unsafe_code)]` and untouched — all new code lives in three new crates (`editor_egui`, `clap_plugin`, `xtask`).
- **Hard Rule 1 — RT sacred**: `process()` is alloc-free, lock-free, no FFI, and touches no `UiState`/`params.session`. The GUI thread only writes the lock-free `engine.commands` and reads the ArcSwap snapshot. Enable `nih_plug` `assert_process_allocs` in debug.
- **Transport units are quarter-notes on both sides** — no conversion. `pos_beats()`/`bar_start_pos_beats()` feed `HostTransport` directly.
- Pin `nih_plug` + `nih_plug_egui` + `nih_plug_xtask` to ONE shared git rev `f36931f` — never float `master`, never mismatched revs.
- **License**: no `vst3-sys` (GPLv3). VST3 ships via clap-wrapper in Phase 4. Phase 0 is CLAP only.
- **MIDI config**: `MIDI_INPUT = Basic`, `MIDI_OUTPUT = Basic`, `AUDIO_IO_LAYOUTS = &[]` (MIDI-only; the Live dummy 2×2 audio bus is deferred to Phase 4).
- **egui version align**: `editor_egui` uses the same egui as `nih_plug_egui` (0.31 @ pinned rev).
- **Zero automation params** — driven via `Command`; `Params` carries only `#[persist]` fields.

## §I INTERFACES
- **crate `editor_egui` (lib)**: `pub trait CommandSink { fn push(&self, cmd: Command) }`, `pub struct UiState`, `pub fn render(ctx, &UiState, &impl CommandSink)`, `pub fn transport_action(playing: bool) -> Command`, `pub fn apply_theme(&Context)`. Never depends on `nih_plug`, never holds an Engine handle, never touches FFI — headless-testable.
- **crate `clap_plugin` (lib + cdylib, `stepforge_clap`)**: `pub struct StepForge` (`impl Plugin` + `impl ClapPlugin` + `nih_export_clap!(StepForge)`), `pub struct StepForgeParams`. Owns `Arc<Engine>`.
- **crate `xtask` (bin)**: `cargo xtask bundle stepforge_clap --release` → `.clap` under `engine/target/bundled/`.
- **Plugin contract**: `Engine::new_host_driven()` in `Default`; `process()` → `transport::map_transport()` → `Engine::render_host()` → `midi::midi_event_to_note()` → `context.send_event()`; the play→stop edge fires an explicit `NoteOff` burst across all 16×128 (CC 123 filtered); `#[persist] session` → `Command::LoadSession` in `initialize()`; an empty session seeds the `demo_session()` beat (audible transport sync pre-Phase-1).
- **Engine reuse API** (unchanged; module-qualified — core has no root re-exports, per `lib.rs`): `engine::Engine::{new_host_driven, render_host, run_worker_loop, snapshot_arc}` + fields `commands`/`hot_events`/`large_events`/`shutdown`; `host::{HostTransport, HostRenderState, MidiEvent}`; `command::Command`; `event::EngineEvent`; `midi_out::push_drop_oldest`; `serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION}`; `models::Session`.
- **CLAP id**: `org.stepforge.clap`; features `[NoteEffect]` (CLAP note port; `midi-effect` dropped in PR #12).
- **Env**: none.

## §R RESEARCH
id|topic|finding|src
R1|NoteEvent parse|`NoteEvent::from_midi(timing, &[u8])` → `Result<NoteEvent, u8>`: NoteOn vel-0 → `NoteOff`; velocity `/127.0` ∈ [0,1]; `timing` = sample offset (clamped to buffer). CC parses to `MidiCC` (NOT `None`) — our `NoteOn`/`NoteOff` filter drops it|github.com/robbert-vdh/nih-plug `src/midi.rs`
R2|Transport units|`pos_beats()`/`bar_start_pos_beats()` = quarter-notes (bar len = `num/den*4.0`); `tempo: Option<f64>`, `sample_rate: f32`, `playing: bool`, `time_sig_{numerator,denominator}: Option<i32>` public. Engine also quarter-notes (`0.25` beat/16th) → direct, no conversion|github.com/robbert-vdh/nih-plug `src/context/process.rs`
R3|MIDI I/O paths|`send_event()` = host output (gated `MIDI_OUTPUT`); `next_event()` = host input (gated `MIDI_INPUT`). Both `&mut self`|github.com/robbert-vdh/nih-plug `src/context/process.rs`
R4|state persistence|no `serialize_state()`; `#[persist = "key"]` on `PersistentField<T: Serialize+Deserialize>` → JSON string in `PluginState.fields` map; restore via `set_state` (blocks GUI, applied end of process cycle). `Arc<RwLock<Vec<u8>>>` qualifies|github.com/robbert-vdh/nih-plug `src/wrapper/state.rs`
R5|process() sole RT mutator|`HostRenderState` as `&mut self` field safe — nih_plug guarantees `process()` sole audio-thread mutator of `&mut self`; `reset()` re-inits after `initialize()`/host reset|docs/superpowers/specs/2026-07-27-clap-egui-editor-design.md (verified via nih-plug trait docs)
R6|egui redraw|`nih_plug_egui` `create_egui_editor` calls `request_repaint()` every frame → ~60Hz continuous redraw while editor open (accepted v1)|docs/superpowers/specs/2026-07-27-clap-egui-editor-design.md
R7|nih_plug release|`nih_plug` `0.0.0`, maintenance mode (README "encourages iced/vizia"); pin shared git rev ≥ egui-0.31 update (2025-02-23); workspace pins `f36931f`|github.com/robbert-vdh/nih-plug + `engine/crates/clap_plugin/Cargo.toml`
R8|audio IO|`AUDIO_IO_LAYOUTS = &[]` MIDI-only compiles + validates in Bitwig/Reaper; Ableton Live rejects MIDI-only → dummy 2×2 bus (Phase 4). `midi_inverter` eg uses `&[]`|docs/superpowers/specs/2026-07-27-clap-egui-editor-design.md
R9|clap-validator|state/param-scan crashes are UPSTREAM nih-plug (unbounded alloc on state restore + validator div-by-zero on zero-param plugin) — NOT our defect, do not chase|PR #12 verification + nih-plug upstream

R1-R4 verified direct @ master (= pinned `f36931f` core trait API); R5/R6/R8 inherited from approved design doc (author verified via zread); R9 from PR #12 verification (upstream nih-plug).

## §V INVARIANTS
V1 — Every `process()` and `reset()` call is alloc-free, lock-free, no FFI, and touches no `UiState`/`params.session`. `assert_process_allocs` (debug) is the enforcement gate; `reset()` runs on the RT thread, so it must be alloc-free.

V2 — Transport mapping stays in quarter-notes on both sides: `pos_beats()`/`bar_start_pos_beats()` feed `HostTransport.block_start_beat`/`bar_start_beat` directly, with no unit conversion.

V3 — Every emitted `MidiEvent` has `sample_offset ∈ [0, block_samples)`; note-off follows note-on per track; only `NoteOn`/`NoteOff` are forwarded (CC dropped).

V4 — `editor_egui` never depends on `nih_plug`, never holds an Engine handle, never touches FFI — pure UI, headless-testable.

V5 — The iOS build stays green: `cargo check -p sequencer_engine --target aarch64-apple-ios` PASS on every task (desktop crates never contaminate iOS).

V6 — The `sequencer_engine` core stays `#![forbid(unsafe_code)]` and untouched — Phase 0 adds no core code.

V7 — All `nih_plug` deps share ONE git rev `f36931f` — never float `master`, never mismatched revs.

V8 — Session restore: if `params.session` is non-empty at `initialize()`, push `Command::LoadSession { bytes }` (runs on the state-worker); `SESSION_FORMAT_VERSION` is embedded in the envelope.

V9 — GUI→RT only writes the lock-free `engine.commands` (`push_drop_oldest`); RT→GUI reads the wait-free ArcSwap snapshot (`snapshot_arc`) and drains fixed-slot channels — the RT thread never blocks on a lock.

V10 — Zero automation params: all interaction flows through `Command`; `Params` is `#[persist]` `editor_state` + `session` only.

V11 — Every `initialize()` triggers `reset()` to re-init `HostRenderState` (clear `pending` MIDI + realign `rt`/`sample_time`/`last_block_start_beat`/`was_playing`). Guards against stale render state across state restore (preset/project swap). `HostRenderState::new()` is not alloc-free (fixed arrays). Cites I.plugin contract.

V12 — The play→stop edge fires an explicit `NoteOff` burst across all 16 channels × 128 notes @ timing 0 — CC 123 all-notes-off is dropped by the `NoteOn`/`NoteOff` filter, so stuck-note prevention needs explicit offs. RT-safe (`send_event` is lock-free output). Cites I.plugin contract.

V13 — On every re-activation (deactivate→initialize same instance), do NOT clear the `engine.shutdown` latch (in `ensure_worker`) → the spawned worker drains commands. Guards the dead-worker regression (PR #12).

## §T TASKS
id|status|task|cites
T1|x|scaffold `editor_egui` + `clap_plugin` + `xtask`; pin `nih_plug` rev `f36931f`; workspace members; `.cargo/config.toml`|V5,V6,V7
T2|x|`transport::map_transport()` pure helper + tests|V2
T3|x|`midi::midi_event_to_note()` pure helper + tests|V3
T4|x|`StepForgeParams` `#[persist]` `editor_state` + `session`; JSON round-trip test|V8,V10
T5|x|`StepForge` struct + `Default` + `impl Plugin` consts + `impl ClapPlugin` + `nih_export_clap!`|I.clap_plugin,V6
T6|x|fill `initialize()` (sample rate + spawn worker + `LoadSession` restore + demo seed), `process()` (RT loop + reused `midi_buf` + stop-edge `NoteOff` burst), `deactivate()` (teardown: join + latch shutdown + reset `HostRenderState`), `reset()` (re-init `HostRenderState` + `was_playing`, alloc-free, RT-thread — closes V11 stale render state on preset/project swap); `assert_process_allocs` enabled (cargo feature)|V1,V2,V3,V8,V9,V11,V12,V13
T6m|~|manual DAW validation of V11 no-deactivate reset path: `cargo xtask bundle stepforge_clap --release` → load in Bitwig → swap preset/project WHILE PLAYING on an active instance → assert playhead realigns + no hanging notes (deferred MIDI dropped on `reset()` is mitigated by `process()` NoteOff burst V12). Headless coverage extended: `reset_then_render_resumes_clean` drives `render_host` from clean baseline (nih_plug `Buffer`/`Transport` unconstructable downstream ∴ literal initialize→reset→process stays manual)|V11,V12
T7|x|`editor_egui` minimal: `UiState` + `CommandSink` + `transport_action` + `render` + `apply_theme` (BPM/play) + tests|V4
T8|x|`editor()` wiring + per-frame GUI tick (drain hot/large channels, surface `EngineEvent::Error`, throttle snapshot refresh + serialize-for-save w/ change-detect)|V4,V9
T9|x|`cargo xtask bundle`; `clap-validator` (R9 upstream crashes noted); load in Bitwig/Reaper (transport follow, MIDI out, state round-trip, editor opens); `cargo test --workspace` + iOS guard|V1,V5
T10|x|Phase 1 EditingView core (umbrella — T10a..T10f). Port SessionMirror → egui widgets; no Swift|V4
T10a|x|expand UiState (port SessionMirror fields: HashMap playheads, HashSet undo, single pattern_loop_count, queued_pattern/quantize, linkPeers/linkEnabled, lastError/lastOverflow — GUI-only ∴ heap OK; ⊥ design-doc fixed-array sketch = RT reflex) + port apply() from SessionMirror.apply(_:) ∀ 21 EngineEvent variants (playhead coalesced via applyPlayhead, ⊥ apply) + arg-shape: recommend ONE apply(&EngineEvent) (faithful Swift; hot bytes decode in editor.rs tick + large already decoded → both → apply), supersedes design-doc apply_hot(slot)/apply_large (keeps editor_egui postcard & HotEventSlot-free ∴ cleanest V4) + wire editor.rs tick (drop Phase 0 inline PlayStateChanged) + applyOptimistic ⊥ ported (iOS MockEngineBridge-only; in-proc engine echoes via channels+snapshot) + headless apply tests (scripted seqs → UiState == hand-computed)|V4
T10b|x|step-grid widget (EditingView TrackList port): pinned track-header col + horizontal step cells, Zoom enum 8/16, StepCell painter (velocity-zone fill, length-window alpha dim cols≥track.length, 2px playhead bar @ playheads[t], ratchet X2/X3/X4), gestures → Command::SetStep/DeleteStep/SetRatchet (tap set Mid / filled cycle Mid→Accent→Low→off / right-click delete / vertical-drag zone). headless gesture tests. Roll/Vary/Cut/Copy/Paste/Trash/Undo = T11 Phase 2|V4
T10c|x|TransportBar: play/stop (transport_action→Command::Play/Stop, reflect UiState.playing actual NOT optimistic), BPM (snapshot read + edit→Command::SetBpm), sync badge read-only (host=transport), zoom toggle|V4
T10d|x|FeelBar: swing slider→Command::SetGlobalSwing, humanize→Command::SetHumanize, quantize-grain selector→Command::SetQuantizeGrain, pattern switcher→Command::QueuePattern|V4
T10e|x|TrackManagementBar: add→Command::AddTrack, remove→Command::RemoveTrack|V4
T10f|x|Phase 1 close: cargo xtask bundle stepforge_clap --release → .clap; Bitwig/Reaper (step-grid edits audible, transport follows, widgets responsive); cargo test -p stepforge_editor_egui + stepforge_clap + workspace; iOS guard cargo check -p sequencer_engine --target aarch64-apple-ios (rustup PATH); flip T10 .→x|V1,V5,V7
T11|x|Phase 2 ActionDrawer + NotePickerSheet (Roll/Vary/Cut/Copy/Paste/Trash/Undo + strength; GM-drums grid + piano roll); DAW smoke GO (Bitwig); review cleanup @1674ccc (shared overlay::should_dismiss + test_support harness merge, stale-target OOB close guard, GM-label LazyLock precompute, dead slider-probe drop); cargo test + clippy -D warnings + iOS guard|V1,V4,V5
T12|x|Phase 3 PerformanceView + PatternOptionsSheet (AppMode toggle Editing↔Performance; 3×3 pattern grid EMPTY/FILLED/PLAYING/QUEUED + PLAYING-cell pulse + follow-action next-destination glow, track activity LEDs + mute toggles, quantize selector; follow-action → SetFollowAction w/ PlaySpecific target picker) + whole-pattern Cut/Copy/Paste/Clear clipboard (CopyPattern/CutPattern/PastePattern/ClearPattern — Clear resets steps so slot stays Some ∴ RT-safe no None-active risk; Paste preserves target id ∴ no PlaySpecific Uuid collision; no new EngineEvent, FullSnapshot carries it); DAW smoke GO (Bitwig) @c9a71b6+dce6acd; cargo test + clippy -D warnings + iOS guard|V1,V4,V5
T13|.|Phase 4 distribution remainder: VST3 (clap-wrapper) + codesign/notarization + CI + Live dummy 2×2 audio bus (umbrella; T13a SettingsSheet + T13b theme/typography done)|V7
T13a|x|CLAP SettingsSheet: gear in TransportBar opens floating Area+Frame::popup; read-only Session status (BPM/Sync/Link) + writable Global MIDI Channel ComboBox 1-16 → Command::SetGlobalMidiChannel (commit-on-change, iOS Picker parity); sync read-only (host owns transport ∴ emits no SetSyncSource, honors T10c); symmetric mutual exclusion w/ note_picker/action_drawer/pattern_options; mode-agnostic (toggle close-list excludes settings; outside-click dismiss shared); UiState.global_midi_channel accessor. No new Command/Event. All automated gates green (133 editor + workspace + clippy -D + iOS + header byte-identical). DAW smoke PENDING (Bitwig)|V4,V7
T13b|x|CLAP theme/typography: theme.rs (iOS Theme.swift port — 5 surface tiers, border/brand/text/zone, DANGER, Spacing f32, Radius u8) + typography.rs (7-role type-scale; egui default fonts; medium→normal, semibold→bold); apply_theme wired (item_spacing/button_padding/rounding/override_text_color/7 text_styles); 9 widgets migrated crate::grid→crate::theme (SURFACE_HIGH→SURFACE_HIGHEST compiler-enforced); LIGHT_RED→DANGER; grid mute-BLACK→ON_PRIMARY; CORNER 3→Radius::SM drift fix; piano-key BLACK/WHITE kept literal (domain metaphor)|V7

## §B BUGS
id|date|cause|fix
B1|2026-07-30|deactivate latches `shutdown=true`; re-activate spawned worker saw `while !shutdown` → dead worker, drained nothing (2nd activation silent)|V13
