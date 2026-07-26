# StepForge Plugin — Phase 0: Rust Host-Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a host-driven mode to `sequencer_engine` so a plugin host's render callback can drive the existing dispatch core sample-accurately, with no new threads on the RT path.

**Architecture:** A new `engine_render` C entry drives the reused, pure `process()` core once per 16th-note boundary that crosses a host audio block. Per-instance mutable render state (RT step accumulator + a fixed-size scheduled-MIDI queue that defers note-offs whose gate spans a block boundary *and* swung note-ons pushed past one — sample-accurate, never clamped — and is cleared on transport stop) lives in a caller-owned `HostRenderState` handle, so `Engine` stays `Send+Sync` with no locks, no `UnsafeCell`, and no `unsafe` in core. Incoming MIDI maps to existing `QueuePattern` commands (pattern select). A runtime `host_driven` flag makes `engine_start` spawn only the state worker. The standalone app path (`engine_new`/`engine_start`) is unchanged.

**Tech Stack:** Rust 2021, `heapless` (existing lock-free ring), `arc_swap` (existing COW snapshot), `proptest`, cbindgen (header regen). No new dependencies.

## Global Constraints

- `engine/crates/core` is `#![forbid(unsafe_code)]` — all `unsafe` stays in `engine/crates/ffi`.
- **Hard Rule 1 (RT is sacred):** `engine_render` runs on the host RT thread — no allocations, no locks, no `Vec`/`String`/`format!`, no FFI out, no CoreMIDI, no Link. Reuse only `process()` (already RT-safe) + the lock-free `MidiOutRing` + a non-blocking `arc_swap` snapshot read + fixed-size arrays.
- **Additive only:** the standalone app (`engine_new` + self-scheduled `engine_start`) must build and behave unchanged. iOS (`cargo check --target aarch64-apple-ios`) must still build.
- **No new `EngineResult` discriminants** — discriminants are stable across releases. Reuse `ErrOther`/`ErrInvalidHandle`/`ErrInvalidBuffer`.
- **No new `Command` variant** for pattern-select-by-note — reuse `Command::QueuePattern { index, quantize }`.
- `engine/include/sequencer_engine.h` is committed and cbindgen-generated; regenerate it after adding Rust types/entries and commit the result.
- Run engine commands from `engine/`. The build env needs the rustup toolchain: `export PATH="$HOME/.cargo/bin:$PATH"` (Homebrew `rustc` can't cross-compile to iOS).

## Scope boundary (explicitly deferred)

- **Ableton Link network construction** (`ExternalClock::new()` builds a real Link + tokio runtime on macOS). In host-driven mode the Link poller is simply not spawned, but `Engine::new()` still constructs an idle Link. Fully avoiding Link network activity inside a DAW is a **Phase 3** (plugin-bundle) concern — irrelevant for Phase 0 host tests. Do **not** feature-gate Link in this plan.
- Plugin wrappers (AUv3 / CLAP / VST3), the Swift editor bundle, and the `stepforge_create_editor_view` seam are Phases 1–4.

## Deviations from the plugin-port design spec (intentional)

These refine the Phase 0 slice of `docs/superpowers/specs/2026-07-26-plugin-port-design.md` without changing its intent:

- **One `MidiEvent` for both directions.** The design's C signature names separate `IncomingMidi` / `OutgoingMidi` types; the plan unifies them into a single `MidiEvent` (same 4-field payload both ways). DRY — the design's prose already permits plain POD structs of MIDI bytes + sample offsets.
- **Accumulator state in `HostRenderState`, not `RtState`.** The design text says "new fractional-16th state in `RtState`"; the plan keeps `RtState` untouched and holds `next_step_beat` / `sample_time` / `pending` in the caller-owned `HostRenderState`. This keeps `RtState` reusable and single-owner (no synchronization) and lets `process_one` be reused unchanged — strictly better.
- **Runtime `host_driven` flag, not a cargo feature.** The design allows either ("a cargo feature … or a runtime construction flag"). The runtime flag keeps a single build matrix and leaves the standalone path bit-for-bit identical; feature-gating Link network activity inside a DAW stays a Phase 3 concern.

## File Structure

- **Create** `engine/crates/core/src/host.rs` — host-driven data types + `Engine::render_host` state. POD types `HostTransport`, `MidiEvent` are `#[repr(C)]` (plain scalar structs — the FFI rule forbids data-carrying `#[repr(C)]` *enums*, not structs).
- **Modify** `engine/crates/core/src/lib.rs` — register `pub mod host;`.
- **Modify** `engine/crates/core/src/engine.rs` — add `host_driven: bool` field + `Engine::new_host_driven()`; `render_host` is an `impl Engine` method there so it can call the private `process_one`.
- **Modify** `engine/crates/ffi/src/handle.rs` — `RenderStateHandle` opaque type + `new_host_handle` + `new_render_state`/`free_render_state`.
- **Modify** `engine/crates/ffi/src/lib.rs` — re-export host types; `engine_start` branches on `host_driven`; add `engine_new_host_driven`, `engine_render_state_new`, `engine_render`, `engine_render_state_free`.
- **Regenerate** `engine/include/sequencer_engine.h`.
- **Create** `engine/crates/core/tests/host_render.rs` — unit + `proptest` invariants for `render_host`.
- **Create** `engine/crates/ffi/tests/host_render_api.rs` — C-ABI round-trip for the new entries.

---

### Task 1: Host-driven data types + render-state scaffold (`core/src/host.rs`)

**Files:**
- Create: `engine/crates/core/src/host.rs`
- Modify: `engine/crates/core/src/lib.rs` (add `pub mod host;`)
- Test: `engine/crates/core/src/host.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Produces: `#[repr(C)] pub struct HostTransport`, `#[repr(C)] pub struct MidiEvent`, `pub struct HostRenderState`, `pub struct PendingMidiQueue` (wraps a private `PendingMidiEvent` slot), `const PENDING_OFF_DEPTH: usize = 64`.

- [ ] **Step 1: Write the failing test**

Append to `engine/crates/core/src/host.rs` (after the impls below exist — write the test first, then the types, in this same file edit):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_queue_schedules_and_drains_due_only() {
        let mut q = PendingMidiQueue::new();
        // Two events scheduled in the future: a note-off and a deferred note-on.
        q.schedule(1_000, 0x80, 36, 0);
        q.schedule(2_000, 0x9A, 38, 100);
        let mut emitted = Vec::new();
        q.drain_due(0, 1_500, |ev| emitted.push(ev));
        // Only the abs=1_000 one is due within [0,1_500).
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].sample_offset, 1_000);
        assert_eq!(emitted[0].data1, 36);
        // Draining again in a later block picks up the survivor — with its data2.
        emitted.clear();
        q.drain_due(1_500, 3_000, |ev| emitted.push(ev));
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].data1, 38);
        assert_eq!(emitted[0].data2, 100, "a deferred note-on keeps its velocity");
        // Fully drained.
        emitted.clear();
        q.drain_due(3_000, 4_000, |_| panic!("no more"));
        assert!(emitted.is_empty());
    }

    #[test]
    fn clear_drops_all_scheduled_events() {
        // Transport stop emits CC 123 all-notes-off, then clears the queue so no
        // stale events (note-off OR deferred note-on) fire for notes the host
        // already killed.
        let mut q = PendingMidiQueue::new();
        q.schedule(1_000, 0x80, 36, 0);
        q.schedule(2_000, 0x9A, 38, 100);
        q.clear();
        let mut emitted = Vec::new();
        q.drain_due(0, 10_000, |ev| emitted.push(ev));
        assert!(emitted.is_empty(), "clear drops all scheduled events");
    }

    #[test]
    fn host_render_state_default_is_stopped_and_uninitialized() {
        let rs = HostRenderState::new();
        assert!(!rs.was_playing);
        assert!(!rs.initialized);
        assert_eq!(rs.sample_time, 0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd engine && cargo test -p sequencer_engine host::tests -- --nocapture`
Expected: FAIL — `unresolved module host` (module not declared yet) / types not defined.

- [ ] **Step 3: Write minimal implementation**

Add to `engine/crates/core/src/lib.rs` (after `pub mod engine;`):

```rust
pub mod host;
```

Create `engine/crates/core/src/host.rs`:

```rust
//! Host-driven render types. The plugin host calls `engine_render` once per
//! audio block on its RT thread; these POD structs cross that C ABI. Plain
//! scalar structs (not data-carrying enums) — `#[repr(C)]` is allowed.
//!
//! `Engine::render_host` (in `engine.rs`, same impl block as `process_one`)
//! consumes a `HostRenderState` owned by the caller, keeping `Engine` lock-free
//! and `Send+Sync` without `UnsafeCell`/`unsafe` in core.

use crate::engine::RtState;

/// Host transport snapshot for one audio block. Filled by the plugin wrapper
/// from AU `musicalContextBlock`/`transportStateBlock` or nih_plug `Transport`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HostTransport {
    /// Tempo in beats-per-minute.
    pub tempo_bpm: f64,
    /// Audio sample rate (Hz).
    pub sample_rate: f64,
    /// Number of samples in this block.
    pub block_samples: u32,
    /// Absolute host beat position at the first sample of this block.
    pub block_start_beat: f64,
    /// Beat position of the current bar's downbeat (≤ `block_start_beat`).
    pub bar_start_beat: f64,
    /// Host transport playing state.
    pub is_playing: bool,
    /// Beats per bar (time-signature numerator, e.g. 4.0 for 4/4). Reserved for
    /// non-4/4 support in a later phase; the Phase 0 accumulator assumes 4/4
    /// (four 16ths per beat) and does not yet read this field. Included now so
    /// the committed header needs no ABI-breaking field addition later.
    pub beats_per_bar: f64,
}

/// One 3-byte MIDI message with a sample offset within the current block. Used
/// for both host → engine input and engine → host output.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiEvent {
    /// Sample offset within the block, in `[0, block_samples)`.
    pub sample_offset: u32,
    /// Full status byte including channel (e.g. `0x90 | ch`).
    pub status: u8,
    /// MIDI data byte 1 (note / controller).
    pub data1: u8,
    /// MIDI data byte 2 (velocity / value).
    pub data2: u8,
}

impl MidiEvent {
    pub const fn zero() -> Self {
        Self { sample_offset: 0, status: 0, data1: 0, data2: 0 }
    }
}

/// Maximum simultaneous deferred MIDI events. A note's gate (default 50 ms)
/// often spans several audio blocks, and a high-swing note-on can land past the
/// block that fired its boundary. Both are held here until due. Bounded → RT-safe.
pub const PENDING_OFF_DEPTH: usize = 64;

#[derive(Clone, Copy)]
struct PendingMidiEvent {
    abs_sample: u64,
    status: u8,
    data1: u8,
    data2: u8,
    active: bool,
}

/// Fixed-size, single-threaded (host-RT-owner) scheduled-MIDI queue. Holds both
/// deferred note-offs (gate spans past the block) and deferred note-ons (swing
/// pushes the note past the block that fired its boundary). No locks, no
/// allocation — `render_host` is the only accessor.
pub struct PendingMidiQueue {
    slots: [PendingMidiEvent; PENDING_OFF_DEPTH],
}

impl PendingMidiQueue {
    pub fn new() -> Self {
        Self {
            slots: [PendingMidiEvent { abs_sample: 0, status: 0, data1: 0, data2: 0, active: false };
                PENDING_OFF_DEPTH],
        }
    }

    /// Schedule a 3-byte MIDI message at absolute sample time. Finds an inactive
    /// slot; if none, evicts the slot with the largest `abs_sample` (drop-furthest).
    pub fn schedule(&mut self, abs_sample: u64, status: u8, data1: u8, data2: u8) {
        let mut victim = 0usize;
        let mut victim_abs = u64::MIN;
        for (i, s) in self.slots.iter_mut().enumerate() {
            if !s.active {
                s.active = true;
                s.abs_sample = abs_sample;
                s.status = status;
                s.data1 = data1;
                s.data2 = data2;
                return;
            }
            if s.abs_sample > victim_abs {
                victim_abs = s.abs_sample;
                victim = i;
            }
        }
        // Full — evict the furthest-future slot if this one is sooner.
        if abs_sample < self.slots[victim].abs_sample {
            self.slots[victim] = PendingMidiEvent { abs_sample, status, data1, data2, active: true };
        }
    }

    /// Emit events whose `abs_sample` falls in `[block_start_abs, block_end_abs)`
    /// as `MidiEvent`s (offset relative to `block_start_abs`) and deactivate them.
    pub fn drain_due(
        &mut self,
        block_start_abs: u64,
        block_end_abs: u64,
        mut out: impl FnMut(MidiEvent),
    ) {
        for s in self.slots.iter_mut() {
            if s.active && s.abs_sample >= block_start_abs && s.abs_sample < block_end_abs {
                out(MidiEvent {
                    sample_offset: (s.abs_sample - block_start_abs) as u32,
                    status: s.status,
                    data1: s.data1,
                    data2: s.data2,
                });
                s.active = false;
            }
        }
    }

    /// Drop every scheduled event. Called on transport stop after CC 123
    /// all-notes-off — the host killed the notes, so individual events still
    /// queued (note-offs or deferred note-ons) are stale and must not fire.
    pub fn clear(&mut self) {
        for s in self.slots.iter_mut() {
            s.active = false;
        }
    }
}

impl Default for PendingMidiQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-instance host-render state, owned by the plugin wrapper (one per engine
/// handle) and passed to `Engine::render_host` each block. Persistent across
/// blocks; single-owner → no synchronization needed.
pub struct HostRenderState {
    /// Reused RT tick state (per-track step indices, RNG, bar position).
    pub rt: RtState,
    pub pending: PendingMidiQueue,
    /// Beat position of the next 16th-step boundary to fire.
    pub next_step_beat: f64,
    /// Absolute sample time at the start of the next block to render.
    pub sample_time: u64,
    /// Last seen `block_start_beat` (seek/discontinuity detection).
    pub last_block_start_beat: f64,
    pub was_playing: bool,
    pub initialized: bool,
}

impl HostRenderState {
    pub fn new() -> Self {
        Self {
            rt: RtState::new(1), // reseeded by `begin_play` on play-start
            pending: PendingMidiQueue::new(),
            next_step_beat: 0.0,
            sample_time: 0,
            last_block_start_beat: f64::NAN,
            was_playing: false,
            initialized: false,
        }
    }
}

impl Default for HostRenderState {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd engine && cargo test -p sequencer_engine host::tests -- --nocapture`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git -C /Users/gus/Git/StepForge add engine/crates/core/src/host.rs engine/crates/core/src/lib.rs
git -C /Users/gus/Git/StepForge commit -m "feat(core): host-driven render types + HostRenderState (Phase 0)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: `Engine::new_host_driven` + `host_driven` flag

**Files:**
- Modify: `engine/crates/core/src/engine.rs` (struct field + constructors)
- Test: `engine/crates/core/src/engine.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: none.
- Produces: `Engine::host_driven: bool`, `Engine::new_host_driven() -> Engine`. `Engine::new()` keeps `host_driven == false` (standalone unchanged).

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `engine/crates/core/src/engine.rs`:

```rust
    #[test]
    fn host_driven_flag_reflects_constructor() {
        assert!(!Engine::new().host_driven, "standalone default is self-scheduled");
        assert!(Engine::new_host_driven().host_driven, "host-driven constructor sets the flag");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd engine && cargo test -p sequencer_engine host_driven_flag_reflects_constructor`
Expected: FAIL — no field `host_driven` / no method `new_host_driven`.

- [ ] **Step 3: Write minimal implementation**

In `engine/crates/core/src/engine.rs`, add the field to the `Engine` struct (after `pub reload_generation: AtomicU32,`, the last field at ~line 284):

```rust
    /// Host-driven mode: when true, `engine_start` spawns only the state worker
    /// and the host drives dispatch via `Engine::render_host` (plugin port,
    /// Phase 0). Standalone (`Engine::new`) keeps this false.
    pub host_driven: bool,
```

In `Engine::new()` (the `Self { ... }` literal at ~line 298), add as the last field before the closing brace:

```rust
            host_driven: false,
```

Add a constructor right after `Engine::new()`:

```rust
    /// Construct an engine in host-driven mode (plugin host drives rendering via
    /// `Engine::render_host`). Identical to `Engine::new` except `host_driven`,
    /// which makes `engine_start` spawn only the state worker.
    pub fn new_host_driven() -> Self {
        let mut e = Self::new();
        e.host_driven = true;
        e
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd engine && cargo test -p sequencer_engine host_driven_flag_reflects_constructor`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git -C /Users/gus/Git/StepForge add engine/crates/core/src/engine.rs
git -C /Users/gus/Git/StepForge commit -m "feat(core): Engine::new_host_driven + host_driven flag

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: `Engine::render_host` — transport/play transitions + step accumulator

**Files:**
- Modify: `engine/crates/core/src/engine.rs` (new method on `impl Engine`)
- Test: `engine/crates/core/tests/host_render.rs` (new integration test)

**Interfaces:**
- Consumes (from Task 1): `HostTransport`, `HostRenderState`, `MidiEvent`. Reuses `Engine::process_one` (private, same `impl`), `Engine::begin_play`, `Engine::snapshot` (`arc_swap`), `Engine::midi`, `Engine::commands`.
- Produces: `Engine::render_host(&self, rs: &mut HostRenderState, transport: &HostTransport, midi_in: &[MidiEvent], midi_out: &mut [MidiEvent]) -> usize` — returns events written.

- [ ] **Step 1: Write the failing test**

Create `engine/crates/core/tests/host_render.rs`:

```rust
//! Integration tests for `Engine::render_host` (Phase 0 host-driven mode).
//! Drives the reused `process()` core from a synthetic host transport — no
//! threads, no plugin wrapper.

use sequencer_engine::engine::Engine;
use sequencer_engine::host::{HostRenderState, HostTransport};
use sequencer_engine::models::{Session, Step, VelocityZone, STEP_COUNT};

fn session_with_step0_hit() -> Session {
    let mut s = Session::default(); // bpm 120, 4 default tracks, patterns all Some
    let p = s.patterns[0].as_mut().unwrap();
    p.tracks[0].steps[0] = Step { active: true, velocity_zone: VelocityZone::Accent, ..Default::default() };
    p.tracks[0].midi_note = 36;
    s
}

fn transport(tempo: f64, sr: f64, block: u32, beat: f64, bar: f64, playing: bool) -> HostTransport {
    HostTransport { tempo_bpm: tempo, sample_rate: sr, block_samples: block, block_start_beat: beat, bar_start_beat: bar, is_playing: playing, beats_per_bar: 4.0 }
}

#[test]
fn stopped_engine_emits_nothing() {
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    let n = eng.render_host(&mut rs, &transport(120.0, 48_000.0, 256, 0.0, 0.0, false), &[], &mut out);
    assert_eq!(n, 0, "stopped transport emits no note-ons");
}

#[test]
fn play_advances_one_step_per_16th_boundary() {
    // 120 BPM, 48 kHz: one 16th = 60/120/4 s = 0.125 s = 6000 samples.
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let sr = 48_000.0;
    let block = 6_000u32; // exactly one 16th per block
    let mut beat = 0.0f64;
    let mut total_notes = 0usize;
    for i in 0..16 {
        let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
        let n = eng.render_host(&mut rs, &transport(120.0, sr, block, beat, 0.0, true), &[], &mut out);
        for ev in &out[..n] {
            // Status FAMILY, not a literal nibble: the default
            // global_midi_channel is 10, so the drum channel rides nibble 0xA
            // (0x9A note-on / 0x8A note-off). Asserting the family keeps this
            // correct if the default channel ever changes.
            let fam = ev.status & 0xF0;
            assert!(fam == 0x90 || fam == 0x80, "note-on/off status family, got {:#04x}", ev.status);
            if fam == 0x90 && ev.data2 > 0 { total_notes += 1; }
        }
        beat += 0.25; // one 16th per block
        let _ = i;
    }
    // Track 0 has a hit only on step 0. The 16-block run starts at a bar
    // boundary (beat 0 == bar_start_beat 0), so step 0 fires at beat 0 in
    // block 0 (immediate downbeat on play-start — I1 fix), then again each
    // subsequent bar. Asserting "at least one" keeps this robust to the exact
    // per-track step mapping.
    assert!(total_notes >= 1, "expected at least one note-on over a bar");
    // global_step stayed in range.
    assert!(rs.rt.global_step < STEP_COUNT as u32);
}

#[test]
fn play_start_mid_bar_aligns_per_track_step() {
    // Host resumes at beat 1.0 — four 16ths into the bar. Each track's playhead
    // must align to step 4 (not step 0), so a mid-bar resume doesn't replay the
    // downbeat. With immediate-fire (I1 fix), step 4 also FIRES in this block,
    // advancing `global_step` and each `per_track[..].step_idx` by 1 (the
    // default `speed_ratio == 1.0` advances exactly +1 per `process_one`).
    // If alignment were WRONG (step 0), `global_step` would be 1, not 5 — so
    // the assertions below still distinguish correct alignment.
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    eng.render_host(&mut rs, &transport(120.0, 48_000.0, 256, 1.0, 0.0, true), &[], &mut out);
    let length = eng.snapshot_arc().patterns[0].as_ref().unwrap().tracks[0].length;
    assert_eq!(rs.rt.per_track[0].step_idx, 5 % length, "step 4 aligned + fired (advancing to 5)");
    assert_eq!(rs.rt.global_step, 5, "global_step 4 aligned + fired (advancing to 5)");
}

#[test]
fn play_start_at_bar_boundary_fires_downbeat_immediately() {
    // I1 regression guard: at a bar boundary (sixteenths == 0), the downbeat
    // must fire IMMEDIATELY in block 0 at sample_offset == 0 — not at
    // beat 0.25 (~125 ms silence at 120 BPM). Pre-fix, block 0 emitted
    // nothing because `next_step_beat` was set to the NEXT boundary; this
    // test pins the immediate-downbeat behavior after the `+ 1.0` removal.
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    let n = eng.render_host(&mut rs, &transport(120.0, 48_000.0, 256, 0.0, 0.0, true), &[], &mut out);
    assert!(
        out[..n].iter().any(|ev| (ev.status & 0xF0) == 0x90
            && ev.data1 == 36
            && ev.data2 > 0
            && ev.sample_offset == 0),
        "downbeat (note 36) must fire at sample_offset 0 in block 0 (immediate on play-start)"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd engine && cargo test -p sequencer_engine --test host_render`
Expected: FAIL — no method `render_host`.

- [ ] **Step 3: Write minimal implementation**

Add to `engine/crates/core/src/engine.rs` at the top of the file, near the other `use crate::...` (add these lines to the existing `use` block):

```rust
use crate::host::{HostRenderState, HostTransport, MidiEvent, PendingMidiQueue};
```

Then add this method to `impl Engine` (place it right after `fn process_one`, ~line 492):

```rust
    /// Host-driven render: advance the engine across one host audio block on the
    /// host's RT thread. Fires `process_one` once per 16th boundary that crosses
    /// the block, converts the lock-free MIDI ring to sample-offset `MidiEvent`s,
    /// schedules note-offs that outlast the block, maps incoming note-ons to
    /// pattern-select commands, and honors play/stop transitions. RT-safe (Hard
    /// Rule 1): no alloc, no lock — reuses `process()`, the lock-free ring, a
    /// non-blocking snapshot read, and fixed-size arrays in `HostRenderState`.
    pub fn render_host(
        &self,
        rs: &mut HostRenderState,
        transport: &HostTransport,
        midi_in: &[MidiEvent],
        midi_out: &mut [MidiEvent],
    ) -> usize {
        let mut written = 0usize;
        let block = transport.block_samples as u64;
        let block_start_abs = rs.sample_time;
        let block_end_abs = block_start_abs + block;

        let snap = self.snapshot.load(); // zero-alloc Guard; immutable for the block
        let channel = snap.global_midi_channel;

        // (1) Emit pending note-offs from prior blocks due within this block.
        rs.pending.drain_due(block_start_abs, block_end_abs, |ev| {
            if written < midi_out.len() {
                midi_out[written] = ev;
                written += 1;
            }
        });

        // (2) Incoming note-ons in the command octave → pattern-select commands.
        //     Reuses the worker's `QueuePattern` path; latency ≤ one worker drain.
        for ev in midi_in {
            if (ev.status & 0xF0) == 0x90 && ev.data2 > 0 {
                let idx = (ev.data1 as usize).saturating_sub(60) % crate::models::PATTERN_SLOTS;
                let _ = crate::midi_out::push_drop_oldest(
                    &self.commands,
                    crate::command::Command::QueuePattern {
                        index: idx,
                        quantize: crate::models::QuantizeGrain::NextStep,
                    },
                );
            }
        }

        let bps = (transport.tempo_bpm / 60.0).max(1e-6);
        let samples_per_beat = transport.sample_rate / bps;
        let block_end_beat =
            transport.block_start_beat + (block as f64) / samples_per_beat.max(1e-6);

        // (3) Stop transition: freeze advancement + all-notes-off at offset 0.
        if !transport.is_playing {
            if rs.was_playing {
                if written < midi_out.len() {
                    midi_out[written] = MidiEvent {
                        sample_offset: 0,
                        status: 0xB0 | (channel & 0x0F),
                        data1: 123,
                        data2: 0,
                    };
                    written += 1;
                }
                rs.was_playing = false;
                // CC 123 killed every sustaining note; drop any events still
                // queued (note-offs AND deferred note-ons) so they can't fire as
                // stale events in later stopped blocks.
                rs.pending.clear();
            }
            rs.sample_time = block_end_abs;
            rs.last_block_start_beat = transport.block_start_beat;
            return written;
        }

        // (4) Play-start or seek: reseed RNG + align global_step/next_step_beat
        //     to the host bar so step 0 lands on the downbeat.
        let jumped = !rs.initialized
            || rs.last_block_start_beat.is_nan()
            || transport.block_start_beat < rs.last_block_start_beat
            || (transport.block_start_beat - rs.last_block_start_beat)
                > 2.0 * (block as f64) / samples_per_beat.max(1e-6) + 1.0;
        if !rs.was_playing || jumped {
            self.begin_play(&mut rs.rt);
            let into_bar = (transport.block_start_beat - transport.bar_start_beat).max(0.0);
            let sixteenths = (into_bar * 4.0).floor() as u32;
            rs.rt.global_step = sixteenths % crate::models::STEP_COUNT as u32;
            // Align each track's playhead to the same bar position, so a mid-bar
            // resume starts at the right step instead of replaying step 0.
            // `begin_play` just reset every step_idx to 0; overwrite per track.
            // Exact for speed_ratio 1.0; other ratios are approximated (speed_acc
            // reset to 0) and self-correct within a step or two. Bounds-checked
            // against both the active pattern's track count and `per_track`.
            if let Some(pattern) = snap
                .patterns
                .get(snap.active_pattern_index)
                .and_then(|p| p.as_ref())
            {
                for (idx, track) in pattern.tracks.iter().enumerate() {
                    if let Some(slot) = rs.rt.per_track.get_mut(idx) {
                        slot.step_idx = (sixteenths as usize) % track.length.max(1);
                        slot.speed_acc = 0;
                    }
                }
            }
            // `next_step_beat` is the CURRENT 16th boundary (at or before
            // `block_start_beat`), NOT the one after — so on play-start at a
            // bar boundary (`sixteenths == 0`) the downbeat fires in block 0
            // at sample 0 (immediate-fire). Matches the standalone `run_rt_loop`,
            // which calls `process_one` on the first tick after Play (no
            // ~125 ms silent pre-roll at 120 BPM).
            rs.next_step_beat = transport.bar_start_beat + sixteenths as f64 * 0.25;
            rs.initialized = true;
        }
        rs.was_playing = true;

        // (5) Fire every 16th boundary that crosses this block. Strict `<`: a
        // boundary exactly at block_end_beat belongs to the next block.
        while rs.next_step_beat < block_end_beat {
            let off = ((rs.next_step_beat - transport.block_start_beat) * samples_per_beat) as i64;
            let boundary_offset = off.clamp(0, block as i64) as u32;
            self.process_one(&mut rs.rt, &snap, true, 0);
            // Drain this boundary's notes; assign boundary-relative offsets.
            while let Some(msg) = self.midi.dequeue() {
                emit_midi_msg(
                    &msg,
                    boundary_offset,
                    block,
                    block_start_abs,
                    transport.sample_rate,
                    &mut rs.pending,
                    midi_out,
                    &mut written,
                );
            }
            rs.next_step_beat += 0.25;
        }

        rs.sample_time = block_end_abs;
        rs.last_block_start_beat = transport.block_start_beat;
        written
    }
```

And add the free helper function near the bottom of `engine.rs` (next to `fn zone_weight`):

```rust
/// Convert a drained `MidiMsg` (micros-relative, from `process`) into
/// sample-offset `MidiEvent`s for this block — sample-accurately, never clamped.
///
/// A note-on whose swing/micro-timing pushes it past the block that fired its
/// boundary (common at high swing + small host blocks) is deferred onto `pending`
/// for the block that actually contains it, exactly as a spanning note-off is.
/// This matches how the standalone CoreMIDI worker resolves the same offset
/// against wall-clock — no timing is lost to a block-end clamp.
///
/// RT-safe (no alloc): fixed arrays + the bounded `pending` queue only.
#[allow(clippy::too_many_arguments)]
fn emit_midi_msg(
    msg: &crate::midi_out::MidiMsg,
    boundary_offset: u32,
    block_samples: u64,
    block_start_abs: u64,
    sample_rate: f64,
    pending: &mut PendingMidiQueue,
    out: &mut [MidiEvent],
    written: &mut usize,
) {
    let sub_samples = (msg.send_at_offset_micros as f64 / 1_000_000.0 * sample_rate) as u32;
    let on_within = boundary_offset.saturating_add(sub_samples); // true offset; may exceed block
    let on_abs = block_start_abs + on_within as u64;
    // NOTE: `block_samples` is u64 (feeds absolute-sample arithmetic in
    // `render_host`); `on_within` is u32 (a within-block offset). Parenthesize
    // the widening cast: `on_within as u64 < block_samples` won't parse (rustc
    // reads the `<` as generic args on `u64`); `(on_within as u64) <` is the
    // compiler-suggested disambiguation. Semantics: widen then compare (u64<u64).
    let note_off_status = msg.status.wrapping_sub(0x10); // 0x9X → 0x8X, channel nibble preserved; wrapping_sub stays panic-free on RT for any status byte
    let gate_samples = (msg.gate_micros as f64 / 1_000_000.0 * sample_rate) as u64;

    if (on_within as u64) < block_samples {
        // Note-on fires inside this block.
        if *written < out.len() {
            out[*written] = MidiEvent {
                sample_offset: on_within,
                status: msg.status,
                data1: msg.note,
                data2: msg.velocity,
            };
            *written += 1;
        }
        // Matching note-off — its gate may span into a later block.
        if (msg.status & 0xF0) == 0x90 && msg.gate_micros > 0 {
            let off_within = on_within as u64 + gate_samples;
            if off_within < block_samples {
                if *written < out.len() {
                    out[*written] = MidiEvent {
                        sample_offset: off_within as u32,
                        status: note_off_status,
                        data1: msg.note,
                        data2: 0,
                    };
                    *written += 1;
                }
            } else {
                pending.schedule(block_start_abs + off_within, note_off_status, msg.note, 0);
            }
        }
    } else {
        // Note-on lands past this block's end — defer it (sample-accurate) to the
        // block containing `on_abs`, and defer its note-off relative to it.
        pending.schedule(on_abs, msg.status, msg.note, msg.velocity);
        if (msg.status & 0xF0) == 0x90 && msg.gate_micros > 0 {
            pending.schedule(on_abs + gate_samples, note_off_status, msg.note, 0);
        }
    }
}
```

(Note: `msg.status.wrapping_sub(0x10)` turns a `0x9c` note-on status into the `0x8c` note-off status on the same channel; `wrapping_sub` is used instead of `-` so an unexpected status byte can't debug-panic on the RT path.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cd engine && cargo test -p sequencer_engine --test host_render`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git -C /Users/gus/Git/StepForge add engine/crates/core/src/engine.rs engine/crates/core/tests/host_render.rs
git -C /Users/gus/Git/StepForge commit -m "feat(core): Engine::render_host — host-driven step accumulator + transitions

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: `render_host` — note-off scheduling across block boundaries

**Files:**
- Modify: `engine/crates/core/tests/host_render.rs` (add test)
- (No source change — verifies the Task-3 scheduling path.)

**Interfaces:**
- Consumes: `Engine::render_host` (Task 3), `PendingMidiQueue::schedule/drain_due` (Task 1).

- [ ] **Step 1: Write the failing test**

Add to `engine/crates/core/tests/host_render.rs`:

```rust
#[test]
fn note_off_outlasts_block_and_fires_in_a_future_block() {
    // Default gate is 50 ms. At 48 kHz that is 2_400 samples — many blocks.
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let block = 256u32;
    let sr = 48_000.0;
    let mut beat = 0.0f64;
    let mut saw_note_on = false;
    let mut saw_note_off = false;
    for _ in 0..48 {
        let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
        let n = eng.render_host(&mut rs, &transport(120.0, sr, block, beat, 0.0, true), &[], &mut out);
        for ev in &out[..n] {
            // Status family, not literal nibble (default channel 10 → 0xA).
            if (ev.status & 0xF0) == 0x90 && ev.data1 == 36 && ev.data2 > 0 { saw_note_on = true; }
            if (ev.status & 0xF0) == 0x80 && ev.data1 == 36 { saw_note_off = true; }
        }
        beat += (block as f64) / sr * 2.0; // advance ~one 16th (6000 samples) every few blocks
        if saw_note_on && saw_note_off { break; }
    }
    assert!(saw_note_on, "note-on for note 36 must fire");
    assert!(saw_note_off, "matching note-off must fire (possibly a later block via the pending queue)");
}

#[test]
fn swung_note_on_past_block_defers_to_a_future_block() {
    // Odd steps are swing-delayed (clock.rs: `swing_offset_micros`). At 49% swing,
    // 120 BPM, 48 kHz, the delay is 0.49 * 125_000 µs ≈ 2_940 samples. With
    // 1_000-sample blocks a swung note-on lands ~2_940 samples past its boundary
    // block → it must defer (sample-accurate) to a later block, NOT clamp to
    // block_samples-1 and fire inside the boundary block.
    let mut s = Session { global_swing_pct: 49.0, ..Default::default() };
    let p = s.patterns[0].as_mut().unwrap();
    p.tracks[0].steps[1] = Step { active: true, velocity_zone: VelocityZone::Accent, ..Default::default() };
    p.tracks[0].midi_note = 36;
    let eng = Engine::new_host_driven();
    eng.publish(s);
    let mut rs = HostRenderState::new();
    let sr = 48_000.0;
    let block = 1_000u32;
    let beats_per_block = block as f64 / sr * (120.0 / 60.0);
    let mut beat = 0.0f64;
    let mut boundary_block: Option<usize> = None; // block where step 1's boundary fires
    let mut note_on_block: Option<usize> = None;  // block where the swung note-on emits
    for i in 0..64 {
        let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
        let global_before = rs.rt.global_step;
        let n = eng.render_host(&mut rs, &transport(120.0, sr, block, beat, 0.0, true), &[], &mut out);
        // Step 1's boundary is the global_step 1 → 2 transition.
        if boundary_block.is_none() && global_before == 1 && rs.rt.global_step == 2 {
            boundary_block = Some(i);
        }
        for ev in &out[..n] {
            assert!(ev.sample_offset < block, "no note clamped to block-1");
            if (ev.status & 0xF0) == 0x90 && ev.data1 == 36 && ev.data2 > 0 {
                note_on_block = Some(i);
            }
        }
        beat += beats_per_block;
    }
    let boundary_block = boundary_block.expect("step-1 boundary fired within a bar");
    let note_on_block = note_on_block.expect("swung note-on eventually emitted");
    assert!(
        note_on_block > boundary_block,
        "swung note-on deferred to block {note_on_block}, boundary was block {boundary_block} (would be equal if clamped)"
    );
}
```

- [ ] **Step 2: Run test to verify it fails (or passes if Task 3 already covers it)**

Run: `cd engine && cargo test -p sequencer_engine --test host_render note_off_outlasts_block`
Expected: PASS (this verifies the Task-3 scheduling path end-to-end). If it fails, the note-off status math in `emit_midi_msg` is wrong — fix before proceeding.

- [ ] **Step 3: Commit**

```bash
git -C /Users/gus/Git/StepForge add engine/crates/core/tests/host_render.rs
git -C /Users/gus/Git/StepForge commit -m "test(core): host render schedules note-offs across block boundaries

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: `render_host` — stop freezes + all-notes-off; play-start reseeds deterministically

**Files:**
- Modify: `engine/crates/core/tests/host_render.rs`

**Interfaces:**
- Consumes: `Engine::render_host`, `Engine::begin_play` (RNG seed from snapshot hash).

- [ ] **Step 1: Write the failing test**

Add to `engine/crates/core/tests/host_render.rs`:

```rust
#[test]
fn stop_transition_emits_all_notes_off_and_freezes() {
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    // Play a block, then stop.
    eng.render_host(&mut rs, &transport(120.0, 48_000.0, 256, 0.0, 0.0, true), &[], &mut out);
    out.iter_mut().for_each(|e| *e = sequencer_engine::host::MidiEvent::zero());
    let before_next = rs.next_step_beat;
    let n = eng.render_host(&mut rs, &transport(120.0, 48_000.0, 256, 0.25, 0.0, false), &[], &mut out);
    // Stop emits CC 123 all-notes-off on channel 10.
    // CC 123 all-notes-off on the drum channel (default channel 10 → 0xBA).
    assert!(out[..n].iter().any(|e| (e.status & 0xF0) == 0xB0 && e.data1 == 123), "all-notes-off on stop");
    // No new note-ons while stopped.
    assert!(!out[..n].iter().any(|e| (e.status & 0xF0) == 0x90 && e.data2 > 0), "no note-ons while stopped");
    assert!(!rs.was_playing);
    // next_step_beat unchanged across the stopped block.
    assert_eq!(rs.next_step_beat, before_next);
}

#[test]
fn play_start_reseeds_rng_deterministically() {
    // Two engines + render states with identical sessions must produce the same
    // first note velocity after begin_play reseeds from the snapshot hash.
    // humanize_velocity > 0 forces the RNG to actually shape velocity — with the
    // default 0.0 this test would pass trivially without exercising the reseed.
    // The block is 12_000 samples (two 16ths at 120 BPM/48 kHz) so the step-0
    // boundary at beat 0.25 falls strictly inside the block and actually fires;
    // a 6_000-sample block ends exactly on beat 0.25 and (strict `<`) fires
    // nothing, leaving va == vb == None — a false pass.
    let mut s = session_with_step0_hit();
    s.humanize_velocity = 0.5;
    let mut out_a = [sequencer_engine::host::MidiEvent::zero(); 64];
    let mut out_b = [sequencer_engine::host::MidiEvent::zero(); 64];
    let eng_a = Engine::new_host_driven();
    eng_a.publish(s.clone());
    let mut rs_a = HostRenderState::new();
    let na = eng_a.render_host(&mut rs_a, &transport(120.0, 48_000.0, 12_000, 0.0, 0.0, true), &[], &mut out_a);
    let eng_b = Engine::new_host_driven();
    eng_b.publish(s);
    let mut rs_b = HostRenderState::new();
    let nb = eng_b.render_host(&mut rs_b, &transport(120.0, 48_000.0, 12_000, 0.0, 0.0, true), &[], &mut out_b);
    // Status family, not literal 0x99: default channel 10 → 0x9A. A literal
    // 0x99 would match nothing, leaving va == vb == None (a false pass).
    let va = out_a[..na]
        .iter()
        .find(|e| (e.status & 0xF0) == 0x90 && e.data2 > 0)
        .map(|e| e.data2);
    let vb = out_b[..nb]
        .iter()
        .find(|e| (e.status & 0xF0) == 0x90 && e.data2 > 0)
        .map(|e| e.data2);
    assert_eq!(va, vb, "identical sessions reseed identically");
}
```

- [ ] **Step 2: Run tests**

Run: `cd engine && cargo test -p sequencer_engine --test host_render`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git -C /Users/gus/Git/StepForge add engine/crates/core/tests/host_render.rs
git -C /Users/gus/Git/StepForge commit -m "test(core): host render stop-freeze + deterministic reseed invariants

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: `proptest` invariants — offsets within block, global_step bounded, steady advance

**Files:**
- Modify: `engine/crates/core/tests/host_render.rs`

**Interfaces:**
- Consumes: `Engine::render_host` (already `proptest` is a dev-dep of `core`).

- [ ] **Step 1: Write the failing test**

Add to `engine/crates/core/tests/host_render.rs`:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn rendered_offsets_stay_in_block_and_step_stays_bounded(
        tempo in 60.0f64..200.0,
        sr in 44_100.0f64..96_000.0,
        block in 16u32..1024,
        n_blocks in 1usize..64,
    ) {
        let eng = Engine::new_host_driven();
        eng.publish(session_with_step0_hit());
        let mut rs = HostRenderState::new();
        let beats_per_block = (block as f64) / sr * (tempo / 60.0);
        let mut beat = 0.0f64;
        for _ in 0..n_blocks {
            let bar = (beat / 4.0).floor() * 4.0;
            let mut out = [sequencer_engine::host::MidiEvent::zero(); 256];
            let n = eng.render_host(
                &mut rs,
                &transport(tempo, sr, block, beat, bar, true),
                &[],
                &mut out,
            );
            for ev in &out[..n] {
                prop_assert!(ev.sample_offset < block, "offset {} >= block {}", ev.sample_offset, block);
            }
            prop_assert!(rs.rt.global_step < STEP_COUNT as u32, "global_step out of range");
            beat += beats_per_block;
        }
        // Over a steady run next_step_beat tracks the playhead within one 16th
        // (it is the first unconsumed boundary, in [beat, beat + 0.25)).
        prop_assert!(
            rs.next_step_beat >= beat - 1e-6 && rs.next_step_beat <= beat + 0.25 + 1e-6,
            "next_step_beat {} drifted from beat {}", rs.next_step_beat, beat
        );
    }
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cd engine && cargo test -p sequencer_engine --test host_render`
Expected: PASS (proptest runs many cases).

- [ ] **Step 3: Commit**

```bash
git -C /Users/gus/Git/StepForge add engine/crates/core/tests/host_render.rs
git -C /Users/gus/Git/StepForge commit -m "test(core): proptest host-render offsets + step-bound invariants

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: `render_host` — incoming MIDI selects patterns

**Files:**
- Modify: `engine/crates/core/tests/host_render.rs`

**Interfaces:**
- Consumes: `Engine::render_host` incoming-MIDI → `Command::QueuePattern` path; `Engine::apply_command` worker arm; `Engine::commands` queue + `Engine::run_worker_loop`/`apply_pattern_switch`.

- [ ] **Step 1: Write the failing test**

Add to `engine/crates/core/tests/host_render.rs`:

```rust
#[test]
fn incoming_command_octave_note_queues_pattern_select() {
    let eng = Engine::new_host_driven();
    let s = session_with_step0_hit();
    eng.publish(s);
    let mut rs = HostRenderState::new();
    // Note 61 in the command octave (60..) → pattern index 1.
    let midi_in = [sequencer_engine::host::MidiEvent { sample_offset: 0, status: 0x90, data1: 61, data2: 100 }];
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    eng.render_host(&mut rs, &transport(120.0, 48_000.0, 256, 0.0, 0.0, true), &midi_in, &mut out);
    // The render pushed a QueuePattern{1} command; apply it directly (no worker thread here).
    let cmd = eng.commands.dequeue().expect("a queued command");
    assert!(matches!(cmd, sequencer_engine::command::Command::QueuePattern { index: 1, .. }));
}
```

- [ ] **Step 2: Run test**

Run: `cd engine && cargo test -p sequencer_engine --test host_render incoming_command_octave`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git -C /Users/gus/Git/StepForge add engine/crates/core/tests/host_render.rs
git -C /Users/gus/Git/StepForge commit -m "test(core): host render incoming-MIDI pattern-select mapping

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: FFI — host-driven constructor + render-state handle + `engine_render` + `engine_start` branch + header regen

**Files:**
- Modify: `engine/crates/ffi/src/handle.rs` (`RenderStateHandle`, `new_host_handle`, `new_render_state`, `free_render_state`)
- Modify: `engine/crates/ffi/src/lib.rs` (re-exports; `engine_start` branch; 4 new entries)
- Regenerate: `engine/include/sequencer_engine.h`
- Test: `engine/crates/ffi/tests/host_render_api.rs`

**Interfaces:**
- Consumes (core): `Engine::new_host_driven`, `Engine::render_host`, `Engine::host_driven`, `sequencer_engine::host::{HostTransport, MidiEvent, HostRenderState}`.
- Produces (FFI/C): `engine_new_host_driven`, `engine_render_state_new`, `engine_render`, `engine_render_state_free`; `RenderStateHandle` opaque; re-exported `HostTransport`/`MidiEvent`.

- [ ] **Step 1: Write the failing test**

Create `engine/crates/ffi/tests/host_render_api.rs`:

```rust
//! C-ABI round-trip for the host-driven entries (Phase 0). Exercises the full
//! `engine_new_host_driven → engine_start → engine_render → engine_stop → free`
//! lifecycle in-process, mirroring how a plugin wrapper will call it.

use sequencer_engine_ffi::{
    engine_free, engine_new, engine_new_host_driven, engine_render, engine_render_state_free,
    engine_render_state_new, engine_start, engine_stop, EngineResult, MidiEvent, HostTransport,
};

#[test]
fn host_driven_lifecycle_round_trip() {
    unsafe {
        let eng = engine_new_host_driven();
        assert!(!eng.is_null());
        let rs = engine_render_state_new();
        assert!(!rs.is_null());
        // host-driven start spawns only the state worker.
        assert!(matches!(engine_start(eng), EngineResult::Ok));

        let t = HostTransport { tempo_bpm: 120.0, sample_rate: 48_000.0, block_samples: 6_000, block_start_beat: 0.0, bar_start_beat: 0.0, is_playing: true, beats_per_bar: 4.0 };
        let mut out = [MidiEvent::zero(); 64];
        let mut count = 0usize;
        let r = engine_render(eng, rs, &t, [].as_ptr(), 0, out.as_mut_ptr(), out.len(), &mut count);
        assert!(matches!(r, EngineResult::Ok), "render returned {r:?}");
        // Default session has no active steps → no notes; still must not crash and
        // count must be ≤ capacity.
        assert!(count <= out.len());

        assert!(matches!(engine_stop(eng), EngineResult::Ok));
        engine_render_state_free(rs);
        engine_free(eng);
    }
}

#[test]
fn null_handle_or_state_is_rejected() {
    use sequencer_engine_ffi::HostTransport;
    unsafe {
        let rs = engine_render_state_new();
        let t = HostTransport { tempo_bpm: 120.0, sample_rate: 48_000.0, block_samples: 256, block_start_beat: 0.0, bar_start_beat: 0.0, is_playing: false, beats_per_bar: 4.0 };
        let mut out = [MidiEvent::zero(); 8];
        let mut count = 0usize;
        let r = engine_render(std::ptr::null_mut(), rs, &t, [].as_ptr(), 0, out.as_mut_ptr(), out.len(), &mut count);
        assert!(matches!(r, EngineResult::ErrInvalidHandle));
        engine_render_state_free(rs);
        // NULL render state is a tolerated no-op for free.
        engine_render_state_free(std::ptr::null_mut());
    }
}

#[test]
fn engine_render_rejects_standalone_engine() {
    // M2 host-pairing guard: a host that mis-pairs `engine_new` (standalone)
    // with `engine_render` must get `ErrInvalidHandle` instead of silently
    // double-dispatching (self-scheduled RT thread + host RT thread both
    // driving `process_one`). `host_driven` is false from construction, so no
    // `engine_start` is needed to exercise the guard.
    unsafe {
        let eng = engine_new(); // NOT host_driven
        assert!(!eng.is_null());
        let rs = engine_render_state_new();
        assert!(!rs.is_null());

        let t = HostTransport { tempo_bpm: 120.0, sample_rate: 48_000.0, block_samples: 256, block_start_beat: 0.0, bar_start_beat: 0.0, is_playing: true, beats_per_bar: 4.0 };
        let mut out = [MidiEvent::zero(); 8];
        let mut count = 0usize;
        let r = engine_render(eng, rs, &t, [].as_ptr(), 0, out.as_mut_ptr(), out.len(), &mut count);
        assert!(matches!(r, EngineResult::ErrInvalidHandle), "engine_render on standalone engine must reject (got {r:?})");

        engine_render_state_free(rs);
        engine_free(eng);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd engine && cargo test -p sequencer_engine_ffi --test host_render_api`
Expected: FAIL — unresolved `engine_new_host_driven`, `RenderStateHandle`, etc.

- [ ] **Step 3: Write minimal implementation**

In `engine/crates/ffi/src/handle.rs`, add after `EngineHandle`:

```rust
use sequencer_engine::host::HostRenderState;

/// Opaque handle to a `HostRenderState` (one per host-driven engine instance).
/// Owned by the plugin wrapper; never dereferenced in C.
#[repr(C)]
pub struct RenderStateHandle {
    _private: [u8; 0],
}

/// Allocate a host-driven engine handle (never NULL).
pub fn new_host_handle() -> *mut EngineHandle {
    let engine = Arc::new(Engine::new_host_driven());
    Arc::into_raw(engine) as *mut EngineHandle
}

/// Allocate a fresh render-state handle (never NULL).
pub fn new_render_state() -> *mut RenderStateHandle {
    Box::into_raw(Box::new(HostRenderState::new())) as *mut RenderStateHandle
}

/// Free a render-state handle. NULL is a tolerated no-op.
///
/// # Safety
/// `handle` is NULL or a pointer from [`new_render_state`]; no concurrent use.
pub unsafe fn free_render_state(handle: *mut RenderStateHandle) {
    if handle.is_null() {
        return;
    }
    unsafe { drop(Box::from_raw(handle as *mut HostRenderState)) };
}
```

In `engine/crates/ffi/src/lib.rs`, add the re-export near the top (after `pub use handle::EngineHandle;`):

```rust
pub use handle::RenderStateHandle;
pub use sequencer_engine::host::{HostRenderState, HostTransport, MidiEvent};
```

Add `engine_start` branching — insert at the very top of the `engine_start` body, right after the `if engine.is_null()` check and the `let eng = unsafe { Arc::from_raw(...) }` + `eng.shutdown.store(false, ...)` lines (i.e., before `create_client_and_port`):

```rust
        // Host-driven mode (plugin port, Phase 0): spawn ONLY the state worker.
        // The host drives dispatch via `engine_render`; no self-scheduled RT
        // thread, no CoreMIDI worker, no Link poller.
        if eng.host_driven {
            let eng_worker = Arc::clone(&eng);
            let worker_handle = std::thread::spawn(move || {
                eng_worker.run_worker_loop();
            });
            *eng.worker_handle.lock().unwrap() = Some(worker_handle);
            std::mem::forget(eng);
            return Ok(());
        }
```

Add the four new `extern "C"` entries at the end of `engine/crates/ffi/src/lib.rs`:

```rust
/// Create a host-driven engine. Returns an opaque handle (never NULL). Pair with
/// [`engine_render_state_new`] and drive via [`engine_render`].
#[no_mangle]
pub extern "C" fn engine_new_host_driven() -> *mut EngineHandle {
    match catch_unwind(handle::new_host_handle) {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

/// Allocate a per-instance render-state handle for [`engine_render`].
#[no_mangle]
pub extern "C" fn engine_render_state_new() -> *mut RenderStateHandle {
    match catch_unwind(AssertUnwindSafe(handle::new_render_state)) {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a render-state handle. NULL is a tolerated no-op.
///
/// # Safety
/// `handle` is NULL or from [`engine_render_state_new`]; no concurrent use.
#[no_mangle]
pub unsafe extern "C" fn engine_render_state_free(handle: *mut RenderStateHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe { handle::free_render_state(handle) }));
}

/// Advance the engine by one host audio block on the host's RT thread. Writes
/// outgoing MIDI `MidiEvent`s into `midi_out` (sample offsets within the block)
/// and returns the count in `*midi_out_count`.
///
/// # Safety
/// `engine`/`rs`/`transport` valid (or NULL for `engine`/`rs` → error); `midi_in`
/// valid for `midi_in_count` entries; `midi_out` valid for `midi_out_cap` entries;
/// `midi_out_count` writable (or NULL).
#[no_mangle]
pub unsafe extern "C" fn engine_render(
    engine: *mut EngineHandle,
    rs: *mut RenderStateHandle,
    transport: *const HostTransport,
    midi_in: *const MidiEvent,
    midi_in_count: usize,
    midi_out: *mut MidiEvent,
    midi_out_cap: usize,
    midi_out_count: *mut usize,
) -> EngineResult {
    match catch_unwind(AssertUnwindSafe(|| {
        if engine.is_null() || rs.is_null() || transport.is_null() {
            return Err(EngineResult::ErrInvalidHandle);
        }
        if midi_in.is_null() && midi_in_count != 0 {
            return Err(EngineResult::ErrInvalidBuffer);
        }
        // SAFETY: caller upholds Hard Rule 5 (no concurrent free); handles come
        // from engine_new_host_driven / engine_render_state_new.
        let eng = unsafe { &*(engine as *const Engine) };
        // M2: enforce the host-pairing contract — `engine_render` only drives
        // host-mode engines. A host that mis-pairs `engine_new` (standalone)
        // with `engine_render` would otherwise double-dispatch (self-scheduled
        // RT thread + host RT thread both driving `process_one`).
        if !eng.host_driven {
            return Err(EngineResult::ErrInvalidHandle);
        }
        let state = unsafe { &mut *(rs as *mut HostRenderState) };
        let transport = unsafe { &*transport };
        let midi_in: &[MidiEvent] = if midi_in_count == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(midi_in, midi_in_count) }
        };
        let midi_out: &mut [MidiEvent] = if midi_out_cap == 0 || midi_out.is_null() {
            &mut []
        } else {
            unsafe { core::slice::from_raw_parts_mut(midi_out, midi_out_cap) }
        };
        let written = eng.render_host(state, transport, midi_in, midi_out);
        if !midi_out_count.is_null() {
            unsafe { *midi_out_count = written };
        }
        Ok(())
    })) {
        Ok(Ok(())) => EngineResult::Ok,
        Ok(Err(e)) => e,
        Err(_) => EngineResult::ErrOther,
    }
}
```

- [ ] **Step 4: Regenerate the C header**

Run:
```bash
cd engine && export PATH="$HOME/.cargo/bin:$PATH" && cbindgen --crate sequencer_engine_ffi --config cbindgen.toml --output include/sequencer_engine.h --lang c
```
Then verify the new symbols are present:
```bash
grep -E 'engine_new_host_driven|engine_render_state_new|engine_render|RenderStateHandle|HostTransport|MidiEvent' include/sequencer_engine.h
```
Expected: matches for all six. `HostTransport` and `MidiEvent` are the **first core-crate types** to appear in this header — every prior exported fn uses only ffi-local types — and `cbindgen.toml` has `parse_deps = false`, so cbindgen cannot see their field definitions and may emit them as **opaque forward-declarations**. Verify the full struct bodies are emitted, not just opaque typedefs:

```bash
grep -A7 'typedef struct HostTransport' include/sequencer_engine.h
```

Expected: the field list (`tempo_bpm`, `sample_rate`, …, `beats_per_bar`). If instead you see an opaque `typedef struct HostTransport HostTransport;` with no body, let cbindgen parse the core crate — add `parse_deps = true` plus `include = ["sequencer_engine"]` under `[parse]` in `cbindgen.toml` and regen. (`parse_deps` is a **bool** in cbindgen, not an array — the array form `parse_deps = ["sequencer_engine"]` panics in cbindgen 0.29.x; `include` is what scopes parsing to ONLY `sequencer_engine`, so serde/postcard/arc-swap/heapless are not parsed.) This is safe and additive: only types reachable from exported fns are emitted, and both structs carry only scalars, so no extra core types leak into the header.

- [ ] **Step 5: Run test to verify it passes**

Run: `cd engine && cargo test -p sequencer_engine_ffi --test host_render_api`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git -C /Users/gus/Git/StepForge add engine/crates/ffi/src/handle.rs engine/crates/ffi/src/lib.rs engine/include/sequencer_engine.h engine/crates/ffi/tests/host_render_api.rs
git -C /Users/gus/Git/StepForge commit -m "feat(ffi): host-driven engine + engine_render C ABI (Phase 0)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: RT-safety audit + lint + iOS gate + standalone regression

**Files:**
- Audit (read-only): `engine/crates/core/src/engine.rs` (`render_host`, `emit_midi_msg`), `engine/crates/core/src/host.rs`.
- No source changes expected.

- [ ] **Step 1: Run the RT audit skill**

Run the project `/audit-rt` command against `engine/crates/core/src/engine.rs` and `engine/crates/core/src/host.rs`. Verify it flags nothing on the `render_host` / `emit_midi_msg` path: no `Vec`/`String`/`format!`, no `Mutex`/lock, no FFI, no CoreMIDI, no Link. The only `arc_swap` snapshot read and lock-free `MidiOutRing`/`CommandQueue` pushes are permitted (pre-existing RT-safe primitives).

- [ ] **Step 2: Lint + format the workspace**

Run:
```bash
cd engine && export PATH="$HOME/.cargo/bin:$PATH" && cargo fmt && cargo clippy --all-targets -- -D warnings
```
Expected: clean.

- [ ] **Step 3: Full Rust test sweep (host + iOS target gate)**

Run:
```bash
cd engine && export PATH="$HOME/.cargo/bin:$PATH" && cargo test
cargo check --target aarch64-apple-ios
```
Expected: all existing tests still pass (standalone path unchanged); the new `host::tests`, `host_render`, and `host_render_api` tests pass; iOS still type-checks (host-driven code is target-agnostic).

- [ ] **Step 4: Standalone app regression (build the xcframework)**

Run:
```bash
cd engine && export PATH="$HOME/.cargo/bin:$PATH" && ./scripts/build_engine.sh
```
Expected: xcframework + header regenerate successfully (the standalone app's preBuildScript runs this too).

- [ ] **Step 5: Commit any formatting churn + record audit result**

```bash
git -C /Users/gus/Git/StepForge add -u
git -C /Users/gus/Git/StepForge commit -m "chore(core,ffi): fmt + clippy gate after Phase 0 host-adapter

Co-Authored-By: Claude <noreply@anthropic.com>"
```
(If there is nothing to commit, skip. The `/audit-rt` result is recorded in the PR description.)

---

## Verification (whole-plan summary)

- **Core unit/integration:** `cargo test -p sequencer_engine` incl. `host::tests` (queue schedule/drain carrying `data2`, `clear`, render-state), `host_render` (stopped, step-advance, mid-bar per-track alignment, note-off-spanning, swung-note-on deferral, stop-freeze, deterministic reseed, incoming-MIDI pattern-select), and the `proptest` (offsets within block, `global_step` bounded, steady advance).
- **FFI C-ABI:** `cargo test -p sequencer_engine_ffi --test host_render_api` (lifecycle round-trip, NULL-handle rejection).
- **Lint/format:** `cargo fmt && cargo clippy --all-targets -- -D warnings`.
- **iOS gate:** `cargo check --target aarch64-apple-ios`.
- **Standalone regression:** `engine/scripts/build_engine.sh` regenerates the xcframework + header; the existing app targets still build (`xcodebuild` against `app/StepForge.xcodeproj` if Xcode is available).
- **RT-safety:** `/audit-rt` on the two new/modified core files flags nothing on the render path.

## Notes for the implementing engineer

- The host-driven path reuses the existing RT-safe primitives verbatim — do **not** add new allocation, locking, or FFI on the `render_host`/`emit_midi_msg` path. If a test or step seems to need a `Vec`, use a fixed-size array (see `PendingMidiQueue`).
- `Engine::render_host` deliberately reuses the private `process_one` (same `impl` block) so the host-driven path shares the exact dispatch + scheduler semantics of the self-scheduled RT loop — no divergence in note firing.
- Note-off status is derived from the note-on status (`status.wrapping_sub(0x10)`) rather than hardcoded `0x80`, so the channel nibble is preserved (`0x9A` → `0x8A`) and no status byte can debug-panic on RT.
- Three host-render timing behaviors are sample-accurate and leak-free: (1) a note-on whose swing/micro-timing pushes it past the block that fired its boundary is deferred on `pending` to the block that actually contains it — never clamped to `block_samples - 1` (the `PendingMidiQueue` carries `data2`, so it holds note-ons as well as note-offs); (2) transport stop emits CC 123 *then* `pending.clear()`, so no stale note-offs or deferred note-ons fire for notes the host already killed; (3) on play-start/seek each track's `step_idx` is aligned to the host bar (`sixteenths % length`, `speed_acc` reset to 0 — exact for `speed_ratio` 1.0).
- Phase 0 does **not** disable Link construction; that is a Phase 3 plugin-bundle concern (see "Scope boundary"). Do not feature-gate Link here.
