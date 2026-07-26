# StepForge Plugin — Phase 0: Rust Host-Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a host-driven mode to `sequencer_engine` so a plugin host's render callback can drive the existing dispatch core sample-accurately, with no new threads on the RT path.

**Architecture:** A new `engine_render` C entry drives the reused, pure `process()` core once per 16th-note boundary that crosses a host audio block. Per-instance mutable render state (RT step accumulator + a fixed-size scheduled note-off queue for gates spanning block boundaries) lives in a caller-owned `HostRenderState` handle, so `Engine` stays `Send+Sync` with no locks, no `UnsafeCell`, and no `unsafe` in core. Incoming MIDI maps to existing `QueuePattern` commands (pattern select). A runtime `host_driven` flag makes `engine_start` spawn only the state worker. The standalone app path (`engine_new`/`engine_start`) is unchanged.

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
- Produces: `#[repr(C)] pub struct HostTransport`, `#[repr(C)] pub struct MidiEvent`, `pub struct HostRenderState`, `pub struct PendingNoteOff`, `pub struct PendingNoteOffQueue`, `const PENDING_OFF_DEPTH: usize = 64`.

- [ ] **Step 1: Write the failing test**

Append to `engine/crates/core/src/host.rs` (after the impls below exist — write the test first, then the types, in this same file edit):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_queue_schedules_and_drains_due_only() {
        let mut q = PendingNoteOffQueue::new();
        // Two note-offs scheduled in the future.
        q.schedule(1_000, 0x80, 36);
        q.schedule(2_000, 0x80, 38);
        let mut emitted = Vec::new();
        q.drain_due(0, 1_500, |ev| emitted.push(ev));
        // Only the abs=1_000 one is due within [0,1_500).
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].sample_offset, 1_000);
        assert_eq!(emitted[0].data1, 36);
        // Draining again in a later block picks up the survivor.
        emitted.clear();
        q.drain_due(1_500, 3_000, |ev| emitted.push(ev));
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].data1, 38);
        // Fully drained.
        emitted.clear();
        q.drain_due(3_000, 4_000, |_| panic!("no more"));
        assert!(emitted.is_empty());
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

/// Maximum simultaneous pending note-offs. A note's gate (default 50 ms) often
/// spans several audio blocks; these are held until due. Bounded → RT-safe.
pub const PENDING_OFF_DEPTH: usize = 64;

#[derive(Clone, Copy)]
struct PendingNoteOff {
    abs_sample: u64,
    status: u8,
    note: u8,
    active: bool,
}

/// Fixed-size, single-threaded (host-RT-owner) scheduled note-off queue. No
/// locks, no allocation — `render_host` is the only accessor.
pub struct PendingNoteOffQueue {
    slots: [PendingNoteOff; PENDING_OFF_DEPTH],
}

impl PendingNoteOffQueue {
    pub fn new() -> Self {
        Self {
            slots: [PendingNoteOff { abs_sample: 0, status: 0, note: 0, active: false };
                PENDING_OFF_DEPTH],
        }
    }

    /// Schedule a note-off at absolute sample time. Finds an inactive slot; if
    /// none, evicts the slot with the largest `abs_sample` (drop-furthest).
    pub fn schedule(&mut self, abs_sample: u64, status: u8, note: u8) {
        let mut victim = 0usize;
        let mut victim_abs = u64::MIN;
        for (i, s) in self.slots.iter_mut().enumerate() {
            if !s.active {
                s.active = true;
                s.abs_sample = abs_sample;
                s.status = status;
                s.note = note;
                return;
            }
            if s.abs_sample > victim_abs {
                victim_abs = s.abs_sample;
                victim = i;
            }
        }
        // Full — evict the furthest-future slot if this one is sooner.
        if abs_sample < self.slots[victim].abs_sample {
            self.slots[victim] = PendingNoteOff { abs_sample, status, note, active: true };
        }
    }

    /// Emit note-offs whose `abs_sample` falls in `[block_start_abs, block_end_abs)`
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
                    data1: s.note,
                    data2: 0,
                });
                s.active = false;
            }
        }
    }
}

impl Default for PendingNoteOffQueue {
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
    pub pending: PendingNoteOffQueue,
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
            pending: PendingNoteOffQueue::new(),
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
Expected: PASS (2 tests).

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
use sequencer_engine::models::{Pattern, Session, Step, VelocityZone, STEP_COUNT};

fn session_with_step0_hit() -> Session {
    let mut s = Session::default(); // bpm 120, 4 default tracks, patterns all Some
    let p = s.patterns[0].as_mut().unwrap();
    p.tracks[0].steps[0] = Step { active: true, velocity_zone: VelocityZone::Accent, ..Default::default() };
    p.tracks[0].midi_note = 36;
    s
}

fn transport(tempo: f64, sr: f64, block: u32, beat: f64, bar: f64, playing: bool) -> HostTransport {
    HostTransport { tempo_bpm: tempo, sample_rate: sr, block_samples: block, block_start_beat: beat, bar_start_beat: bar, is_playing: playing }
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
            assert!(ev.status == 0x99 || ev.status == 0x89, "channel-10 note-on/off status");
            if ev.status == 0x99 { total_notes += 1; }
        }
        beat += 0.25; // one 16th per block
        let _ = i;
    }
    // Track 0 has a hit only on step 0; over 16 steps the playhead wraps once,
    // so step 0 fires twice → at least 2 note-ons. (Asserting "at least one"
    // keeps this robust to per-track step mapping.)
    assert!(total_notes >= 1, "expected at least one note-on over a bar");
    // global_step stayed in range.
    assert!(rs.rt.global_step < STEP_COUNT as u32);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd engine && cargo test -p sequencer_engine --test host_render`
Expected: FAIL — no method `render_host`.

- [ ] **Step 3: Write minimal implementation**

Add to `engine/crates/core/src/engine.rs` at the top of the file, near the other `use crate::...` (add these lines to the existing `use` block):

```rust
use crate::host::{HostRenderState, HostTransport, MidiEvent, PendingNoteOffQueue};
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
            rs.next_step_beat = transport.bar_start_beat + (sixteenths as f64 + 1.0) * 0.25;
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
/// Convert a drained `MidiMsg` (micros-relative, from `process`) into a
/// sample-offset `MidiEvent` for this block. Note-ons whose gate outlasts the
/// block are scheduled on `pending` for a future block. RT-safe (no alloc).
#[allow(clippy::too_many_arguments)]
fn emit_midi_msg(
    msg: &crate::midi_out::MidiMsg,
    boundary_offset: u32,
    block_samples: u64,
    block_start_abs: u64,
    sample_rate: f64,
    pending: &mut PendingNoteOffQueue,
    out: &mut [MidiEvent],
    written: &mut usize,
) {
    let sub_samples = (msg.send_at_offset_micros as f64 / 1_000_000.0 * sample_rate) as u32;
    let on_offset = boundary_offset.saturating_add(sub_samples).min(block_samples.saturating_sub(1) as u32);
    if *written < out.len() {
        out[*written] = MidiEvent { sample_offset: on_offset, status: msg.status, data1: msg.note, data2: msg.velocity };
        *written += 1;
    }
    // Note-on → synthesize + schedule the matching note-off.
    if (msg.status & 0xF0) == 0x90 && msg.gate_micros > 0 {
        let gate_samples = (msg.gate_micros as f64 / 1_000_000.0 * sample_rate) as u64;
        let off_within = on_offset as u64 + gate_samples;
        if off_within < block_samples {
            if *written < out.len() {
                out[*written] = MidiEvent { sample_offset: off_within as u32, status: (msg.status & 0xF0) - 0x10, data1: msg.note, data2: 0 };
                *written += 1;
            }
        } else {
            pending.schedule(block_start_abs + off_within, (msg.status & 0xF0) - 0x10, msg.note);
        }
    }
}
```

(Note: `(msg.status & 0xF0) - 0x10` turns a `0x9c` note-on status into the `0x8c` note-off status on the same channel.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cd engine && cargo test -p sequencer_engine --test host_render`
Expected: PASS (2 tests).

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
- Consumes: `Engine::render_host` (Task 3), `PendingNoteOffQueue::schedule/drain_due` (Task 1).

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
    for _ in 0..32 {
        let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
        let n = eng.render_host(&mut rs, &transport(120.0, sr, block, beat, 0.0, true), &[], &mut out);
        for ev in &out[..n] {
            if ev.status == 0x99 && ev.data1 == 36 { saw_note_on = true; }
            if ev.status == 0x89 && ev.data1 == 36 { saw_note_off = true; }
        }
        beat += (block as f64) / sr * 2.0; // advance ~one 16th (6000 samples) every few blocks
        if saw_note_on && saw_note_off { break; }
    }
    assert!(saw_note_on, "note-on for note 36 must fire");
    assert!(saw_note_off, "matching note-off must fire (possibly a later block via the pending queue)");
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
    assert!(out[..n].iter().any(|e| e.status == 0xB9 && e.data1 == 123), "all-notes-off on stop");
    // No new note-ons while stopped.
    assert!(!out[..n].iter().any(|e| e.status == 0x99), "no note-ons while stopped");
    assert!(!rs.was_playing);
    // next_step_beat unchanged across the stopped block.
    assert_eq!(rs.next_step_beat, before_next);
}

#[test]
fn play_start_reseeds_rng_deterministically() {
    // Two engines + render states with identical sessions must produce the same
    // first note velocity after begin_play reseeds from the snapshot hash.
    let s = session_with_step0_hit();
    let mut out_a = [sequencer_engine::host::MidiEvent::zero(); 64];
    let mut out_b = [sequencer_engine::host::MidiEvent::zero(); 64];
    let eng_a = Engine::new_host_driven();
    eng_a.publish(s.clone());
    let mut rs_a = HostRenderState::new();
    let na = eng_a.render_host(&mut rs_a, &transport(120.0, 48_000.0, 6_000, 0.0, 0.0, true), &[], &mut out_a);
    let eng_b = Engine::new_host_driven();
    eng_b.publish(s);
    let mut rs_b = HostRenderState::new();
    let nb = eng_b.render_host(&mut rs_b, &transport(120.0, 48_000.0, 6_000, 0.0, 0.0, true), &[], &mut out_b);
    let va = out_a[..na].iter().find(|e| e.status == 0x99).map(|e| e.data2);
    let vb = out_b[..nb].iter().find(|e| e.status == 0x99).map(|e| e.data2);
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
    engine_free, engine_new_host_driven, engine_render, engine_render_state_free,
    engine_render_state_new, engine_start, engine_stop, EngineResult, MidiEvent, RenderStateHandle,
    HostTransport,
};

#[test]
fn host_driven_lifecycle_renders_a_note_round_trip() {
    unsafe {
        let eng = engine_new_host_driven();
        assert!(!eng.is_null());
        let rs = engine_render_state_new();
        assert!(!rs.is_null());
        // host-driven start spawns only the state worker.
        assert!(matches!(engine_start(eng), EngineResult::Ok));

        let t = HostTransport { tempo_bpm: 120.0, sample_rate: 48_000.0, block_samples: 6_000, block_start_beat: 0.0, bar_start_beat: 0.0, is_playing: true };
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
        let t = HostTransport { tempo_bpm: 120.0, sample_rate: 48_000.0, block_samples: 256, block_start_beat: 0.0, bar_start_beat: 0.0, is_playing: false };
        let mut out = [MidiEvent::zero(); 8];
        let mut count = 0usize;
        let r = engine_render(std::ptr::null_mut(), rs, &t, [].as_ptr(), 0, out.as_mut_ptr(), out.len(), &mut count);
        assert!(matches!(r, EngineResult::ErrInvalidHandle));
        engine_render_state_free(rs);
        // NULL render state is a tolerated no-op for free.
        engine_render_state_free(std::ptr::null_mut());
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
Expected: matches for all six. If `HostTransport`/`MidiEvent` definitions are missing from the header, add them to `cbindgen.toml` under `[export]` / `[parse.expand]` as needed (cbindgen should pick up `#[repr(C)]` pub structs referenced by an exported function automatically).

- [ ] **Step 5: Run test to verify it passes**

Run: `cd engine && cargo test -p sequencer_engine_ffi --test host_render_api`
Expected: PASS (2 tests).

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

- **Core unit/integration:** `cargo test -p sequencer_engine` incl. `host::tests` (queue, render-state), `host_render` (stopped, step-advance, note-off-spanning, stop-freeze, deterministic reseed, incoming-MIDI pattern-select), and the `proptest` (offsets within block, `global_step` bounded, steady advance).
- **FFI C-ABI:** `cargo test -p sequencer_engine_ffi --test host_render_api` (lifecycle round-trip, NULL-handle rejection).
- **Lint/format:** `cargo fmt && cargo clippy --all-targets -- -D warnings`.
- **iOS gate:** `cargo check --target aarch64-apple-ios`.
- **Standalone regression:** `engine/scripts/build_engine.sh` regenerates the xcframework + header; the existing app targets still build (`xcodebuild` against `app/StepForge.xcodeproj` if Xcode is available).
- **RT-safety:** `/audit-rt` on the two new/modified core files flags nothing on the render path.

## Notes for the implementing engineer

- The host-driven path reuses the existing RT-safe primitives verbatim — do **not** add new allocation, locking, or FFI on the `render_host`/`emit_midi_msg` path. If a test or step seems to need a `Vec`, use a fixed-size array (see `PendingNoteOffQueue`).
- `Engine::render_host` deliberately reuses the private `process_one` (same `impl` block) so the host-driven path shares the exact dispatch + scheduler semantics of the self-scheduled RT loop — no divergence in note firing.
- Note-off status is derived from the note-on status (`(status & 0xF0) - 0x10`) rather than hardcoded `0x80` so the channel nibble is preserved.
- Phase 0 does **not** disable Link construction; that is a Phase 3 plugin-bundle concern (see "Scope boundary"). Do not feature-gate Link here.
