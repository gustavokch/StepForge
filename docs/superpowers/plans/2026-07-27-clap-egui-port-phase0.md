# StepForge CLAP egui Port — Phase 0 (Plugin Skeleton + RT Loop) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up a pure-Rust CLAP plugin (`nih_plug` + `nih_plug_egui`) that drives the existing host-driven engine, follows host transport, emits MIDI via the host note port, persists session state, and shows a minimal egui editor — validating the entire engine↔plugin↔editor loop before any real UI work.

**Architecture:** Three new workspace crates: `editor_egui` (pure egui UI lib, no `nih_plug`), `clap_plugin` (the `nih_plug::Plugin` owning `Arc<Engine>`), and `xtask` (bundler). The plugin links `sequencer_engine` directly (no FFI); `process()` maps host `Transport` → `HostTransport` (1:1, both quarter-notes), calls `Engine::render_host`, and converts each `MidiEvent` → `NoteEvent` via `send_event`. The editor shares `Arc<Engine>` and a Rust `UiState`, draining events per frame.

**Tech Stack:** Rust, `sequencer_engine` (existing core), `nih_plug` + `nih_plug_egui` + `nih_plug_xtask` (pinned git rev ≥ 2025-02-23, egui 0.31), `postcard`, `parking_lot`.

## Global Constraints

(From the approved spec `docs/superpowers/specs/2026-07-27-clap-egui-editor-design.md`. Every task implicitly includes these.)

- **macOS-only desktop** for Phase 0; the iOS build MUST stay green: `cargo check -p sequencer_engine --target aarch64-apple-ios` must pass after every task.
- **`sequencer_engine` core stays `#![forbid(unsafe_code)]` and untouched** — Phase 0 adds no core code. All new code lives in the three new crates.
- **Hard Rule 1 (RT thread is sacred):** `process()` is allocation-free, lock-free, no FFI, no `UiState`/`params.session` access. The GUI thread only writes lock-free `engine.commands` and reads `ArcSwap` snapshots. Enable `nih_plug`'s `assert_process_allocs` in debug builds.
- **Transport units are quarter-notes on both sides — no conversion.** `pos_beats()`/`bar_start_pos_beats()` map directly to `HostTransport.block_start_beat`/`bar_start_beat`.
- **Pin `nih_plug`, `nih_plug_egui`, `nih_plug_xtask` to ONE shared git rev** (≥ 2025-02-23); never float `branch = "master"`.
- **License:** no `vst3-sys`; VST3 is Phase 4 (clap-wrapper). Phase 0 ships CLAP only.
- **MIDI config:** `MIDI_INPUT = Basic`, `MIDI_OUTPUT = Basic`, `AUDIO_IO_LAYOUTS = &[]` (MIDI-only; Bitwig/Reaper accept it; Live compat is Phase 4).
- **Symbol reachability:** all `sequencer_engine::` symbols the plan references (`Command`, `EngineEvent`, `Session`, `Engine`, `host::{HostTransport,HostRenderState,MidiEvent}`, `midi_out::push_drop_oldest`, `serde_ext::{SessionEnvelope,SESSION_FORMAT_VERSION}`) are already public (consumed by `sequencer_engine_ffi`); reach them via the crate root or named module. If a path differs in your checkout, use the equivalent pub path.

---

## File Structure (created in this plan)

| File | Responsibility |
|---|---|
| `engine/crates/editor_egui/Cargo.toml` | Crate manifest; deps `sequencer_engine`, `egui = "0.31"`. |
| `engine/crates/editor_egui/src/lib.rs` | `CommandSink` trait, `render()`, `transport_action()`, `apply_theme()`. |
| `engine/crates/editor_egui/src/ui_state.rs` | `UiState` (Phase 0: `playing`, `session`). |
| `engine/crates/clap_plugin/Cargo.toml` | Crate manifest; cdylib + rlib; deps `sequencer_engine`, `stepforge_editor_egui`, `nih_plug`, `nih_plug_egui`, `postcard`, `parking_lot`. |
| `engine/crates/clap_plugin/src/lib.rs` | `StepForge` struct, `Default`, `impl Plugin`, `impl ClapPlugin`, `nih_export_clap!`. |
| `engine/crates/clap_plugin/src/params.rs` | `StepForgeParams` (`#[persist] editor_state` + `session`). |
| `engine/crates/clap_plugin/src/transport.rs` | `map_transport()` pure helper. |
| `engine/crates/clap_plugin/src/midi.rs` | `midi_event_to_note()` pure helper. |
| `engine/crates/clap_plugin/src/editor.rs` | `EngineCommandSink`, `spawn_editor()` + per-frame GUI tick. |
| `engine/crates/xtask/Cargo.toml` | Crate manifest; dep `nih_plug_xtask`. |
| `engine/crates/xtask/src/main.rs` | `fn main() { nih_plug_xtask::main() }`. |
| `engine/.cargo/config.toml` | `[alias] xtask = "run --package xtask --release --"`. |
| `engine/Cargo.toml` | **Modify:** add the three crates to `[workspace] members`. |

---

### Task 1: Workspace scaffolding + pin nih-plug

**Files:**
- Create: `engine/crates/editor_egui/{Cargo.toml,src/lib.rs}`
- Create: `engine/crates/clap_plugin/{Cargo.toml,src/lib.rs}`
- Create: `engine/crates/xtask/{Cargo.toml,src/main.rs}`
- Create: `engine/.cargo/config.toml`
- Modify: `engine/Cargo.toml` (workspace members)

**Interfaces:**
- Consumes: nothing (greenfield crates).
- Produces: three compiling workspace members; `cargo check` (host) green; iOS core still builds. The `<SHA>` used below is produced in Step 1 and reused in every nih-plug dep line.

- [ ] **Step 1: Fetch the nih-plug rev to pin**

Run: `git ls-remote https://github.com/robbert-vdh/nih-plug.git refs/heads/master`
Expected: one line like `<40-hex-SHA>\trefs/heads/master`. Copy the SHA — it is the `<SHA>` used in every nih-plug dependency line below (all three MUST use the same SHA).

- [ ] **Step 2: Create `engine/crates/editor_egui/Cargo.toml`**

```toml
[package]
name = "stepforge_editor_egui"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
path = "src/lib.rs"

[dependencies]
sequencer_engine = { path = "../core" }
egui = "0.31"
```

- [ ] **Step 3: Create `engine/crates/editor_egui/src/lib.rs` (stub)**

```rust
//! StepForge egui editor — pure UI, no nih_plug dependency.
//! Filled in across Tasks 7-8.
```

- [ ] **Step 4: Create `engine/crates/clap_plugin/Cargo.toml`**

```toml
[package]
name = "stepforge_clap"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
crate-type = ["cdylib", "rlib"]
path = "src/lib.rs"

[dependencies]
sequencer_engine = { path = "../core" }
stepforge_editor_egui = { path = "../editor_egui" }
nih_plug = { git = "https://github.com/robbert-vdh/nih-plug.git", rev = "<SHA>" }
nih_plug_egui = { git = "https://github.com/robbert-vdh/nih-plug.git", rev = "<SHA>" }
postcard = { version = "1", features = ["alloc"] }
parking_lot = "0.12"

[dev-dependencies]
serde_json = "1"
```

- [ ] **Step 5: Create `engine/crates/clap_plugin/src/lib.rs` (stub)**

```rust
//! StepForge CLAP plugin. Filled in across Tasks 4-8.
```

- [ ] **Step 6: Create `engine/crates/xtask/Cargo.toml`**

```toml
[package]
name = "xtask"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "xtask"
path = "src/main.rs"

[dependencies]
nih_plug_xtask = { git = "https://github.com/robbert-vdh/nih-plug.git", rev = "<SHA>" }
```

- [ ] **Step 7: Create `engine/crates/xtask/src/main.rs`**

```rust
fn main() -> nih_plug_xtask::Result<()> {
    nih_plug_xtask::main()
}
```

- [ ] **Step 8: Create `engine/.cargo/config.toml`**

```toml
[alias]
xtask = "run --package xtask --release --"
```

- [ ] **Step 9: Register the three crates in the workspace**

In `engine/Cargo.toml`, add `"crates/editor_egui"`, `"crates/clap_plugin"`, and `"crates/xtask"` to the existing `members = [ ... ]` array (alongside `"crates/core"`, `"crates/ffi"`).

- [ ] **Step 10: Verify host build + iOS build**

Run: `cargo check` (from `engine/`)
Expected: compiles (the stub crates are empty but valid).

Run: `cargo check -p sequencer_engine --target aarch64-apple-ios`
Expected: PASS (iOS core untouched — confirms the new desktop crates don't contaminate the iOS build).

- [ ] **Step 11: Commit**

```bash
git add engine/crates/editor_egui engine/crates/clap_plugin engine/crates/xtask engine/.cargo/config.toml engine/Cargo.toml engine/Cargo.lock
git commit -m "feat(clap): scaffold editor_egui + clap_plugin + xtask crates"
```

---

### Task 2: `map_transport` pure helper (TDD)

**Files:**
- Create: `engine/crates/clap_plugin/src/transport.rs`
- Modify: `engine/crates/clap_plugin/src/lib.rs` (add `mod transport;`)

**Interfaces:**
- Consumes: `sequencer_engine::host::HostTransport`.
- Produces: `pub fn map_transport(tempo, playing, pos_beats, bar_start_pos_beats, sample_rate, block_samples) -> HostTransport`. (Takes primitives, not `&Transport`, so it is test-constructible — `Transport`'s position accessors are `pub(crate)`.)

- [ ] **Step 1: Write the failing test**

Append to `engine/crates/clap_plugin/src/transport.rs`:

```rust
use sequencer_engine::host::HostTransport;

/// Map host transport primitives to the engine's `HostTransport`. Both sides use
/// quarter-notes for beat position, so the mapping is direct (spec §Transport).
pub fn map_transport(
    tempo: Option<f64>,
    playing: bool,
    pos_beats: Option<f64>,
    bar_start_pos_beats: Option<f64>,
    sample_rate: f32,
    block_samples: u32,
) -> HostTransport {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_fields_fall_back() {
        let t = map_transport(None, false, None, None, 48000.0, 256);
        assert_eq!(t.tempo_bpm, 120.0);
        assert_eq!(t.block_start_beat, 0.0);
        assert_eq!(t.bar_start_beat, 0.0);
        assert!(!t.is_playing);
        assert_eq!(t.sample_rate, 48000.0);
        assert_eq!(t.block_samples, 256);
        assert_eq!(t.beats_per_bar, 4.0); // engine assumes 4/4 today
    }

    #[test]
    fn some_fields_map_directly() {
        let t = map_transport(Some(140.0), true, Some(17.5), Some(16.0), 44100.0, 512);
        assert_eq!(t.tempo_bpm, 140.0);
        assert!(t.is_playing);
        assert_eq!(t.block_start_beat, 17.5);
        assert_eq!(t.bar_start_beat, 16.0);
        assert_eq!(t.sample_rate, 44100.0);
        assert_eq!(t.block_samples, 512);
    }
}
```

Add `mod transport;` to `engine/crates/clap_plugin/src/lib.rs`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p stepforge_clap transport`
Expected: FAIL (panics `not yet implemented` / `unimplemented`).

- [ ] **Step 3: Implement**

Replace the `unimplemented!()` body:

```rust
pub fn map_transport(
    tempo: Option<f64>,
    playing: bool,
    pos_beats: Option<f64>,
    bar_start_pos_beats: Option<f64>,
    sample_rate: f32,
    block_samples: u32,
) -> HostTransport {
    HostTransport {
        tempo_bpm: tempo.unwrap_or(120.0),
        sample_rate: sample_rate as f64,
        block_samples,
        block_start_beat: pos_beats.unwrap_or(0.0),
        bar_start_beat: bar_start_pos_beats.unwrap_or(0.0),
        is_playing: playing,
        beats_per_bar: 4.0,
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p stepforge_clap transport`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add engine/crates/clap_plugin/src/transport.rs engine/crates/clap_plugin/src/lib.rs
git commit -m "feat(clap): map_transport host→engine transport helper"
```

---

### Task 3: `midi_event_to_note` pure helper (TDD)

**Files:**
- Create: `engine/crates/clap_plugin/src/midi.rs`
- Modify: `engine/crates/clap_plugin/src/lib.rs` (add `mod midi;`)

**Interfaces:**
- Consumes: `sequencer_engine::host::MidiEvent`, `nih_plug::prelude::NoteEvent`.
- Produces: `pub fn midi_event_to_note(&MidiEvent) -> Option<NoteEvent<()>>`. `NoteEvent::timing` is a sample offset within the block, identical to `MidiEvent.sample_offset`.

- [ ] **Step 1: Write the failing test**

Append to `engine/crates/clap_plugin/src/midi.rs`:

```rust
use nih_plug::prelude::NoteEvent;
use sequencer_engine::host::MidiEvent;

/// Convert one engine `MidiEvent` (3-byte message + sample offset) into a host
/// `NoteEvent`. `NoteEvent::from_midi` normalizes velocity to [0,1] and treats
/// note-on-velocity-0 as NoteOff. Only NoteOn/NoteOff are forwarded.
pub fn midi_event_to_note(ev: &MidiEvent) -> Option<NoteEvent<()>> {
    let _ = ev;
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(offset: u32, status: u8, d1: u8, d2: u8) -> MidiEvent {
        MidiEvent { sample_offset: offset, status, data1: d1, data2: d2 }
    }

    #[test]
    fn note_on_maps_with_normalized_velocity() {
        let n = midi_event_to_note(&ev(10, 0x90, 60, 127)).unwrap();
        assert!(matches!(n, NoteEvent::NoteOn { timing: 10, note: 60, .. }));
        if let NoteEvent::NoteOn { velocity, .. } = n {
            assert!((velocity - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn note_off_maps() {
        let n = midi_event_to_note(&ev(20, 0x80, 60, 0)).unwrap();
        assert!(matches!(n, NoteEvent::NoteOff { timing: 20, note: 60, .. }));
    }

    #[test]
    fn note_on_velocity_zero_becomes_note_off() {
        let n = midi_event_to_note(&ev(5, 0x90, 42, 0)).unwrap();
        assert!(matches!(n, NoteEvent::NoteOff { .. }));
    }

    #[test]
    fn non_note_status_is_dropped() {
        // CC (status 0xB0) is not a NoteOn/NoteOff → None.
        assert!(midi_event_to_note(&ev(0, 0xB0, 7, 100)).is_none());
    }
}
```

Add `mod midi;` to `engine/crates/clap_plugin/src/lib.rs`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p stepforge_clap midi`
Expected: FAIL (`unimplemented`).

- [ ] **Step 3: Implement**

Replace the body:

```rust
pub fn midi_event_to_note(ev: &MidiEvent) -> Option<NoteEvent<()>> {
    let note = NoteEvent::from_midi(ev.sample_offset, &[ev.status, ev.data1, ev.data2]).ok()?;
    let is_note = matches!(note, NoteEvent::NoteOn { .. } | NoteEvent::NoteOff { .. });
    if is_note { Some(note) } else { None }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p stepforge_clap midi`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add engine/crates/clap_plugin/src/midi.rs engine/crates/clap_plugin/src/lib.rs
git commit -m "feat(clap): midi_event_to_note conversion helper"
```

---

### Task 4: `StepForgeParams` + session persistence (TDD)

**Files:**
- Create: `engine/crates/clap_plugin/src/params.rs`
- Modify: `engine/crates/clap_plugin/src/lib.rs` (add `mod params;`)

**Interfaces:**
- Consumes: `nih_plug::params::Params`, `nih_plug_egui::EguiState`.
- Produces: `pub struct StepForgeParams` with two `#[persist]` fields: `editor_state: Arc<EguiState>`, `session: Arc<RwLock<Vec<u8>>>` (postcard `SessionEnvelope` bytes).

- [ ] **Step 1: Write the failing test**

Append to `engine/crates/clap_plugin/src/params.rs`:

```rust
use nih_plug::params::Params;
use nih_plug_egui::EguiState;
use parking_lot::RwLock;
use std::sync::Arc;

/// All non-parameter plugin state. Zero automation params — StepForge is driven
/// via `Command`s; `Params` carries only persisted fields.
#[derive(Params)]
pub struct StepForgeParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    /// Postcard `SessionEnvelope` bytes, serialized fresh on the GUI thread.
    #[persist = "session"]
    pub session: Arc<RwLock<Vec<u8>>>,
}

impl Default for StepForgeParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(900, 600),
            session: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_constructs() {
        let p = StepForgeParams::default();
        assert!(p.session.read().is_empty());
    }

    #[test]
    fn session_bytes_round_trip_as_json() {
        // #[persist] serializes via serde_json; a bare Vec<u8> becomes a JSON
        // number array and must round-trip byte-for-byte.
        let p = StepForgeParams::default();
        let payload = vec![0xDEu8, 0xAD, 0xBE, 0xEF];
        *p.session.write() = payload.clone();
        let s = serde_json::to_string(&*p.session.read()).unwrap();
        let back: Vec<u8> = serde_json::from_str(&s).unwrap();
        assert_eq!(back, payload);
    }
}
```

Add `mod params;` to `engine/crates/clap_plugin/src/lib.rs`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p stepforge_clap params`
Expected: FAIL (compile error: `Params` derive / `#[persist]` — these resolve once the crate compiles; if it compiles, the test runs and passes).

- [ ] **Step 3: Verify against the pinned example**

Run: `cargo test -p stepforge_clap params`
If there is a compile error on the `Params` derive or `EguiState` API, open the pinned nih-plug example and match its incantation:

```bash
git clone --filter=blob:none --sparse https://github.com/robbert-vdh/nih-plug.git /tmp/nih-plug-ref 2>/dev/null || true
git -C /tmp/nih-plug-ref sparse-checkout set plugins/examples/gain_gui_egui 2>/dev/null || true
git -C /tmp/nih-plug-ref checkout <SHA> 2>/dev/null || true
```
Then read `/tmp/nih-plug-ref/plugins/examples/gain_gui_egui/src/lib.rs` and align: the `#[derive(Params)]` field attributes and `EguiState::from_size` usage. Adjust `params.rs` to match.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p stepforge_clap params`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add engine/crates/clap_plugin/src/params.rs engine/crates/clap_plugin/src/lib.rs
git commit -m "feat(clap): StepForgeParams with persisted editor-state + session"
```

---

### Task 5: `StepForge` Plugin struct, `Default`, constants, `ClapPlugin`

**Files:**
- Modify: `engine/crates/clap_plugin/src/lib.rs` (replace stub with full module)

**Interfaces:**
- Consumes: `sequencer_engine::{Engine, host::HostRenderState}`, `stepforge_editor_egui::UiState`, `crate::params::StepForgeParams`.
- Produces: `pub struct StepForge` (fields: `engine`, `host_render_state`, `sample_rate`, `params`, `worker_handle`, `ui_state`); `impl Default`; `impl Plugin` (constants + `params()`; `initialize`/`process`/`deactivate`/`editor` filled in later tasks); `impl ClapPlugin`; `nih_export_clap!(StepForge)`.

- [ ] **Step 1: Write the struct, Default, and test first**

Replace `engine/crates/clap_plugin/src/lib.rs` with:

```rust
//! StepForge CLAP plugin (Phase 0 skeleton).

mod midi;
mod params;
mod transport;
// `editor` module added in Task 8.

use nih_plug::prelude::*;
use parking_lot::{Mutex, RwLock};
use std::sync::Arc;

use params::StepForgeParams;
use sequencer_engine::host::HostRenderState;
use sequencer_engine::Engine;
use stepforge_editor_egui::UiState;

pub struct StepForge {
    engine: Arc<Engine>,
    host_render_state: HostRenderState,
    sample_rate: f32,
    params: Arc<StepForgeParams>,
    /// State-worker JoinHandle, spawned in `initialize`, joined in `deactivate`.
    worker_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// GUI mirror; cloned into the editor closure in Task 8.
    ui_state: Arc<RwLock<UiState>>,
}

impl Default for StepForge {
    fn default() -> Self {
        Self {
            engine: Arc::new(Engine::new_host_driven()),
            host_render_state: HostRenderState::new(),
            sample_rate: 48000.0,
            params: Arc::new(StepForgeParams::default()),
            worker_handle: Mutex::new(None),
            ui_state: Arc::new(RwLock::new(UiState::default())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_constructs() {
        let p = StepForge::default();
        assert_eq!(p.sample_rate, 48000.0);
        let _ = p.params.clone();
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p stepforge_clap default_constructs`
Expected: FAIL (compile error: `Plugin` not implemented yet, so `StepForge` isn't a valid plugin — but the struct + Default + test should compile. If the only error is the missing `impl Plugin`, proceed; this task adds it next.)

- [ ] **Step 3: Add `impl Plugin` (constants + `params()`; lifecycle bodies deferred)**

Append to `engine/crates/clap_plugin/src/lib.rs`:

```rust
impl Plugin for StepForge {
    const NAME: &'static str = "StepForge";
    const VENDOR: &'static str = "StepForge";
    const URL: &'static str = "https://github.com/gus/StepForge";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[]; // MIDI-only (Phase 0)
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, _buffer_config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        // Filled in Task 6.
        true
    }

    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        // Filled in Task 6.
        ProcessStatus::Normal
    }

    fn deactivate(&mut self) {
        // Filled in Task 6.
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        None // Filled in Task 8.
    }
}
```

- [ ] **Step 4: Add `impl ClapPlugin` + the export macro**

Append to `engine/crates/clap_plugin/src/lib.rs`:

```rust
impl ClapPlugin for StepForge {
    const CLAP_ID: &'static str = "org.stepforge.clap";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("MIDI drum sequencer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [&'static str] = &["midi-effect", "note-effect"];
}

nih_export_clap!(StepForge);
```

- [ ] **Step 5: Verify it compiles; align with the pinned example if the const set differs**

Run: `cargo check -p stepforge_clap`
If a const type/name mismatches the pinned rev (e.g. `EMAIL` is `Option<&str>`, or `ClapPlugin` has a different const set), read `/tmp/nih-plug-ref/plugins/examples/gain/src/lib.rs` (the `impl ClapPlugin` block) and `src/plugin.rs` and adjust the consts to match. Re-run until `cargo check -p stepforge_clap` passes.

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p stepforge_clap default_constructs`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add engine/crates/clap_plugin/src/lib.rs
git commit -m "feat(clap): StepForge Plugin + ClapPlugin skeleton"
```

---

### Task 6: `process()`, `initialize()` (worker + restore), `deactivate()`

**Files:**
- Modify: `engine/crates/clap_plugin/src/lib.rs` (fill the three lifecycle methods)

**Interfaces:**
- Consumes: `crate::transport::map_transport`, `crate::midi::midi_event_to_note`, `Engine::render_host`, `sequencer_engine::midi_out::push_drop_oldest`, `Command::LoadSession`.
- Produces: a working `process()` RT path; worker spawn/restore in `initialize()`; clean shutdown in `deactivate()`.

- [ ] **Step 1: Fill in `initialize()` (cache sample rate, spawn worker, restore session)**

Replace the `initialize` body:

```rust
    fn initialize(&mut self, _layout: &AudioIOLayout, buffer_config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        self.sample_rate = buffer_config.sample_rate;

        // Host-driven mode does NOT auto-spawn the state worker — do it once here.
        let mut wh = self.worker_handle.lock();
        if wh.is_none() {
            let e = Arc::clone(&self.engine);
            *wh = Some(
                std::thread::Builder::new()
                    .name("stepforge-worker".into())
                    .spawn(move || e.run_worker_loop())
                    .expect("spawn state-worker"),
            );
        }
        drop(wh);

        // State is deserialized into params.session BEFORE initialize; restore it.
        let bytes = self.params.session.read().clone();
        if !bytes.is_empty() {
            let _ = sequencer_engine::midi_out::push_drop_oldest(
                &self.engine.commands,
                sequencer_engine::Command::LoadSession { bytes },
            );
        }
        true
    }
```

- [ ] **Step 2: Fill in `process()` (the RT path)**

Replace the `process` body:

```rust
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let tr = context.transport();
        let transport = transport::map_transport(
            tr.tempo,
            tr.playing,
            tr.pos_beats(),
            tr.bar_start_pos_beats(),
            self.sample_rate,
            buffer.samples() as u32,
        );

        // Stack-allocated, fixed-size output buffer. MidiEvent: Copy.
        let mut midi_out: [sequencer_engine::host::MidiEvent; 1024] =
            [sequencer_engine::host::MidiEvent::zero(); 1024];

        let n = self.engine.render_host(&mut self.host_render_state, &transport, &[], &mut midi_out);

        for ev in &midi_out[..n] {
            if let Some(note) = midi::midi_event_to_note(ev) {
                context.send_event(note);
            }
        }

        ProcessStatus::Normal
    }
```

- [ ] **Step 3: Fill in `deactivate()` (shut down + join the worker)**

Replace the `deactivate` body:

```rust
    fn deactivate(&mut self) {
        self.engine.shutdown.store(std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.worker_handle.lock().take() {
            let _ = handle.join();
        }
        // Reset render state so a possible re-initialize starts clean.
        self.host_render_state = HostRenderState::new();
    }
```

- [ ] **Step 4: Enable alloc-assertions on the RT path (debug builds)**

In `engine/crates/clap_plugin/Cargo.toml`, change the `nih_plug` dependency to enable the feature:

```toml
nih_plug = { git = "https://github.com/robbert-vdh/nih-plug.git", rev = "<SHA>", features = ["assert_process_allocs"] }
```

(This aborts if `process()` allocates in debug/test builds. It is a no-op guard in normal operation; revisit gating for release in a later task.)

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p stepforge_clap`
Expected: PASS. (The pure helpers `map_transport` and `midi_event_to_note` are unit-tested in Tasks 2-3; the `process()` integration is exercised manually in Task 9.)

- [ ] **Step 6: Run all clap_plugin tests**

Run: `cargo test -p stepforge_clap`
Expected: PASS (transport, midi, params, default_constructs).

- [ ] **Step 7: Commit**

```bash
git add engine/crates/clap_plugin/src/lib.rs engine/crates/clap_plugin/Cargo.toml
git commit -m "feat(clap): process()/initialize()/deactivate() RT loop + worker lifecycle"
```

---

### Task 7: Minimal `editor_egui` (BPM + play) (TDD)

**Files:**
- Create: `engine/crates/editor_egui/src/ui_state.rs`
- Modify: `engine/crates/editor_egui/src/lib.rs` (replace stub)

**Interfaces:**
- Consumes: `sequencer_engine::{Command, Session}`.
- Produces: `pub trait CommandSink { fn push(&self, cmd: Command); }`, `pub struct UiState { playing, session }`, `pub fn transport_action(playing: bool) -> Command`, `pub fn render(ctx, &UiState, &impl CommandSink)`, `pub fn apply_theme(&egui::Context)`.

- [ ] **Step 1: Create `engine/crates/editor_egui/src/ui_state.rs`**

```rust
//! GUI mirror of engine state (Phase 0 subset; Phase 1 ports the full
//! SessionMirror apply logic).
use sequencer_engine::Session;
use std::sync::Arc;

#[derive(Default)]
pub struct UiState {
    pub playing: bool,
    pub session: Option<Arc<Session>>,
}
```

- [ ] **Step 2: Write the failing tests in `engine/crates/editor_egui/src/lib.rs`**

```rust
//! StepForge egui editor — pure UI, no nih_plug dependency.

pub mod ui_state;
pub use ui_state::UiState;

use egui::Context;
use sequencer_engine::Command;

/// Sink for commands emitted by UI interactions.
pub trait CommandSink {
    fn push(&self, cmd: Command);
}

/// Pure: which transport command a play/stop toggle emits given current state.
pub fn transport_action(playing: bool) -> Command {
    unimplemented!()
}

/// Dark graphite theme (Phase 0 minimal; full palette in Phase 4).
pub fn apply_theme(ctx: &Context) {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(egui::Color32::WHITE);
    ctx.set_visuals(v);
}

/// Render the Phase 0 editor: BPM readout (from snapshot) + play/stop toggle.
pub fn render(ctx: &Context, ui_state: &UiState, sink: &impl CommandSink) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let bpm = ui_state.session.as_ref().map(|s| s.bpm).unwrap_or(120.0);
        ui.heading(format!("StepForge — {:.1} BPM", bpm));
        if ui.button(if ui_state.playing { "■ Stop" } else { "▶ Play" }).clicked() {
            sink.push(transport_action(ui_state.playing));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<Command>>);
    impl CommandSink for RecordingSink {
        fn push(&self, cmd: Command) { self.0.lock().unwrap().push(cmd); }
    }

    #[test]
    fn transport_action_toggles() {
        assert!(matches!(transport_action(false), Command::Play));
        assert!(matches!(transport_action(true), Command::Stop));
    }

    #[test]
    fn render_does_not_panic() {
        let ctx = Context::default();
        let sink = RecordingSink::default();
        let mut state = UiState::default();
        render(&ctx, &state, &sink);
        state.session = Some(Arc::new(sequencer_engine::Session::default()));
        render(&ctx, &state, &sink);
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p stepforge_editor_egui`
Expected: FAIL (`transport_action_toggles` panics `unimplemented`).

- [ ] **Step 4: Implement `transport_action`**

Replace the `unimplemented!()` body:

```rust
pub fn transport_action(playing: bool) -> Command {
    if playing { Command::Stop } else { Command::Play }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p stepforge_editor_egui`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add engine/crates/editor_egui/src/lib.rs engine/crates/editor_egui/src/ui_state.rs
git commit -m "feat(editor-egui): minimal UiState + render + CommandSink"
```

---

### Task 8: `editor()` wiring + per-frame GUI tick

**Files:**
- Create: `engine/crates/clap_plugin/src/editor.rs`
- Modify: `engine/crates/clap_plugin/src/lib.rs` (add `mod editor;`, fill `editor()`)

**Interfaces:**
- Consumes: `stepforge_editor_egui::{render, apply_theme, UiState, CommandSink}`, `nih_plug_egui::create_egui_editor`, `sequencer_engine::{EngineEvent, serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION}}`, `Engine::{snapshot_arc, commands, hot_events, large_events}`.
- Produces: `pub fn spawn_editor(...) -> Option<Box<dyn Editor>>`; `StepForge::editor()` returns it.

- [ ] **Step 1: Create `engine/crates/clap_plugin/src/editor.rs`**

```rust
//! Editor spawn + per-frame GUI tick: drain RT→GUI events into UiState,
//! throttle-refresh the authoritative snapshot + serialize-for-save, then render.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, EguiState};
use parking_lot::{RwLock, RwLockReadGuard};

use sequencer_engine::midi_out::push_drop_oldest;
use sequencer_engine::serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION};
use sequencer_engine::{Command, Engine, EngineEvent};
use stepforge_editor_egui::{apply_theme, render, CommandSink, UiState};

pub struct EngineCommandSink {
    pub engine: Arc<Engine>,
}
impl CommandSink for EngineCommandSink {
    fn push(&self, cmd: Command) {
        let _ = push_drop_oldest(&self.engine.commands, cmd);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_editor(
    engine: Arc<Engine>,
    ui_state: Arc<RwLock<UiState>>,
    session: Arc<RwLock<Vec<u8>>>,
    egui_state: Arc<EguiState>,
) -> Option<Box<dyn Editor>> {
    let frame = Arc::new(AtomicU64::new(0));
    create_egui_editor(
        egui_state,
        (),
        |_ctx, _user_state: &mut ()| {
            // build closure runs once before the first frame.
        },
        move |ctx, _setter, _user_state: &mut ()| {
            let n = frame.fetch_add(1, Ordering::Relaxed) + 1;
            tick(&engine, &ui_state, &session, n);
            apply_theme(ctx);
            let st: RwLockReadGuard<'_, UiState> = ui_state.read();
            render(ctx, &st, &EngineCommandSink { engine: engine.clone() });
        },
    )
}

fn tick(engine: &Arc<Engine>, ui_state: &Arc<RwLock<UiState>>, session: &Arc<RwLock<Vec<u8>>>, frame: u64) {
    {
        let mut st = ui_state.write();
        // Hot channel: small fixed-slot events (Phase 0: just PlayStateChanged).
        while let Some(slot) = engine.hot_events.dequeue() {
            if let Ok(ev) = postcard::from_bytes::<EngineEvent>(&slot.bytes[..slot.len as usize]) {
                if let EngineEvent::PlayStateChanged { playing } = ev {
                    st.playing = playing;
                }
                // Remaining variants are ported in Phase 1.
            }
        }
        // Large channel: discard for Phase 0 (Serialized/FullSnapshot handled via snapshot_arc below).
        while engine.large_events.dequeue().is_some() {}

        // Throttled authoritative snapshot refresh + serialize-for-save (~1 Hz at 60 fps).
        if frame % 60 == 0 {
            let snap = engine.snapshot_arc();
            st.session = Some(snap.clone());
            let env = SessionEnvelope {
                version: SESSION_FORMAT_VERSION,
                session: (*snap).clone(),
            };
            if let Ok(bytes) = postcard::to_allocvec(&env) {
                *session.write() = bytes;
            }
        }
    }
}
```

Add `mod editor;` near the top of `engine/crates/clap_plugin/src/lib.rs` (with the other `mod` declarations).

- [ ] **Step 2: Fill in `StepForge::editor()`**

In `engine/crates/clap_plugin/src/lib.rs`, replace the `editor` body:

```rust
    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::spawn_editor(
            Arc::clone(&self.engine),
            Arc::clone(&self.ui_state),
            Arc::clone(&self.params.session),
            self.params.editor_state.clone(),
        )
    }
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p stepforge_clap`
If the `create_egui_editor` closure signature or `EguiState` usage differs from the pinned rev, read `/tmp/nih-plug-ref/plugins/examples/gain_gui_egui/src/lib.rs` and align (closure arity, user-state type, whether `build` takes `&mut T`). Re-run until it compiles.

- [ ] **Step 4: Run all tests**

Run: `cargo test -p stepforge_clap && cargo test -p stepforge_editor_egui`
Expected: PASS (all prior tests still green).

- [ ] **Step 5: Commit**

```bash
git add engine/crates/clap_plugin/src/editor.rs engine/crates/clap_plugin/src/lib.rs
git commit -m "feat(clap): wire egui editor + per-frame GUI tick"
```

---

### Task 9: Bundle + verify in a host

**Files:** none (build/verification task; no source changes unless verification finds a defect).

**Interfaces:**
- Consumes: the complete Phase 0 plugin from Tasks 1-8.
- Produces: a loadable `.clap` bundle validated in `clap-validator` and Bitwig/Reaper.

- [ ] **Step 1: Build the CLAP bundle**

Run: `cargo xtask bundle stepforge_clap --release`
Expected: a `.clap` bundle is produced under `engine/target/bundled/` (note the exact path from the xtask output). If the bundle name differs (e.g. `StepForge.clap`), use that.

- [ ] **Step 2: Run clap-validator (if installed)**

Install if needed: `cargo install clap-validator` (skip if already present).
Run: `clap-validator run <path-to-bundle>` (or `clap-validator validate <path>`)
Expected: no critical/severe failures on the plugin's declared capabilities. Investigate and fix any defect surfaced (return to the relevant task).

- [ ] **Step 3: Install the bundle where your DAW scans, then verify in Bitwig**

Copy (or symlink) the `.clap` into your Bitwig CLAP scan folder, then in Bitwig:
- Add "StepForge" as a MIDI insert / instrument FX on a track whose output feeds a drum instrument.
- Start transport → drum notes follow the playhead (default session plays the seed pattern).
- Change host tempo → note rate tracks it.
- Stop transport → notes stop (all-notes-off).
- Save the project, reopen it → the session is restored (BPM/pattern preserved).

Expected: all four behaviors hold. If MIDI is silent, confirm the track routes the plugin's MIDI out to an instrument and that `MIDI_OUTPUT = Basic` is in effect.

- [ ] **Step 4: Confirm the editor opens**

In the same Bitwig session, open the StepForge editor window.
Expected: a small dark window showing "StepForge — <BPM>" and a ▶ Play button; clicking it pushes a `Command::Play` (cosmetic in Phase 0 — host transport still owns actual playback; full transport reflection lands in Phase 1).

- [ ] **Step 5: Re-run the full test suite + iOS guard**

Run: `cargo test --workspace` (host)
Expected: PASS (all crates).

Run: `cargo check -p sequencer_engine --target aarch64-apple-ios`
Expected: PASS (iOS core untouched by Phase 0).

- [ ] **Step 6: Record Phase 0 completion**

No source changes are expected from this task (build artifacts are not committed). If verification surfaced fixes, those were committed in the relevant task. Mark Phase 0 done.

---

## Verification (Phase 0 exit criteria)

- `cargo test --workspace` passes (host): `map_transport`, `midi_event_to_note`, `StepForgeParams`, `StepForge::default`, `editor_egui` smoke + `transport_action`.
- `cargo check -p sequencer_engine --target aarch64-apple-ios` passes (iOS uncontaminated).
- `cargo xtask bundle stepforge_clap --release` produces a `.clap`.
- The plugin loads in Bitwig/Reaper, follows transport, emits MIDI, and round-trips project state.
- `assert_process_allocs` does not fire in debug.

## Notes / deferrals (explicit, not gaps)

- **MIDI input consumption** is wired later: `MIDI_INPUT = Basic` is set, but `process()` passes `&[]` in Phase 0. A later phase reads `context.next_event()` into the `midi_in` slice for pattern-select (already handled by `render_host`, engine.rs:540-551).
- **Audio bus for Ableton Live** (`AUDIO_IO_LAYOUTS` dummy 2×2): deliberately deferred from the spec's Phase 0 wording to Phase 4. `AUDIO_IO_LAYOUTS = &[]` (MIDI-only) compiles cleanly and validates the loop in Bitwig/Reaper; the dummy bus is only needed for Live, and its exact `AudioIOLayout` shape will be verified alongside Live/VST3 testing in Phase 4.
- **Full `UiState.apply_*`** (the complete `SessionMirror` port) is Phase 1.
- **Continuous ~60 Hz redraw** while the editor is open is accepted for v1 (nih_plug_egui calls `request_repaint()` unconditionally).
- **VST3 (clap-wrapper) + signing/notarization** is Phase 4.
