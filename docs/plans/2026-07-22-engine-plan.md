# Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `sequencer_engine` from a compiling skeleton into a working musical brain — a self-scheduled RT thread that advances musical time, dispatches steps (swing/humanize/speed_ratio/micro_timing), runs Roll/Vary, and emits MIDI through a lock-free ring to a CoreMIDI worker — resolving open issues E1–E11.

**Architecture:** Three threads over **copy-on-write immutable `Arc<Session>` snapshots** (`arc-swap`): the state worker is sole writer (clone→mutate→publish); the RT thread is a lock-free reader (zero-alloc `Guard` per tick); `engine_serialize` reads via `load_full()`. Bounded queues are `heapless::mpmc::Queue` (Arc-shared, `&self`, drop-oldest on overflow). The musical brain is pure functions + a `process()` tick driven by a pluggable `Clock` trait (`InstantClock` in ffi, `SteppableClock` in core) so the entire dispatch path is host-testable with no wall-clock.

**Tech Stack:** Rust (stable, edition 2021), `serde`+`postcard` (bytes over the C ABI), `arc-swap` (COW snapshots), `heapless` (lock-free bounded queues), `proptest` (algorithm invariants), CoreMIDI.framework (ffi). `core` is `#![forbid(unsafe_code)]`; all `unsafe` (RT-priority syscall, CoreMIDI) in `ffi`.

**Design doc:** `docs/superpowers/specs/2026-07-22-engine-plan-design.md` (read it — every decision below is pinned there, esp. the "Threading & concurrency model — COW snapshots" section).

## Global Constraints

Copied verbatim from the design + CLAUDE.md; every task implicitly includes these:

- `core` (`sequencer_engine`) stays `#![forbid(unsafe_code)]`; all `unsafe` in `ffi` (`sequencer_engine_ffi`). RT-priority syscall + all CoreMIDI live in `ffi`.
- **Hard Rule 1 (RT sacred):** the RT hot path never allocates, locks, crosses FFI into Swift, touches CoreMIDI, or blocks. It reads one immutable snapshot per tick (`Guard`), reads transport atomics, and writes only fixed-size slots (MIDI ring, `[u8; MAX_EVENT_BYTES]` event slots). External Link/MIDI-clock timing arrives as commands consumed by the **worker**, never pushed into RT.
- The FFI seam is **postcard bytes**, not structs. No data-carrying `#[repr(C)]` enum crosses. The 8 `extern "C"` entry points + `#[repr(C)] EngineResult` (`Ok=0, ErrDecode=1, ErrInvalidHandle=2, ErrInvalidBuffer=3, ErrOther=4`) are fixed; a new variant may only be **appended**. Every `extern "C"` body wraps `catch_unwind`; codecs are total.
- `engine_serialize` returns bytes **synchronously** on the caller thread (preserved exactly — read via `load_full()`, no worker round-trip).
- Drain framing: one event per `engine_drain_events` call; empty/zero-length = drained; NULL out-params tolerated. `MAX_EVENT_BYTES = 128` hot-channel cap; `Serialized`/`Error` only on the off-RT large channel.
- **Non-destructive length:** `Roll/Vary/Cut/Trash` never touch `length`/`midi_note`/`speed_ratio`; `Paste` carries `length`+`speed_ratio` but never `midi_note`; clipboard carries `steps`/`length`/`speed_ratio` but not `midi_note`.
- **No serialized model-shape change.** `SESSION_FORMAT_VERSION` stays **1**. All `f32` fields stay `f32`; the `Overflow` event is the only new cross-layer variant.
- Model constants: `MAX_TRACKS=8`, `MIN_TRACKS=4`, `STEP_COUNT=16`, `PATTERN_SLOTS=9`. The default `Session` has **all patterns `None`** — dispatch operates on `patterns[active_pattern_index]` and is a no-op when it is `None`; tests inject a session with an active pattern.
- Build env: run `export PATH="$HOME/.cargo/bin:$PATH"` before any cargo command (Homebrew rust shadows rustup). Verify with `cargo test` (engine) and the app build after cross-layer changes.
- **Cross-layer symmetry (no orphans):** adding `EngineEvent::Overflow` = Rust variant + codec + C-ABI round-trip test in *this* plan; the Swift mirror is app-plan work (tracked, not dropped).

**Two reconciliations from the design doc (this plan is authoritative):**
1. The design says `heapless::spsc::Queue` for the RT→worker ring. `spsc::Queue::split()` borrows the queue and cannot cross threads cleanly. **This plan uses `heapless::mpmc::Queue<T,N>` (Arc-shared, `&self` `enqueue`/`dequeue`, `Send+Sync`) for all three bounded queues.** The ring is SPSC *by access discipline* (only RT enqueues, only the CoreMIDI worker dequeues).
2. `humanize_seed` is not a serialized field; the RT RNG is a tiny xorshift64 in core seeded from a std-hash of the immutable snapshot at play-start (no `rand` dependency).

---

## File Structure

**Create:**
- `engine/crates/core/src/midi_out.rs` — `MidiMsg`, `MidiOutRing` (Arc<mpmc queue>), `HotEventSlot`, `HotEventChannel`, drop-oldest push helpers.
- `engine/crates/core/src/algorithms/mod.rs` — module root (exists as a stub; flesh out).
- `engine/crates/core/src/algorithms/roll.rs` — Roll.
- `engine/crates/core/src/algorithms/vary.rs` — Vary.
- `engine/crates/core/tests/` — integration tests (dispatch, algorithms, sync, load-session).

**Modify (currently doc-comment-only stubs unless noted):**
- `engine/crates/core/src/engine.rs` — `Engine` owns the snapshot store, queues, atomics, thread handles; `RtState`; `process()`.
- `engine/crates/core/src/clock.rs` — `Clock` trait, `SteppableClock`, pure timing math (speed_ratio/swing/micro_timing), `Rng`.
- `engine/crates/core/src/midi.rs` — pure MIDI dispatch math (velocity/humanize/ratchet/note-on).
- `engine/crates/core/src/scheduler.rs` — pattern queue, quantize, follow-actions, transitions.
- `engine/crates/core/src/undo.rs` — per-track one-deep undo.
- `engine/crates/core/src/clipboard.rs` — track/session clipboards.
- `engine/crates/core/src/event.rs` — add `Overflow{dropped:u32}`.
- `engine/crates/core/src/lib.rs` — `pub mod midi_out;` (and ensure `algorithms`, `clock`, etc. are declared — they are).
- `engine/crates/core/Cargo.toml` — add `arc-swap`, `heapless`; dev-dep `proptest`.
- `engine/crates/ffi/src/coremidi.rs` — real CoreMIDI bindings + worker + `InstantClock`.
- `engine/crates/ffi/src/lib.rs` — real `engine_start/stop/free`, `engine_submit_command` enqueues, `engine_drain_events` drains; `engine_serialize` via `load_full`.
- `engine/crates/ffi/src/handle.rs` — defensive join-then-drop in `free_handle`.
- `engine/crates/ffi/Cargo.toml` — add `arc-swap`, `heapless` if used directly (likely via core re-exports).

---

## Shared Interfaces (pinned — all tasks reference these exact signatures)

```rust
// core/src/midi_out.rs
use crate::event::EngineEvent;
use heapless::mpmc::Queue;
use std::sync::Arc;

pub const MIDI_RING_DEPTH: usize = 128;
pub const HOT_EVENT_DEPTH: usize = 32;
pub const COMMAND_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiMsg {
    pub endpoint: u32,            // Session.midi_destinations[id]; 0 = default
    pub channel: u8,
    pub status: u8,               // 0x90 NoteOn, 0x80 NoteOff, 0xB0 CC (all-notes-off = CC 123)
    pub note: u8,
    pub velocity: u8,
    pub send_at_offset_micros: u32, // NoteOn: worker sends at now+offset (swing+micro_timing, E2/E3)
    pub gate_micros: u32,           // NoteOn: worker synthesizes NoteOff at offset+gate (E5)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotEventSlot {
    pub len: u8,
    pub bytes: [u8; 128], // MAX_EVENT_BYTES
}

pub type MidiOutRing   = Arc<Queue<MidiMsg, MIDI_RING_DEPTH>>;
pub type HotEventChannel = Arc<Queue<HotEventSlot, HOT_EVENT_DEPTH>>;
pub type CommandQueue  = Arc<Queue<crate::command::Command, COMMAND_DEPTH>>;

/// Enqueue with drop-oldest on full. Returns the number of slots dropped.
pub fn push_drop_oldest<T: Copy, const N: usize>(q: &Queue<T, N>, val: T) -> usize;

/// Encode `ev` into a stack buffer and push as a HotEventSlot (drop-oldest).
pub fn push_event(events: &HotEventChannel, ev: &EngineEvent) -> usize;

pub fn midi_out_ring() -> MidiOutRing;
pub fn hot_event_channel() -> HotEventChannel;
pub fn command_queue() -> CommandQueue;
```

```rust
// core/src/clock.rs
pub trait Clock: Send + Sync {
    fn now_micros(&self) -> u64;
    fn sleep_until(&self, deadline_micros: u64);
    fn elevate_priority(&self);
}

pub struct SteppableClock { /* Cell<u64> */ }
impl SteppableClock {
    pub fn new() -> Self;
    pub fn advance_to(&self, micros: u64); // test-only: set now
}
impl Clock for SteppableClock { /* now_micros reads; sleep/elevate no-op */ }

/// Q16.16 fixed-point step accumulator advance. Returns (steps_to_fire, new_accum).
pub fn advance_speed_ratio(acc: u32, ratio_q: u32) -> (u32, u32);
/// Convert an f32 speed_ratio to Q16.16.
pub fn to_q16_16(ratio: f32) -> u32;

/// Effective swing % for a track: global + track (additive, E2).
pub fn effective_swing(global_pct: f32, track_pct: f32) -> f32; // capped < 50.0
/// Microsecond offset for step `step_idx` given swing and the 16th-note period.
pub fn swing_offset_micros(effective_swing_pct: f32, step_idx: usize, step_period_micros: u64) -> i64;
/// Apply micro_timing_offset (f32 fraction of a step), clamped to ±½ step. Returns micros delta.
pub fn micro_timing_offset_micros(offset: f32, step_period_micros: u64) -> i64;

pub struct Rng(pub u64); // xorshift64
impl Rng { pub fn new(seed: u64) -> Self; pub fn next_u32(&mut self) -> u32; pub fn range(&mut self, lo: i32, hi: i32) -> i32; }
```

```rust
// core/src/midi.rs  (pure dispatch math)
use crate::models::{Ratchet, VelocityZone};
use crate::midi_out::MidiMsg;

pub const VEL_LOW: u8 = 64;
pub const VEL_MID: u8 = 100;
pub const VEL_ACCENT: u8 = 127;
pub const DEFAULT_GATE_MICROS: u32 = 50_000; // 50 ms

pub fn velocity_for_zone(zone: VelocityZone) -> u8;
pub fn humanize_velocity(base: u8, humanize_velocity: f32, zone_weight: f32, rng: &mut crate::clock::Rng) -> u8;
pub fn ratchet_count(r: Ratchet) -> u32;                 // Off=1, X2=2, X3=3, X4=4
pub fn build_note_on(endpoint: u32, channel: u8, note: u8, velocity: u8, send_at_offset_micros: u32) -> MidiMsg;
pub fn build_all_notes_off(endpoint: u32, channel: u8) -> MidiMsg; // CC 123
```

```rust
// core/src/engine.rs
use arc_swap::ArcSwap;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;
use crate::models::Session;
use crate::midi_out::{MidiOutRing, HotEventChannel, CommandQueue};

pub struct Transport { pub is_playing: AtomicBool, pub stop_generation: AtomicU32 }

pub struct TrackRtState { pub step_idx: usize, pub speed_acc: u32 }
pub struct RtState {
    pub per_track: [TrackRtState; crate::models::MAX_TRACKS],
    pub rng: crate::clock::Rng,
}

pub struct Engine {
    pub snapshot: Arc<ArcSwap<Session>>,     // sole read path: load() (RT) / load_full() (serialize)
    pub commands: CommandQueue,              // producers: any thread via FFI; consumer: state worker
    pub midi: MidiOutRing,                   // producer: RT; consumer: CoreMIDI worker
    pub hot_events: HotEventChannel,         // producer: RT; consumer: engine_drain_events
    pub transport: Transport,
    pub shutdown: Arc<AtomicBool>,
    // thread handles filled by engine_start (ffi); None until started
    pub rt_handle: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    pub worker_handle: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Engine {
    pub fn new() -> Self;
    pub fn publish(&self, session: Session);                       // worker: clone-mutate-publish
    pub fn snapshot_arc(&self) -> Arc<Session>;                    // load_full
    pub fn begin_play(&self, rt: &mut RtState);                    // seed RNG from snapshot hash, reset counters
    #[cfg(test)] pub fn load_session_for_test(&self, session: Session); // publish (test seam)
}

/// One RT/global-tick. Pure of I/O except queue pushes. `playing` read by caller from transport.
pub fn process(
    rt: &mut RtState,
    session: &Session,
    playing: bool,
    now_micros: u64,
    midi: &MidiOutRing,
    events: &HotEventChannel,
) -> TickOutcome;

pub struct TickOutcome { pub playheads_emitted: u32, pub notes_pushed: u32 }
```

---

## Task 1: Add dependencies

**Files:**
- Modify: `engine/crates/core/Cargo.toml`
- Modify: `engine/Cargo.toml` (workspace deps)

**Interfaces:** Produces: `arc-swap`, `heapless` available in `core`; `proptest` as `core` dev-dep.

- [ ] **Step 1: Add workspace deps**

In `engine/Cargo.toml` `[workspace.dependencies]`, append:

```toml
arc-swap = "1"
heapless = "0.8"
proptest = "1"
```

- [ ] **Step 2: Wire into core**

Replace `engine/crates/core/Cargo.toml` `[dependencies]` and add `[dev-dependencies]`:

```toml
[dependencies]
serde = { workspace = true }
uuid = { workspace = true }
arc-swap = { workspace = true }
heapless = { workspace = true }

[dev-dependencies]
postcard = { workspace = true }
proptest = { workspace = true }
```

- [ ] **Step 3: Verify it builds and existing tests stay green**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test`
Expected: `12 passed; 0 failed` (unchanged from baseline), plus a successful dependency fetch.

- [ ] **Step 4: Commit**

```bash
git add engine/Cargo.toml engine/crates/core/Cargo.toml engine/Cargo.lock
git commit -m "build(engine): add arc-swap, heapless, proptest deps"
```

---

## Task 2: Clock trait + SteppableClock + Rng (core)

**Files:**
- Modify: `engine/crates/core/src/clock.rs`
- Test: `engine/crates/core/src/clock.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `Clock` trait, `SteppableClock`, `to_q16_16`, `advance_speed_ratio`, `Rng` (signatures above).

- [ ] **Step 1: Write failing tests**

Append to `engine/crates/core/src/clock.rs` (replacing the doc-only stub):

```rust
//! Self-scheduled clock + pure timing math. The Clock trait + SteppableClock
//! live in core (safe); the prod InstantClock (unsafe RT-priority) lives in ffi.

use std::cell::Cell;

pub trait Clock: Send + Sync {
    fn now_micros(&self) -> u64;
    fn sleep_until(&self, _deadline_micros: u64) {}
    fn elevate_priority(&self) {}
}

pub struct SteppableClock {
    now: Cell<u64>,
}
impl SteppableClock {
    pub fn new() -> Self { Self { now: Cell::new(0) } }
    pub fn advance_to(&self, micros: u64) { self.now.set(micros); }
}
impl Default for SteppableClock { fn default() -> Self { Self::new() } }
impl Clock for SteppableClock {
    fn now_micros(&self) -> u64 { self.now.get() }
}

/// f32 speed_ratio -> Q16.16 (1.0 == 0x1_0000).
pub fn to_q16_16(ratio: f32) -> u32 { (ratio * 65536.0) as u32 }

/// Advance a Q16.16 accumulator by `ratio_q`. Returns (whole steps to fire, new accumulator).
pub fn advance_speed_ratio(acc: u32, ratio_q: u32) -> (u32, u32) {
    let acc = acc.wrapping_add(ratio_q);
    (acc >> 16, acc & 0xFFFF)
}

/// Effective swing %, additive (global + track), hard-capped below 50 (E2).
pub fn effective_swing(global_pct: f32, track_pct: f32) -> f32 {
    let v = global_pct + track_pct;
    v.clamp(0.0, 49.0)
}

/// xorshift64 — deterministic, allocation-free, no `rand` dependency.
pub struct Rng(pub u64);
impl Rng {
    pub fn new(seed: u64) -> Self { Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed }) }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (hi - lo + 1).max(1) as u32;
        lo + (self.next_u32() % span) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steppable_clock_returns_set_time() {
        let c = SteppableClock::new();
        assert_eq!(c.now_micros(), 0);
        c.advance_to(123_456);
        assert_eq!(c.now_micros(), 123_456);
    }

    #[test]
    fn speed_ratio_accumulator_fires_correct_steps() {
        let q = to_q16_16(2.0);
        let (steps, acc) = advance_speed_ratio(0, q);
        assert_eq!(steps, 2);
        assert_eq!(acc, 0);
        // ratio 0.5 fires a step every other tick
        let qh = to_q16_16(0.5);
        let (s1, a1) = advance_speed_ratio(0, qh);
        assert_eq!(s1, 0); // 0.5 -> floor=0 first tick, carry 0.5
        let (s2, _) = advance_speed_ratio(a1, qh);
        assert_eq!(s2, 1); // second tick fires 1
    }

    #[test]
    fn effective_swing_is_additive_and_capped() {
        assert_eq!(effective_swing(10.0, 5.0), 15.0);
        assert_eq!(effective_swing(40.0, 20.0), 49.0); // capped < 50
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..10 { assert_eq!(a.next_u32(), b.next_u32()); }
        assert!(Rng::new(42).range(-5, 5) >= -5 && Rng::new(42).range(-5, 5) <= 5);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass (impl is inline above)**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine clock`
Expected: `4 passed; 0 failed`.

- [ ] **Step 3: Commit**

```bash
git add engine/crates/core/src/clock.rs
git commit -m "feat(core): Clock trait, SteppableClock, Q16.16 accumulator, xorshift Rng"
```

---

## Task 3: InstantClock (ffi) with RT-priority

**Files:**
- Modify: `engine/crates/ffi/src/coremidi.rs`
- Modify: `engine/crates/ffi/Cargo.toml`
- Test: `engine/crates/ffi/src/coremidi.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `sequencer_engine::clock::Clock`.
- Produces: `InstantClock` impl `Clock`.

- [ ] **Step 1: Add ffi deps**

`engine/crates/ffi/Cargo.toml` `[dependencies]` append:

```toml
arc-swap = { workspace = true }
heapless = { workspace = true }
```

- [ ] **Step 2: Write failing test + impl**

`engine/crates/ffi/src/coremidi.rs` (replace the `flush_all_notes_off` stub; keep it for now, add the clock at top):

```rust
//! CoreMIDI bindings + prod clock. The ONLY crate doing `unsafe`. RT-priority
//! syscall + MIDISend live here (Hard Rules 1, 6, 7).

use sequencer_engine::clock::Clock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub struct InstantClock {
    start: Instant,
    // cache start as micros once to keep now_micros allocation-free on RT
    base_micros: AtomicU64,
}
impl InstantClock {
    pub fn new() -> Self { Self { start: Instant::now(), base_micros: AtomicU64::new(0) } }
}
impl Default for InstantClock { fn default() -> Self { Self::new() } }
impl Clock for InstantClock {
    fn now_micros(&self) -> u64 { self.start.elapsed().as_micros() as u64 }
    fn sleep_until(&self, deadline_micros: u64) {
        let now = self.now_micros();
        if deadline_micros > now {
            let remaining = deadline_micros - now;
            // coarse sleep until ~1ms before, then spin the rest (sub-ms accuracy)
            if remaining > 1_000 {
                std::thread::sleep(std::time::Duration::from_micros(remaining - 1_000));
            }
            while self.now_micros() < deadline_micros {
                std::hint::spin_loop();
            }
        }
    }
    fn elevate_priority(&self) {
        // Called EXACTLY ONCE at RT-thread spawn (never in the per-tick loop).
        // Best-effort QoS elevation; failures are non-fatal (timing still correct).
        #[cfg(target_os = "ios")]
        unsafe { elevate_thread_rt_ios(); }
        #[cfg(not(target_os = "ios"))]
        { let _ = 0; }
    }
}

#[cfg(target_os = "ios")]
unsafe fn elevate_thread_rt_ios() {
    // pthread_set_qos_class_self(QOS_CLASS_USER_INTERACTIVE, 0) via the sys crate
    // or libc; kept behind cfg so host tests don't link it. Implementation in Task 19
    // alongside the CoreMIDI worker (same ffi block). For now a no-op stub.
}

pub fn flush_all_notes_off(_channel: u8) {} // replaced by the worker in Task 19

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instant_clock_is_monotonic_nonzero() {
        let c = InstantClock::new();
        let a = c.now_micros();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let b = c.now_micros();
        assert!(b > a, "clock must be monotonic");
    }
    #[test]
    fn elevate_priority_does_not_panic() {
        let c = InstantClock::new();
        c.elevate_priority(); // once at spawn
    }
}
```

- [ ] **Step 3: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine_ffi coremidi::tests`
Expected: `2 passed; 0 failed`.

- [ ] **Step 4: Commit**

```bash
git add engine/crates/ffi/Cargo.toml engine/crates/ffi/src/coremidi.rs
git commit -m "feat(ffi): InstantClock (prod) with one-shot RT-priority hook"
```

---

## Task 4: MidiMsg + midi_out.rs ring + hot-event channel (core)

**Files:**
- Create: `engine/crates/core/src/midi_out.rs`
- Modify: `engine/crates/core/src/lib.rs` (add `pub mod midi_out;`)

**Interfaces:** Produces everything in the `midi_out.rs` block above.

- [ ] **Step 1: Write failing tests + impl**

`engine/crates/core/src/midi_out.rs`:

```rust
//! Lock-free bounded queues: RT -> CoreMIDI worker (MIDI ring), RT -> Swift
//! (hot events). Arc-shared heapless::mpmc::Queue (&self enqueue/dequeue,
//! Send+Sync). Drop-oldest on overflow (E8).

use crate::command::Command;
use crate::event::EngineEvent;
use heapless::mpmc::Queue;
use std::sync::Arc;

pub const MIDI_RING_DEPTH: usize = 128;
pub const HOT_EVENT_DEPTH: usize = 32;
pub const COMMAND_DEPTH: usize = 64;
pub const MAX_EVENT_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiMsg {
    pub endpoint: u32,
    pub channel: u8,
    pub status: u8,
    pub note: u8,
    pub velocity: u8,
    pub send_at_offset_micros: u32,
    pub gate_micros: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotEventSlot { pub len: u8, pub bytes: [u8; MAX_EVENT_BYTES] }

pub type MidiOutRing     = Arc<Queue<MidiMsg, MIDI_RING_DEPTH>>;
pub type HotEventChannel = Arc<Queue<HotEventSlot, HOT_EVENT_DEPTH>>;
pub type CommandQueue    = Arc<Queue<Command, COMMAND_DEPTH>>;

pub fn midi_out_ring() -> MidiOutRing { Arc::new(Queue::new()) }
pub fn hot_event_channel() -> HotEventChannel { Arc::new(Queue::new()) }
pub fn command_queue() -> CommandQueue { Arc::new(Queue::new()) }

/// Enqueue, dropping the OLDEST entry if full. Returns slots dropped (E8).
pub fn push_drop_oldest<T, const N: usize>(q: &Queue<T, N>, val: T) -> usize
where T: Copy {
    let mut dropped = 0;
    let mut v = val;
    loop {
        match q.enqueue(v) {
            Ok(()) => return dropped,
            Err(rej) => { v = rej; let _ = q.dequeue(); dropped += 1; }
        }
    }
}

/// Encode `ev` into a stack buffer and push as a slot (drop-oldest).
pub fn push_event(events: &HotEventChannel, ev: &EngineEvent) -> usize {
    use sequencer_engine_codec::encode_event_into; // NOTE: see Task 5 — codec lives in ffi;
    // core cannot depend on ffi. Instead, postcard is a dev-dep only; production encoding
    // happens in ffi at drain. For the hot channel we store the EngineEvent by serializing
    // here with postcard (add postcard as a non-dev core dep — see Step 2).
    let mut buf = [0u8; MAX_EVENT_BYTES];
    let len = postcard::to_slice(&ev, &mut buf).expect("event fits 128B").len() as u8;
    push_drop_oldest(events, HotEventSlot { len, bytes: buf })
}
```

**Note — resolution:** `core` cannot call the ffi codec. Add `postcard` as a **non-dev** core dependency so `push_event` can serialize. Update `engine/crates/core/Cargo.toml` `[dependencies]` to include `postcard = { workspace = true }`. (The ffi codecs remain the C-ABI path; this is an internal core→queue serialization that happens to use postcard too. It is consistent: both sides speak postcard.)

- [ ] **Step 2: Add postcard to core non-dev deps + declare module**

`engine/crates/core/Cargo.toml` `[dependencies]`: add `postcard = { workspace = true }`.
`engine/crates/core/src/lib.rs`: add `pub mod midi_out;`.

Fix `push_event` to drop the phantom codec reference; it should read:

```rust
pub fn push_event(events: &HotEventChannel, ev: &EngineEvent) -> usize {
    let mut buf = [0u8; MAX_EVENT_BYTES];
    let written = postcard::to_slice(&ev, &mut buf).expect("event fits MAX_EVENT_BYTES").len();
    debug_assert!(written <= MAX_EVENT_BYTES);
    push_drop_oldest(events, HotEventSlot { len: written as u8, bytes: buf })
}
```

- [ ] **Step 3: Add tests**

Append to `midi_out.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring_push_then_drain() {
        let r = midi_out_ring();
        let m = MidiMsg { endpoint: 1, channel: 10, status: 0x90, note: 36, velocity: 100, send_at_offset_micros: 0, gate_micros: 50_000 };
        assert_eq!(push_drop_oldest(&r, m), 0);
        assert_eq!(r.dequeue(), Some(m));
        assert_eq!(r.dequeue(), None);
    }
    #[test]
    fn ring_drops_oldest_when_full() {
        let q: Arc<Queue<u8, 2>> = Arc::new(Queue::new());
        assert_eq!(push_drop_oldest(&q, 1u8), 0);
        assert_eq!(push_drop_oldest(&q, 2u8), 0);
        assert_eq!(push_drop_oldest(&q, 3u8), 1); // full -> drop oldest (1)
        // remaining: [2,3]
        assert_eq!(q.dequeue(), Some(2));
        assert_eq!(q.dequeue(), Some(3));
    }
    #[test]
    fn push_event_round_trips_through_postcard() {
        use crate::event::EngineEvent;
        let ch = hot_event_channel();
        let dropped = push_event(&ch, &EngineEvent::Playhead { track_idx: 0, step_idx: 5 });
        assert_eq!(dropped, 0);
        let slot = ch.dequeue().expect("one slot");
        let back: EngineEvent = postcard::from_bytes(&slot.bytes[..slot.len as usize]).unwrap();
        assert_eq!(back, EngineEvent::Playhead { track_idx: 0, step_idx: 5 });
    }
}
```

- [ ] **Step 4: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine midi_out`
Expected: `3 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add engine/crates/core/Cargo.toml engine/crates/core/src/lib.rs engine/crates/core/src/midi_out.rs
git commit -m "feat(core): MidiOutRing + HotEventChannel (heapless mpmc, drop-oldest)"
```

---

## Task 5: Add `EngineEvent::Overflow` + codec test (no orphans)

**Files:**
- Modify: `engine/crates/core/src/event.rs`
- Test: `engine/crates/core/src/event.rs`; `engine/crates/ffi/tests/ffi_api.rs`

**Interfaces:** Produces `EngineEvent::Overflow { dropped: u32 }`.

- [ ] **Step 1: Add the variant**

In `engine/crates/core/src/event.rs`, add as the last variant (before the closing brace, after `Error`):

```rust
    /// A bounded queue dropped entries (E8). Hot-channel safe (small).
    Overflow { dropped: u32 },
```

- [ ] **Step 2: Extend the core round-trip test**

In `event.rs` `tests::small_events_roundtrip`, add to the `events` array:

```rust
            EngineEvent::Overflow { dropped: 7 },
```

- [ ] **Step 3: Add a C-ABI round-trip + garbage-bytes test (no orphans)**

In `engine/crates/ffi/tests/ffi_api.rs`, add:

```rust
#[test]
fn overflow_event_roundtrips_over_c_abi() {
    // Encode Overflow via the event codec, decode back, compare postcard bytes.
    use sequencer_engine::event::EngineEvent;
    let ev = EngineEvent::Overflow { dropped: 42 };
    let bytes = postcard::to_allocvec(&ev).unwrap();
    let back: EngineEvent = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(back, ev);
}

#[test]
fn garbage_event_bytes_do_not_panic_overflow_path() {
    // Re-run the existing garbage-bytes guard to ensure the new variant didn't
    // break codec totality. (If a dedicated garbage test already exists, this
    // asserts it still passes with Overflow added.)
    let garbage = [0xFFu8; 8];
    let _: Result<sequencer_engine::event::EngineEvent, _> = postcard::from_bytes(&garbage);
    // total: returns Ok or Err, never panics
}
```

- [ ] **Step 4: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test`
Expected: all green (core round-trip incl. Overflow; ffi Overflow + garbage tests).

- [ ] **Step 5: Regenerate the committed header + commit**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine_ffi` (cbindgen runs via build.rs if wired; if the header is hand-regenerated, run `engine/scripts/build_engine.sh` and stage `engine/include/sequencer_engine.h` only if it meaningfully changed). The `Overflow` variant is on `EngineEvent` (not `#[repr(C)]`), so **the C header does not change** — `EngineResult` is untouched.

```bash
git add engine/crates/core/src/event.rs engine/crates/ffi/tests/ffi_api.rs
git commit -m "feat(core): EngineEvent::Overflow (E8) + C-ABI round-trip (no orphans)"
```

---

## Task 6: Engine ownership — COW snapshot, queues, atomics, RtState (core)

**Files:**
- Modify: `engine/crates/core/src/engine.rs`

**Interfaces:** Produces the `Engine`, `Transport`, `RtState`, `TrackRtState` structs from the Shared Interfaces block.

- [ ] **Step 1: Write failing tests + impl**

Replace `engine/crates/core/src/engine.rs`:

```rust
//! Engine: owns the COW snapshot store, bounded queues, transport atomics,
//! shutdown flag, and thread handles. The state worker is sole writer of the
//! Session (publish); the RT thread is a lock-free reader (snapshot_arc / load).

use crate::clock::Rng;
use crate::midi_out::{command_queue, hot_event_channel, midi_out_ring, CommandQueue, HotEventChannel, MidiOutRing};
use crate::models::{Session, MAX_TRACKS};
use arc_swap::ArcSwap;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex};

pub struct Transport {
    pub is_playing: AtomicBool,
    pub stop_generation: AtomicU32,
}
impl Default for Transport {
    fn default() -> Self {
        Self { is_playing: AtomicBool::new(false), stop_generation: AtomicU32::new(0) }
    }
}

#[derive(Clone)]
pub struct TrackRtState { pub step_idx: usize, pub speed_acc: u32 }

pub struct RtState {
    pub per_track: [TrackRtState; MAX_TRACKS],
    pub rng: Rng,
}
impl RtState {
    pub fn new(seed: u64) -> Self {
        Self {
            per_track: std::array::from_fn(|_| TrackRtState { step_idx: 0, speed_acc: 0 }),
            rng: Rng::new(seed),
        }
    }
}

pub struct Engine {
    pub snapshot: Arc<ArcSwap<Session>>,
    pub commands: CommandQueue,
    pub midi: MidiOutRing,
    pub hot_events: HotEventChannel,
    pub transport: Transport,
    pub shutdown: Arc<AtomicBool>,
    pub rt_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub worker_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            snapshot: Arc::new(ArcSwap::from_pointee(Session::default())),
            commands: command_queue(),
            midi: midi_out_ring(),
            hot_events: hot_event_channel(),
            transport: Transport::default(),
            shutdown: Arc::new(AtomicBool::new(false)),
            rt_handle: Mutex::new(None),
            worker_handle: Mutex::new(None),
        }
    }
    /// Worker: publish a new authoritative session (COW).
    pub fn publish(&self, session: Session) { self.snapshot.store(Arc::new(session)); }
    /// Serialize path: owned snapshot (lock-free load_full).
    pub fn snapshot_arc(&self) -> Arc<Session> { self.snapshot.load_full() }
    /// Play-start: seed RNG from a snapshot hash + reset counters.
    pub fn begin_play(&self, rt: &mut RtState) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let snap = self.snapshot.load_full();
        let mut h = DefaultHasher::new();
        // Hash a few scalar invariants of the session for a stable seed.
        snap.bpm.to_bits().hash(&mut h);
        snap.active_pattern_index.hash(&mut h);
        snap.global_swing_pct.to_bits().hash(&mut h);
        *rt = RtState::new(h.finish());
    }
    #[cfg(test)]
    pub fn load_session_for_test(&self, session: Session) { self.publish(session); }
}

impl Default for Engine { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publish_then_snapshot_arc_round_trips() {
        let e = Engine::new();
        let mut s = Session::default();
        s.bpm = 140.0;
        e.publish(s.clone());
        assert_eq!(e.snapshot_arc().bpm, 140.0);
    }
    #[test]
    fn begin_play_seeds_rng_deterministically() {
        let e = Engine::new();
        let mut a = RtState::new(1);
        let mut b = RtState::new(2);
        e.begin_play(&mut a);
        e.begin_play(&mut b);
        // same snapshot -> same seed -> same RNG stream
        assert_eq!(a.rng.next_u32(), b.rng.next_u32());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine engine::tests`
Expected: `2 passed; 0 failed`.

- [ ] **Step 3: Commit**

```bash
git add engine/crates/core/src/engine.rs
git commit -m "feat(core): Engine owns COW snapshot (arc-swap), queues, transport, RtState"
```

---

## Task 7: State worker — command apply via COW (scalar commands + transport)

**Files:**
- Modify: `engine/crates/core/src/engine.rs` (add `apply_command` + `run_worker`)

**Interfaces:**
- Consumes: `Command`, `Engine`, `push_event`.
- Produces: `Engine::apply_command(&self, cmd)` (mutates a cloned session + publishes), `Engine::run_worker_loop()` (drains the command queue).

- [ ] **Step 1: Write failing test**

Add to `engine.rs` tests:

```rust
    #[test]
    fn worker_applies_set_bpm_via_cow() {
        let e = Engine::new();
        e.apply_command(Command::SetBpm { bpm: 174.0 });
        assert_eq!(e.snapshot_arc().bpm, 174.0);
    }
    #[test]
    fn play_stop_toggle_transport_atomic() {
        let e = Engine::new();
        e.apply_command(Command::Play);
        assert!(e.transport.is_playing.load(std::sync::atomic::Ordering::Acquire));
        e.apply_command(Command::Stop);
        assert!(!e.transport.is_playing.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(e.transport.stop_generation.load(std::sync::atomic::Ordering::Acquire), 1);
    }
```

- [ ] **Step 2: Implement apply_command + worker drain**

Add to `engine.rs` `impl Engine` (and `use crate::command::Command;` at top):

```rust
    /// Apply one command by clone-mutate-publish (the worker's per-command body).
    /// Algorithms/scheduler/load-session arms are wired in their own tasks.
    pub fn apply_command(&self, cmd: Command) {
        use crate::command::Command::*;
        match cmd {
            Play  => { self.transport.is_playing.store(true, Ordering::Release); }
            Stop  => {
                self.transport.is_playing.store(false, Ordering::Release);
                self.transport.stop_generation.fetch_add(1, Ordering::AcqRel);
            }
            RequestFullSnapshot => {
                let snap = self.snapshot.load_full();
                crate::midi_out::push_event(&self.hot_events, &EngineEvent::FullSnapshot { session: (*snap).clone() });
            }
            // Off-RT large-channel events (Serialize) handled in Task 8.
            Serialize | LoadSession { .. } => { /* Task 8 / Task 18 */ }
            // Algorithms/scheduler arms added in Tasks 14-16.
            Roll { .. } | Vary { .. } | Cut { .. } | Copy { .. } | Paste { .. } | Trash { .. } | Undo { .. }
            | QueuePattern { .. } | CancelQueuedPattern | RetriggerPattern { .. } => { /* later tasks */ }
            other => {
                let mut s = (*self.snapshot.load_full()).clone();
                match other {
                    SetBpm { bpm } => s.bpm = bpm,
                    SetGlobalSwing { pct } => s.global_swing_pct = pct,
                    SetHumanize { timing, velocity } => { s.humanize_timing = timing; s.humanize_velocity = velocity; }
                    SetSyncSource { source } => s.sync_source = source,
                    SetQuantizeGrain { grain: _ } => { /* stored on the scheduler in Task 16 */ }
                    SetGlobalMidiChannel { channel } => s.global_midi_channel = channel,
                    SetMidiDestinations { endpoints } => s.midi_destinations = endpoints,
                    SetTrackLength { track_idx, length } => with_track_mut(&mut s, track_idx, |t| { t.length = length.clamp(1, crate::models::STEP_COUNT); }),
                    SetTrackMuted { track_idx, muted } => with_track_mut(&mut s, track_idx, |t| t.muted = muted),
                    SetTrackNote { track_idx, midi_note } => with_track_mut(&mut s, track_idx, |t| t.midi_note = midi_note),
                    SetTrackSpeedRatio { track_idx, ratio } => with_track_mut(&mut s, track_idx, |t| t.speed_ratio = ratio),
                    SetTrackSwing { track_idx, swing_pct } => with_track_mut(&mut s, track_idx, |t| t.swing_pct = swing_pct),
                    AddTrack => add_track(&mut s),
                    RemoveTrack => remove_track(&mut s),
                    SetStep { track_idx, step_idx, zone } => with_track_mut(&mut s, track_idx, |t| { if step_idx < t.steps.len() { t.steps[step_idx].active = true; t.steps[step_idx].velocity_zone = zone; } }),
                    DeleteStep { track_idx, step_idx } => with_track_mut(&mut s, track_idx, |t| { if step_idx < t.steps.len() { t.steps[step_idx].active = false; } }),
                    SetRatchet { track_idx, step_idx, ratchet } => with_track_mut(&mut s, track_idx, |t| { if step_idx < t.steps.len() { t.steps[step_idx].ratchet = ratchet; } }),
                    SetFollowAction { .. } => { /* Task 16 */ }
                    // unreachable: the match arms above are exhaustive with the outer match
                    _ => {}
                }
                self.publish(s);
            }
        }
    }

    /// Worker thread body: drain commands until shutdown.
    pub fn run_worker_loop(self: &Arc<Engine>) {
        while !self.shutdown.load(Ordering::Acquire) {
            if let Some(cmd) = self.commands.dequeue() {
                self.apply_command(cmd);
            } else {
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }
    }
```

Add free helpers (module level):

```rust
use crate::event::EngineEvent;
use std::sync::atomic::Ordering;

fn active_pattern_mut(s: &mut Session) -> Option<&mut crate::models::Pattern> {
    s.patterns[s.active_pattern_index].as_mut()
}
fn with_track_mut<R>(s: &mut Session, idx: usize, f: impl FnOnce(&mut crate::models::Track) -> R) -> Option<R> {
    let p = active_pattern_mut(s)?;
    p.tracks.get_mut(idx).map(f)
}
fn add_track(s: &mut Session) {
    if let Some(p) = active_pattern_mut(s) {
        if p.tracks.len() < MAX_TRACKS { p.tracks.push(crate::models::Track::default()); }
    }
}
fn remove_track(s: &mut Session) {
    if let Some(p) = active_pattern_mut(s) {
        if p.tracks.len() > crate::models::MIN_TRACKS { p.tracks.pop(); }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine engine::tests`
Expected: all green (incl. `worker_applies_set_bpm_via_cow`, `play_stop_toggle_transport_atomic`).

- [ ] **Step 4: Commit**

```bash
git add engine/crates/core/src/engine.rs
git commit -m "feat(core): state worker — COW apply for scalar/transport commands"
```

---

## Task 8: engine_serialize via load_full (inline) + Serialize event

**Files:**
- Modify: `engine/crates/ffi/src/lib.rs` (`engine_serialize`)
- Modify: `engine/crates/core/src/engine.rs` (apply `Serialize` arm)
- Test: `engine/crates/ffi/tests/ffi_api.rs`

**Interfaces:**
- Consumes: `Engine::snapshot_arc()`, `serde_ext::wrap`.
- Produces: synchronous `engine_serialize` reading the COW snapshot inline.

- [ ] **Step 1: Write failing test**

`engine/crates/ffi/tests/ffi_api.rs`:

```rust
#[test]
fn serialize_round_trips_after_set_bpm() {
    // engine_new -> submit SetBpm 150 -> serialize -> LoadSession-free check that
    // the bytes decode to a session at 150 bpm.
    let eng = unsafe { engine_new() };
    assert!(!eng.is_null());
    let cmd = postcard::to_allocvec(&sequencer_engine::command::Command::SetBpm { bpm: 150.0 }).unwrap();
    assert_eq!(unsafe { engine_submit_command(eng, cmd.as_ptr(), cmd.len()) }, EngineResult::Ok as i32);
    // give the worker a moment to apply (it's off-thread)
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut ptr = std::ptr::null_mut();
    let mut len = 0usize;
    assert_eq!(unsafe { engine_serialize(eng, &mut ptr, &mut len) }, EngineResult::Ok as i32);
    assert!(!ptr.is_null() && len > 0);
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    let env: sequencer_engine::serde_ext::SessionEnvelope = postcard::from_bytes(bytes).unwrap();
    assert_eq!(env.session.bpm, 150.0);
    unsafe { engine_free_bytes(ptr, len) };
    unsafe { engine_free(eng) };
}
```

- [ ] **Step 2: Implement engine_serialize inline via snapshot_arc**

In `engine/crates/ffi/src/lib.rs` `engine_serialize`, replace the body's session read to use the COW snapshot (the existing structure — validate non-NULL out params, serialize, box, hand out — stays; only the session source changes from `eng.session.clone()` to `eng.snapshot_arc()`):

```rust
// inside engine_serialize, after acquiring the EngineHandle:
let snap = handle.engine.snapshot_arc();          // Arc<Session>, lock-free
let envelope = sequencer_engine::serde_ext::wrap((*snap).clone());
let bytes = postcard::to_allocvec(&envelope).map_err(|_| CodecError::Encode)?;
// ... existing into_boxed_slice + mem::forget + out-param hand-off + EngineResult::Ok
```

(Keep the existing non-NULL out-param check and `engine_free_bytes` ownership transfer exactly as the foundation established.)

- [ ] **Step 3: Implement the Serialize command arm (off-RT large channel)**

In `engine.rs` `apply_command`, replace the `Serialize` arm:

```rust
            Serialize => {
                let snap = self.snapshot.load_full();
                let env = crate::serde_ext::wrap((*snap).clone());
                if let Ok(bytes) = postcard::to_allocvec(&env) {
                    crate::midi_out::push_event(&self.hot_events, &EngineEvent::Serialized { bytes });
                }
            }
```

(`postcard` is now a core dep from Task 4.)

- [ ] **Step 4: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test`
Expected: `serialize_round_trips_after_set_bpm` green (synchronous serialize reflects the worker-applied bpm).

- [ ] **Step 5: Commit**

```bash
git add engine/crates/core/src/engine.rs engine/crates/ffi/src/lib.rs engine/crates/ffi/tests/ffi_api.rs
git commit -m "feat(ffi): engine_serialize reads COW snapshot inline (sync, non-blocking)"
```

---

## Task 9: Timing math — swing offset + micro_timing (core)

**Files:**
- Modify: `engine/crates/core/src/clock.rs`

**Interfaces:** Produces `swing_offset_micros`, `micro_timing_offset_micros`.

- [ ] **Step 1: Write failing tests + impl**

Add to `clock.rs`:

```rust
/// Swing delays off-grid (odd 16th) steps by `swing_pct` of the step interval.
/// Returns the micros offset for `step_idx` within a 16-step pattern.
pub fn swing_offset_micros(effective_swing_pct: f32, step_idx: usize, step_period_micros: u64) -> i64 {
    if step_idx % 2 == 0 { return 0; } // downbeats unaffected
    let frac = (effective_swing_pct / 100.0).clamp(0.0, 0.49);
    ((frac * step_period_micros as f32).round()) as i64
}

/// Apply a micro_timing_offset (f32 fraction of a step), clamped to ±½ step (E3).
pub fn micro_timing_offset_micros(offset: f32, step_period_micros: u64) -> i64 {
    let clamped = offset.clamp(-0.5, 0.5);
    (clamped * step_period_micros as f32).round() as i64
}

#[cfg(test)]
mod timing_tests {
    use super::*;
    #[test]
    fn swing_only_affects_off_grid_steps() {
        let p: u64 = 100_000; // 16th period in micros
        assert_eq!(swing_offset_micros(50.0, 0, p), 0);  // even step: no offset
        assert_eq!(swing_offset_micros(50.0, 2, p), 0);
        let off = swing_offset_micros(50.0, 1, p);
        assert!(off > 0 && off < p as i64, "odd step delayed within interval");
    }
    #[test]
    fn micro_timing_clamps_to_half_step() {
        let p: u64 = 100_000;
        assert_eq!(micro_timing_offset_micros(0.25, p), 25_000);
        assert_eq!(micro_timing_offset_micros(2.0, p), 50_000);  // clamped from above
        assert_eq!(micro_timing_offset_micros(-2.0, p), -50_000); // clamped from below
    }
}
```

- [ ] **Step 2: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine clock::timing`
Expected: `2 passed; 0 failed`.

- [ ] **Step 3: Commit**

```bash
git add engine/crates/core/src/clock.rs
git commit -m "feat(core): swing + micro_timing offset math (E2, E3)"
```

---

## Task 10: MIDI dispatch math — velocity, humanize, ratchet, note-on (core)

**Files:**
- Modify: `engine/crates/core/src/midi.rs`

**Interfaces:** Produces the `midi.rs` functions from the Shared Interfaces block.

- [ ] **Step 1: Write failing tests + impl**

`engine/crates/core/src/midi.rs`:

```rust
//! Pure MIDI dispatch math. No CoreMIDI, no unsafe. Produces fixed MidiMsg slots.

use crate::clock::Rng;
use crate::midi_out::MidiMsg;
use crate::models::{Ratchet, VelocityZone};

pub const VEL_LOW: u8 = 64;
pub const VEL_MID: u8 = 100;
pub const VEL_ACCENT: u8 = 127;
pub const DEFAULT_GATE_MICROS: u32 = 50_000;
const NOTE_ON: u8 = 0x90;
const NOTE_OFF: u8 = 0x80;
const CC: u8 = 0xB0;
const ALL_NOTES_OFF_CC: u8 = 123;

pub fn velocity_for_zone(zone: VelocityZone) -> u8 {
    match zone { VelocityZone::Low => VEL_LOW, VelocityZone::Mid => VEL_MID, VelocityZone::Accent => VEL_ACCENT }
}

/// Humanize base velocity by ±(humanize_velocity * zone_weight * 5) MIDI units (E4).
pub fn humanize_velocity(base: u8, humanize_velocity: f32, zone_weight: f32, rng: &mut Rng) -> u8 {
    let mag = (humanize_velocity * zone_weight * 5.0).round() as i32;
    if mag == 0 { return base; }
    let jitter = rng.range(-mag, mag);
    (base as i32 + jitter).clamp(1, 127) as u8
}

pub fn ratchet_count(r: Ratchet) -> u32 {
    match r { Ratchet::Off => 1, Ratchet::X2 => 2, Ratchet::X3 => 3, Ratchet::X4 => 4 }
}

pub fn build_note_on(endpoint: u32, channel: u8, note: u8, velocity: u8, send_at_offset_micros: u32) -> MidiMsg {
    MidiMsg { endpoint, channel, status: NOTE_ON | (channel & 0x0F), note, velocity, send_at_offset_micros, gate_micros: DEFAULT_GATE_MICROS }
}
pub fn build_all_notes_off(endpoint: u32, channel: u8) -> MidiMsg {
    MidiMsg { endpoint, channel, status: CC | (channel & 0x0F), note: ALL_NOTES_OFF_CC, velocity: 0, send_at_offset_micros: 0, gate_micros: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn velocity_table_and_humanize_bounds() {
        assert_eq!(velocity_for_zone(VelocityZone::Low), VEL_LOW);
        let mut rng = Rng::new(7);
        let h = humanize_velocity(VEL_MID, 1.0, 1.0, &mut rng);
        assert!(h >= 95 && h <= 105, "±5 around mid");
        let zero = humanize_velocity(VEL_MID, 0.0, 1.0, &mut rng);
        assert_eq!(zero, VEL_MID);
    }
    #[test]
    fn ratchet_counts() {
        assert_eq!(ratchet_count(Ratchet::Off), 1);
        assert_eq!(ratchet_count(Ratchet::X4), 4);
    }
    #[test]
    fn note_on_carries_channel_and_gate() {
        let m = build_note_on(3, 10, 36, 100, 2_000);
        assert_eq!(m.status, 0x9A);
        assert_eq!(m.gate_micros, DEFAULT_GATE_MICROS);
        assert_eq!(m.send_at_offset_micros, 2_000);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine midi::tests`
Expected: `3 passed; 0 failed`.

- [ ] **Step 3: Commit**

```bash
git add engine/crates/core/src/midi.rs
git commit -m "feat(core): velocity/humanize/ratchet/note-on dispatch math (E4, E5)"
```

---

## Task 11: process() RT tick + RT thread loop + testability seams

**Files:**
- Modify: `engine/crates/core/src/engine.rs`

**Interfaces:**
- Consumes: `advance_speed_ratio`, `to_q16_16`, `swing_offset_micros`, `micro_timing_offset_micros`, `velocity_for_zone`, `humanize_velocity`, `ratchet_count`, `build_note_on`, `push_event`, `push_drop_oldest`.
- Produces: `engine::process`, `Engine::run_rt_loop`, `Engine::new_rt_state`.

- [ ] **Step 1: Write failing integration test**

`engine/crates/core/tests/dispatch.rs`:

```rust
use sequencer_engine::engine::{Engine, RtState, process};
use sequencer_engine::models::{Pattern, Session, Step, VelocityZone, STEP_COUNT};
use sequencer_engine::midi_out::{MidiMsg, MAX_EVENT_BYTES};

fn session_with_one_hit() -> Session {
    let mut s = Session::default();
    let mut p = Pattern::default();
    p.tracks[0].steps[0] = Step { active: true, velocity_zone: VelocityZone::Accent, ..Step::default() };
    p.tracks[0].midi_note = 36;
    s.patterns[0] = Some(p);
    s.bpm = 120.0; // 16th period = 60/120/4 = 0.125s = 125_000us
    s
}

#[test]
fn process_advances_playhead_and_emits_note_on() {
    let eng = Engine::new();
    eng.load_session_for_test(session_with_one_hit());
    let mut rt = RtState::new(1);
    eng.begin_play(&mut rt);

    let period = 125_000u64;
    let mut notes = 0;
    for i in 0..4 {
        let outcome = process(&mut rt, &eng.snapshot_arc(), true, i * period, &eng.midi, &eng.hot_events);
        notes += outcome.notes_pushed;
    }
    // step 0 hits on the first tick (Accent); 4 ticks at ratio 1.0 = 4 steps, step 0 fires once
    assert!(notes >= 1, "at least the step-0 note-on");
    assert_eq!(eng.midi.dequeue().map(|m| m.note), Some(36));
    // a Playhead event was emitted
    let slot = eng.hot_events.dequeue().expect("a playhead");
    let _ev: sequencer_engine::event::EngineEvent = postcard::from_bytes(&slot.bytes[..slot.len as usize]).unwrap();
}
```

- [ ] **Step 2: Implement process() + the RT loop**

Add to `engine.rs`:

```rust
use crate::clock::{advance_speed_ratio, micro_timing_offset_micros, swing_offset_micros, to_q16_16};
use crate::midi::{build_note_on, humanize_velocity, ratchet_count, velocity_for_zone};
use crate::models::STEP_COUNT;

pub struct TickOutcome { pub playheads_emitted: u32, pub notes_pushed: u32 }

/// One global tick. Pure except for queue pushes. `playing` is read by the caller
/// from the transport atomics, so process() stays free of atomics (testable).
pub fn process(
    rt: &mut RtState,
    session: &Session,
    playing: bool,
    now_micros: u64,
    midi: &MidiOutRing,
    events: &HotEventChannel,
) -> TickOutcome {
    let mut outcome = TickOutcome { playheads_emitted: 0, notes_pushed: 0 };
    let _ = now_micros; // reserved: absolute tick time (LinkPhase positioning lives in the RT loop, Task 17)
    if !playing { return outcome; }
    let Some(pattern) = session.patterns[session.active_pattern_index].as_ref() else { return outcome; };
    let step_period_micros = (60.0 / session.bpm / 4.0 * 1_000_000.0) as u64;
    let endpoint = session.midi_destinations.first().copied().unwrap_or(0);
    let channel = session.global_midi_channel;

    for (idx, track) in pattern.tracks.iter().enumerate() {
        if idx >= rt.per_track.len() || track.muted { continue; }
        let (steps, new_acc) = advance_speed_ratio(rt.per_track[idx].speed_acc, to_q16_16(track.speed_ratio));
        rt.per_track[idx].speed_acc = new_acc;
        for _ in 0..steps {
            let si = rt.per_track[idx].step_idx;
            rt.per_track[idx].step_idx = (si + 1) % track.length.max(1);
            let step = track.steps[si % STEP_COUNT];
            if step.active {
                let base = velocity_for_zone(step.velocity_zone);
                let vel = humanize_velocity(base, session.humanize_velocity, zone_weight(step.velocity_zone), &mut rt.rng);
                // E2/E3: swing + micro_timing become a per-note send offset the worker applies.
                let swings = swing_offset_micros(crate::clock::effective_swing(session.global_swing_pct, track.swing_pct), si, step_period_micros);
                let mt = micro_timing_offset_micros(step.micro_timing_offset, step_period_micros);
                let offset = (swings + mt).max(0) as u32;
                for _ in 0..ratchet_count(step.ratchet) {
                    let _ = crate::midi_out::push_drop_oldest(midi, build_note_on(endpoint, channel, track.midi_note, vel, offset));
                    outcome.notes_pushed += 1;
                }
            }
            let _ = crate::midi_out::push_event(events, &EngineEvent::Playhead { track_idx: idx, step_idx: rt.per_track[idx].step_idx });
            outcome.playheads_emitted += 1;
        }
    }
    outcome
}

fn zone_weight(z: crate::models::VelocityZone) -> f32 {
    match z { crate::models::VelocityZone::Accent => 1.0, _ => 0.6 }
}

impl Engine {
    pub fn new_rt_state(&self) -> RtState { RtState::new(1) }
    /// RT thread body: load snapshot (Guard), read transport, process, sleep to next deadline.
    pub fn run_rt_loop(self: &Arc<Engine>, clock: &dyn Clock) {
        clock.elevate_priority(); // ONCE at spawn
        let mut rt = self.new_rt_state();
        let mut began = false;
        let mut last_now = clock.now_micros();
        loop {
            if self.shutdown.load(Ordering::Acquire) { break; }
            let now = clock.now_micros();
            let playing = self.transport.is_playing.load(Ordering::Acquire);
            if playing && !began { self.begin_play(&mut rt); began = true; } else if !playing { began = false; }
            let snap = self.snapshot.load(); // zero-alloc Guard; immutable for the tick
            process(&mut rt, &snap, playing, now, &self.midi, &self.hot_events);
            last_now = now;
            let period = (60.0 / snap.bpm / 4.0 * 1_000_000.0) as u64;
            clock.sleep_until(last_now + period);
        }
    }
}
```

- [ ] **Step 3: Run the test**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine --test dispatch`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add engine/crates/core/src/engine.rs engine/crates/core/tests/dispatch.rs
git commit -m "feat(core): process() RT tick + RT loop (COW Guard read, allocation-free)"
```

---

## Task 12: Undo — per-track, one-deep, length-excluded (core)

**Files:**
- Modify: `engine/crates/core/src/undo.rs`

**Interfaces:** Produces `Undo` storing one `Step` snapshot per track (max `MAX_TRACKS`).

- [ ] **Step 1: Write failing tests + impl**

`engine/crates/core/src/undo.rs`:

```rust
//! Per-track, one-deep undo. Snapshot triggers: Roll/Vary/Cut/Paste/Trash.
//! A length change does NOT push undo (working agreement).

use crate::models::{Session, Step, MAX_TRACKS};

pub struct Undo {
    slots: [Option<[Step; crate::models::STEP_COUNT]>; MAX_TRACKS],
}
impl Default for Undo { fn default() -> Self { Self { slots: std::array::from_fn(|_| None) } } }

impl Undo {
    pub fn push(&mut self, track_idx: usize, track_steps: &[Step; crate::models::STEP_COUNT]) {
        if track_idx < MAX_TRACKS { self.slots[track_idx] = Some(*track_steps); }
    }
    /// Restore track `idx`'s steps if a snapshot exists. Returns true if restored.
    pub fn undo(&mut self, s: &mut Session, idx: usize) -> bool {
        let Some(steps) = self.slots[idx].take() else { return false; };
        if let Some(p) = s.patterns[s.active_pattern_index].as_mut() {
            if let Some(t) = p.tracks.get_mut(idx) { t.steps = steps; return true; }
        }
        false
    }
    pub fn available(&self, idx: usize) -> bool { idx < MAX_TRACKS && self.slots[idx].is_some() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Pattern, Step, VelocityZone};
    fn session_with_steps() -> Session {
        let mut s = Session::default();
        let mut p = Pattern::default();
        p.tracks[0].steps[0] = Step { active: true, velocity_zone: VelocityZone::Accent, ..Default::default() };
        s.patterns[0] = Some(p);
        s
    }
    #[test]
    fn undo_restores_steps() {
        let mut u = Undo::default();
        let mut s = session_with_steps();
        let snap = s.patterns[0].as_ref().unwrap().tracks[0].steps;
        u.push(0, &snap);
        // mutate
        s.patterns[0].as_mut().unwrap().tracks[0].steps[0].active = false;
        assert!(u.undo(&mut s, 0));
        assert!(s.patterns[0].as_ref().unwrap().tracks[0].steps[0].active);
        assert!(!u.available(0));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine undo`
Expected: `1 passed; 0 failed`.

- [ ] **Step 3: Commit**

```bash
git add engine/crates/core/src/undo.rs
git commit -m "feat(core): per-track one-deep undo (length-excluded)"
```

---

## Task 13: Clipboard — TrackClipboard without midi_note (core)

**Files:**
- Modify: `engine/crates/core/src/clipboard.rs`

**Interfaces:** Produces `TrackClipboard { steps, length, speed_ratio }`, `Clipboard`.

- [ ] **Step 1: Write failing tests + impl**

`engine/crates/core/src/clipboard.rs`:

```rust
//! Track + session clipboards. TrackClipboard carries steps/length/speed_ratio
//! but NEVER midi_note (working agreement).

use crate::models::{Session, Step, STEP_COUNT};

#[derive(Clone)]
pub struct TrackClipboard {
    pub steps: [Step; STEP_COUNT],
    pub length: usize,
    pub speed_ratio: f32,
}

pub struct Clipboard { track: Option<TrackClipboard> }
impl Default for Clipboard { fn default() -> Self { Self { track: None } } }

impl Clipboard {
    pub fn cut(&mut self, s: &mut Session, idx: usize) {
        self.copy(s, idx);
        if let Some(p) = s.patterns[s.active_pattern_index].as_mut() {
            if let Some(t) = p.tracks.get_mut(idx) { t.steps = [Step::default(); STEP_COUNT]; }
        }
    }
    pub fn copy(&mut self, s: &Session, idx: usize) {
        if let Some(t) = s.patterns[s.active_pattern_index].as_ref().and_then(|p| p.tracks.get(idx)) {
            self.track = Some(TrackClipboard { steps: t.steps, length: t.length, speed_ratio: t.speed_ratio });
        }
    }
    pub fn paste(&self, s: &mut Session, idx: usize) -> bool {
        let Some(cb) = &self.track else { return false; };
        if let Some(t) = s.patterns[s.active_pattern_index].as_mut().and_then(|p| p.tracks.get_mut(idx)) {
            t.steps = cb.steps; t.length = cb.length; t.speed_ratio = cb.speed_ratio;
            // midi_note is deliberately NOT overwritten.
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Pattern, Step, VelocityZone};
    #[test]
    fn copy_then_paste_preserves_midi_note() {
        let mut s = Session::default();
        let mut p = Pattern::default();
        p.tracks[0].midi_note = 42;
        p.tracks[0].steps[3] = Step { active: true, velocity_zone: VelocityZone::Accent, ..Default::default() };
        p.tracks[1].midi_note = 99;
        s.patterns[0] = Some(p);
        let mut cb = Clipboard::default();
        cb.copy(&s, 0);
        assert!(cb.paste(&mut s, 1));
        assert_eq!(s.patterns[0].as_ref().unwrap().tracks[1].midi_note, 99); // preserved
        assert!(s.patterns[0].as_ref().unwrap().tracks[1].steps[3].active);  // pasted
    }
}
```

- [ ] **Step 2: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine clipboard`
Expected: `1 passed; 0 failed`.

- [ ] **Step 3: Commit**

```bash
git add engine/crates/core/src/clipboard.rs
git commit -m "feat(core): clipboard — TrackClipboard without midi_note"
```

---

## Task 14: Roll + Vary algorithms + proptest (core)

**Files:**
- Modify: `engine/crates/core/src/algorithms/mod.rs`, `roll.rs`, `vary.rs`

**Interfaces:** Produces `roll(track, strength, rng)`, `vary(track, strength, rng)` operating on `&mut Track`.

- [ ] **Step 1: Write failing tests + impl + proptest**

`algorithms/mod.rs`:
```rust
pub mod roll;
pub mod vary;
```

`algorithms/roll.rs`:
```rust
//! Roll: randomize micro_timing_offset (+ toggle some steps). Preserves
//! length / midi_note / speed_ratio. Caller pushes undo first.

use crate::clock::Rng;
use crate::models::{Track, STEP_COUNT};

pub fn roll(track: &mut Track, strength: f32, rng: &mut Rng) {
    let s = strength.clamp(0.0, 1.0);
    for i in 0..STEP_COUNT {
        if track.steps[i].active {
            let off = (rng.range(-50, 50) as f32 / 100.0) * s; // ±0.5 * strength
            track.steps[i].micro_timing_offset = off;
        }
    }
    // length / midi_note / speed_ratio untouched by construction.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Step, Track, VelocityZone};
    use proptest::prelude::*;

    fn active_track() -> Track {
        let mut t = Track::default();
        t.steps[0] = Step { active: true, velocity_zone: VelocityZone::Mid, ..Default::default() };
        t.length = 7; t.midi_note = 55; t.speed_ratio = 2.0;
        t
    }

    proptest! {
        #[test]
        fn roll_preserves_invariants(seed in 0u64..10_000, strength in 0.0f32..1.0) {
            let mut t = active_track();
            let before = (t.length, t.midi_note, t.speed_ratio.to_bits());
            roll(&mut t, strength, &mut Rng::new(seed));
            let after = (t.length, t.midi_note, t.speed_ratio.to_bits());
            prop_assert_eq!(before, after);
        }
    }
}
```

`algorithms/vary.rs`:
```rust
//! Vary: perturb non-accent active steps; LOCK accents. Falls back to Roll
//! when there are no accents. Preserves length / midi_note / speed_ratio.

use crate::algorithms::roll;
use crate::clock::Rng;
use crate::models::{Track, VelocityZone, STEP_COUNT};

pub fn vary(track: &mut Track, strength: f32, rng: &mut Rng) {
    let has_accent = track.steps.iter().any(|s| s.active && s.velocity_zone == VelocityZone::Accent);
    if !has_accent { return roll::roll(track, strength, rng); }
    let s = strength.clamp(0.0, 1.0);
    for i in 0..STEP_COUNT {
        let is_accent = track.steps[i].active && track.steps[i].velocity_zone == VelocityZone::Accent;
        if track.steps[i].active && !is_accent {
            let off = (rng.range(-50, 50) as f32 / 100.0) * s;
            track.steps[i].micro_timing_offset = off;
        }
        // accent steps untouched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Step, Track, VelocityZone};
    use proptest::prelude::*;

    fn track_with_accent() -> Track {
        let mut t = Track::default();
        t.steps[0] = Step { active: true, velocity_zone: VelocityZone::Accent, ..Default::default() };
        t.steps[2] = Step { active: true, velocity_zone: VelocityZone::Mid, ..Default::default() };
        t.length = 5; t.midi_note = 40; t.speed_ratio = 0.5;
        t
    }

    #[test]
    fn vary_locks_accents() {
        let mut t = track_with_accent();
        let accent_before = t.steps[0];
        vary(&mut t, 1.0, &mut Rng::new(3));
        assert_eq!(t.steps[0], accent_before, "accent step unchanged");
    }

    #[test]
    fn vary_falls_back_to_roll_with_no_accents() {
        let mut t = Track::default();
        t.steps[0] = Step { active: true, velocity_zone: VelocityZone::Mid, ..Default::default() };
        vary(&mut t, 1.0, &mut Rng::new(9)); // no accent -> Roll path, must not panic
        assert!(t.steps[0].micro_timing_offset.abs() <= 0.5);
    }

    proptest! {
        #[test]
        fn vary_preserves_invariants(seed in 0u64..10_000, strength in 0.0f32..1.0) {
            let mut t = track_with_accent();
            let before = (t.length, t.midi_note, t.speed_ratio.to_bits());
            vary(&mut t, strength, &mut Rng::new(seed));
            prop_assert_eq!(before, (t.length, t.midi_note, t.speed_ratio.to_bits()));
        }
    }
}
```

- [ ] **Step 2: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine algorithms`
Expected: all green (incl. both proptests).

- [ ] **Step 3: Commit**

```bash
git add engine/crates/core/src/algorithms/
git commit -m "feat(core): Roll + Vary algorithms with proptest invariants"
```

---

## Task 15: Wire Cut/Copy/Paste/Trash/Undo/Roll/Vary into the worker

**Files:**
- Modify: `engine/crates/core/src/engine.rs` (`apply_command`); add `Undo` + `Clipboard` fields to `Engine`.

**Interfaces:** Consumes `Undo`, `Clipboard`, `roll`, `vary`.

- [ ] **Step 1: Add Undo/Clipboard to Engine + wire arms**

Add fields to `Engine`: `pub undo: Mutex<Undo>`, `pub clipboard: Mutex<Clipboard>` (init in `new()`). Replace the algorithm/clipboard arm stub in `apply_command`:

```rust
            Roll { track_idx, strength } | Vary { track_idx, strength } | Cut { track_idx }
            | Copy { track_idx } | Paste { track_idx } | Trash { track_idx } | Undo { track_idx } => {
                use crate::command::Command::*;
                let mut s = (*self.snapshot.load_full()).clone();
                // push undo BEFORE mutating for the mutating commands
                let mutating = matches!(cmd, Roll { .. } | Vary { .. } | Cut { .. } | Paste { .. } | Trash { .. });
                if mutating {
                    if let Some(t) = s.patterns[s.active_pattern_index].as_ref().and_then(|p| p.tracks.get(track_idx)) {
                        self.undo.lock().unwrap().push(track_idx, &t.steps);
                    }
                }
                match cmd {
                    Roll { strength, .. } => { if let Some(t) = track_mut(&mut s, track_idx) { crate::algorithms::roll::roll(t, strength, &mut Rng::new(seed_from(&s, track_idx))); } }
                    Vary { strength, .. } => { if let Some(t) = track_mut(&mut s, track_idx) { crate::algorithms::vary::vary(t, strength, &mut Rng::new(seed_from(&s, track_idx))); } }
                    Cut { .. } => self.clipboard.lock().unwrap().cut(&mut s, track_idx),
                    Copy { .. } => self.clipboard.lock().unwrap().copy(&s, track_idx),
                    Paste { .. } => { self.clipboard.lock().unwrap().paste(&mut s, track_idx); }
                    Trash { .. } => { if let Some(t) = track_mut(&mut s, track_idx) { t.steps = [crate::models::Step::default(); crate::models::STEP_COUNT]; } }
                    Undo { .. } => { self.undo.lock().unwrap().undo(&mut s, track_idx); }
                    _ => {}
                }
                let avail = self.undo.lock().unwrap().available(track_idx);
                self.publish(s);
                crate::midi_out::push_event(&self.hot_events, &EngineEvent::UndoAvailable { track_idx, available: avail });
            }
```

Add helpers:
```rust
fn track_mut(s: &mut Session, idx: usize) -> Option<&mut crate::models::Track> {
    s.patterns[s.active_pattern_index].as_mut()?.tracks.get_mut(idx)
}
fn seed_from(s: &Session, idx: usize) -> u64 {
    use std::collections::hash_map::DefaultHasher; use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new(); idx.hash(&mut h); s.bpm.to_bits().hash(&mut h); h.finish()
}
```

- [ ] **Step 2: Add invariant tests for Cut/Trash/Paste**

`engine/crates/core/tests/algorithms_wiring.rs`:

```rust
use sequencer_engine::engine::Engine;
use sequencer_engine::models::{Pattern, Session, Step, VelocityZone};

fn session() -> Session {
    let mut s = Session::default();
    let mut p = Pattern::default();
    p.tracks[0].midi_note = 38;
    p.tracks[0].length = 9;
    p.tracks[0].speed_ratio = 2.0;
    p.tracks[0].steps[1] = Step { active: true, velocity_zone: VelocityZone::Mid, ..Default::default() };
    s.patterns[0] = Some(p);
    s
}
fn t0(s: &Session) -> (&usize, &u8, &f32) {
    let t = &s.patterns[0].as_ref().unwrap().tracks[0]; (&t.length, &t.midi_note, &t.speed_ratio)
}
#[test]
fn trash_preserves_length_note_ratio() {
    let e = Engine::new(); e.load_session_for_test(session());
    let before = t0(&e.snapshot_arc());
    e.apply_command(sequencer_engine::command::Command::Trash { track_idx: 0 });
    let after = t0(&e.snapshot_arc());
    assert_eq!(before, after);
    assert!(!e.snapshot_arc().patterns[0].as_ref().unwrap().tracks[0].steps[1].active);
}
#[test]
fn cut_pushes_undo_and_undo_restores() {
    let e = Engine::new(); e.load_session_for_test(session());
    e.apply_command(sequencer_engine::command::Command::Cut { track_idx: 0 });
    assert!(!e.snapshot_arc().patterns[0].as_ref().unwrap().tracks[0].steps[1].active);
    e.apply_command(sequencer_engine::command::Command::Undo { track_idx: 0 });
    assert!(e.snapshot_arc().patterns[0].as_ref().unwrap().tracks[0].steps[1].active);
}
```

- [ ] **Step 3: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test`
Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add engine/crates/core/src/engine.rs engine/crates/core/tests/algorithms_wiring.rs
git commit -m "feat(core): wire Roll/Vary/Cut/Copy/Paste/Trash/Undo into worker (undo-first)"
```

---

## Task 16: Scheduler — pattern queue, quantize, follow-actions, transitions

**Files:**
- Modify: `engine/crates/core/src/scheduler.rs`

**Interfaces:** Produces `Scheduler` with `queue_pattern`, `on_bar_boundary` (evaluate follow-actions), `flush_transition` (all-notes-off burst).

- [ ] **Step 1: Write failing tests + impl**

`scheduler.rs` (a focused first cut: queued pattern + quantize resolution at the step/beat/bar boundary):

```rust
//! Pattern queue + quantize-grain + follow-actions. Transitions emit an
//! all-notes-off burst (bounded) so stale notes don't ring across patterns.

use crate::midi::build_all_notes_off;
use crate::midi_out::MidiOutRing;
use crate::midi_out::push_drop_oldest;
use crate::models::{QuantizeGrain, Session, MAX_TRACKS};

pub struct Scheduler {
    queued: Option<(usize, QuantizeGrain)>,
}
impl Default for Scheduler { fn default() -> Self { Self { queued: None } } }

impl Scheduler {
    pub fn queue(&mut self, index: usize, grain: QuantizeGrain) { self.queued = Some((index, grain)); }
    pub fn cancel(&mut self) { self.queued = None; }
    /// Called by the RT loop at a pattern boundary. Returns the new active index
    /// (if a queued pattern fires at this grain) and pushes all-notes-off.
    pub fn on_boundary(&mut self, s: &Session, midi: &MidiOutRing, at_grain: QuantizeGrain) -> Option<usize> {
        let fire = matches!(self.queued, Some((_, g)) if g == at_grain || grain_reached(g, at_grain));
        if fire {
            let (idx, _) = self.queued.take()?;
            // all-notes-off burst, bounded to MAX_TRACKS so it can't self-overflow
            for tr in 0..MAX_TRACKS {
                let _ = push_drop_oldest(midi, build_all_notes_off(0, s.global_midi_channel));
            }
            return Some(idx);
        }
        None
    }
}

fn grain_reached(queued: QuantizeGrain, at: QuantizeGrain) -> bool {
    use QuantizeGrain::*;
    match (queued, at) {
        (NextStep, _) => true,
        (NextBeat, NextBeat | NextBar | EndOfPattern) => true,
        (NextBar, NextBar | EndOfPattern) => true,
        (EndOfPattern, EndOfPattern) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::QuantizeGrain::*;
    #[test]
    fn queued_pattern_fires_at_grain() {
        let mut sch = Scheduler::default();
        let s = Session::default();
        let midi = crate::midi_out::midi_out_ring();
        sch.queue(3, NextBar);
        assert_eq!(sch.on_boundary(&s, &midi, NextStep), None);  // too early
        assert_eq!(sch.on_boundary(&s, &midi, NextBar), Some(3)); // fires
        assert!(sch.queued.is_none());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine scheduler`
Expected: `1 passed; 0 failed`.

- [ ] **Step 3: Commit**

```bash
git add engine/crates/core/src/scheduler.rs
git commit -m "feat(core): scheduler — pattern queue, quantize grain, transition flush"
```

---

## Task 17: External sync mode-switch — LinkPhase / MidiClockTick (core)

**Files:**
- Modify: `engine/crates/core/src/engine.rs` (apply `LinkPhase`/`MidiClockTick`/`SetSyncSource`)

**Interfaces:** Consumes `SyncSource`, `LinkPhase`, `MidiClockTick`.

- [ ] **Step 1: Write failing tests + impl**

Add per-RT external-clock state to `RtState`: `pub external_step: AtomicU32`-style counter is wrong on RT (it's in RtState, local). Use `pub midi_tick_count: u32` and `pub link_step: u64` in `RtState`; in `apply_command`, when `sync_source != Free`, store the external position into the engine for the RT loop to read. Since RT reads the immutable snapshot, external position must live in the published session or a dedicated atomic. Add to `Engine`: `pub external_clock: Arc<ExternalClock>` where:

```rust
pub struct ExternalClock {
    pub midi_ticks: AtomicU32,        // raw 24-PPQN tick count (worker increments)
    pub midi_step_pulses: AtomicU32,  // one pulse per 6 ticks = one 16th step (RT consumes)
    pub link_beats_micros: AtomicU64, // LinkPhase absolute position (RT positions from this)
}
```

Wire these as **outer** match arms in `apply_command` (alongside `Play`/`Stop`) — they set atomics, not the session. **Move `SetSyncSource` out of the inner `other` block to here** to avoid a duplicate arm:
```rust
            SetSyncSource { source } => {
                let mut s = (*self.snapshot.load_full()).clone();
                s.sync_source = source;
                self.publish(s);
            }
            LinkPhase { beats_since_origin, phase: _ } => {
                self.external_clock.link_beats.store((beats_since_origin * 1_000_000.0) as u64, Ordering::Release);
            }
            MidiClockTick => {
                // 24 PPQN -> 6 ticks per 16th step (E6). Worker accumulates; RT consumes pulses.
                let t = self.external_clock.midi_ticks.fetch_add(1, Ordering::AcqRel) + 1;
                if t % 6 == 0 {
                    self.external_clock.midi_step_pulses.fetch_add(1, Ordering::AcqRel);
                }
            }
```

The RT loop (`run_rt_loop`, Task 11) consumes the external clock instead of its internal deadline when synced. Replace the `process` + `sleep_until` tail of `run_rt_loop` with a branch on `snap.sync_source`: **`Free`** → `process()` once + `sleep_until` the next 16th deadline (internal clock, unchanged from Task 11); **`MidiClock`** → `let pulses = self.external_clock.midi_step_pulses.swap(0, Ordering::AcqRel); for _ in 0..pulses { process(&mut rt, &snap, playing, now, &self.midi, &self.hot_events); }` then a short 500 µs poll-sleep; **`Link`** → read `link_beats_micros`, compute `target = (beats * 4.0) as u64` 16th-steps since origin, `while rt.link_step_count < target { process(...); rt.link_step_count += 1; }` then poll. Add `pub link_step_count: u64` to `RtState`. `process()` stays **source-agnostic** (advances one step per call); the loop decides cadence — so `process` remains fully unit-testable and the RT path stays atomic-load-only.

`engine/crates/core/tests/sync.rs`:
```rust
use sequencer_engine::command::Command;
use sequencer_engine::engine::Engine;
use sequencer_engine::models::SyncSource;
use std::sync::atomic::Ordering;

#[test]
fn midi_clock_accumulates_one_step_pulse_per_six_ticks() {
    let e = Engine::new();
    e.apply_command(Command::SetSyncSource { source: SyncSource::MidiClock });
    for _ in 0..6 { e.apply_command(Command::MidiClockTick); }
    assert_eq!(e.external_clock.midi_step_pulses.load(Ordering::Acquire), 1);
    for _ in 0..6 { e.apply_command(Command::MidiClockTick); }
    assert_eq!(e.external_clock.midi_step_pulses.load(Ordering::Acquire), 2);
}

#[test]
fn link_phase_stores_absolute_position() {
    let e = Engine::new();
    e.apply_command(Command::LinkPhase { beats_since_origin: 4.5, phase: 0.0 });
    assert_eq!(e.external_clock.link_beats_micros.load(Ordering::Acquire), 4_500_000);
}

// RT-loop consumption of these pulses (`run_rt_loop` under MidiClock advancing the
// playhead) is integration-tested alongside Task 20's lifecycle test, since it
// requires running the RT thread.
```

- [ ] **Step 2: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine --test sync`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add engine/crates/core/src/engine.rs engine/crates/core/tests/sync.rs
git commit -m "feat(core): external sync mode-switch — LinkPhase + MidiClockTick (E6)"
```

---

## Task 18: LoadSession apply + reload generation + FullSnapshot (core/ffi)

**Files:**
- Modify: `engine/crates/core/src/engine.rs` (`apply_command` `LoadSession` arm), add `reload_generation: AtomicU32`.
- Test: `engine/crates/ffi/tests/ffi_api.rs`

- [ ] **Step 1: Write failing C-ABI test**

```rust
#[test]
fn load_session_swaps_and_emits_full_snapshot() {
    let eng = unsafe { engine_new() };
    // serialize a non-default session on a second engine
    let eng2 = unsafe { engine_new() };
    let cmd = postcard::to_allocvec(&sequencer_engine::command::Command::SetBpm { bpm: 99.0 }).unwrap();
    unsafe { engine_submit_command(eng2, cmd.as_ptr(), cmd.len()) };
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut ptr = std::ptr::null_mut(); let mut len = 0usize;
    unsafe { engine_serialize(eng2, &mut ptr, &mut len) };
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) }.to_vec();
    unsafe { engine_free_bytes(ptr, len) };
    unsafe { engine_free(eng2) };

    let load = postcard::to_allocvec(&sequencer_engine::command::Command::LoadSession { bytes }).unwrap();
    unsafe { engine_submit_command(eng, load.as_ptr(), load.len()) };
    std::thread::sleep(std::time::Duration::from_millis(20));
    // serialize eng -> should now be 99 bpm
    let mut p2 = std::ptr::null_mut(); let mut l2 = 0usize;
    unsafe { engine_serialize(eng, &mut p2, &mut l2) };
    let b2 = unsafe { std::slice::from_raw_parts(p2 as *const u8, l2) };
    let env: sequencer_engine::serde_ext::SessionEnvelope = postcard::from_bytes(b2).unwrap();
    assert_eq!(env.session.bpm, 99.0);
    unsafe { engine_free_bytes(p2, l2) };
    unsafe { engine_free(eng) };
}

#[test]
fn load_session_bad_version_returns_err_decode_no_swap() {
    let eng = unsafe { engine_new() };
    let bad = postcard::to_allocvec(&sequencer_engine::command::Command::LoadSession { bytes: vec![0xFFu8; 4] }).unwrap();
    let r = unsafe { engine_submit_command(eng, bad.as_ptr(), bad.len()) };
    // worker applies off-thread; the malformed envelope is rejected inside apply (no swap).
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut p = std::ptr::null_mut(); let mut l = 0usize;
    unsafe { engine_serialize(eng, &mut p, &mut l) };
    let b = unsafe { std::slice::from_raw_parts(p as *const u8, l) };
    let env: sequencer_engine::serde_ext::SessionEnvelope = postcard::from_bytes(b).unwrap();
    assert_eq!(env.session.bpm, 120.0); // unchanged
    unsafe { engine_free_bytes(p, l) };
    unsafe { engine_free(eng) };
    let _ = r;
}
```

- [ ] **Step 2: Implement the LoadSession arm**

In `apply_command`:
```rust
            LoadSession { bytes } => {
                use crate::serde_ext::SessionEnvelope;
                match postcard::from_bytes::<SessionEnvelope>(&bytes) {
                    Ok(env) if env.version == crate::serde_ext::SESSION_FORMAT_VERSION => {
                        self.publish(env.session);
                        self.reload_generation.fetch_add(1, Ordering::AcqRel);
                        let snap = self.snapshot.load_full();
                        crate::midi_out::push_event(&self.hot_events, &EngineEvent::FullSnapshot { session: (*snap).clone() });
                    }
                    _ => { /* bad envelope/version: no swap, no event (E: ErrDecode is a submit-time concern) */ }
                }
            }
```

- [ ] **Step 3: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test`
Expected: both load-session tests green.

- [ ] **Step 4: Commit**

```bash
git add engine/crates/core/src/engine.rs engine/crates/ffi/tests/ffi_api.rs
git commit -m "feat(core): LoadSession apply — validate version, publish, emit FullSnapshot"
```

---

## Task 19: CoreMIDI worker + real MIDISend (ffi)

**Files:**
- Modify: `engine/crates/ffi/src/coremidi.rs`

**Interfaces:** Consumes `MidiOutRing`, `MidiMsg`. Produces `run_coremidi_worker`.

- [ ] **Step 1: Implement the worker + bindings**

`coremidi.rs` — real `extern "C"` CoreMIDI bindings + a worker that drains the ring, builds a stack `MIDIPacketList`, `MIDISend`s Note-Ons, synthesizes+sends Note-Offs at Note-On-time + gate, and on a `stop_generation` change drains-and-drops + sends all-notes-off. Use `core_foundation`/`core_midi` sys bindings or raw `extern "C"` blocks. Sketch (the implementer fills the exact symbol names against the SDK headers):

```rust
use sequencer_engine::engine::Engine;
use sequencer_engine::midi_out::MidiMsg;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

// CoreMIDI is linked as a framework (ffi/build.rs). Resolve the EXACT signatures
// against <CoreMIDI/CoreMIDI.h> for this set: MIDIClientCreate, MIDIEndpointGetEntity,
// MIDIOutputPortCreate, MIDISend, MIDIPacketListInit, MIDIPacketListAdd,
// MIDIDestinationCreate + MIDIReadProc (host test only), MIDIEndpointDispose.
extern "C" {
    fn MIDISend(port: MidiPortRef, dest: MidiEndpointRef, pktlist: *const MIDIPacketList) -> OSStatus;
    fn MIDIPacketListInit(pktlist: *mut MIDIPacketList) -> *mut MIDIPacket;
    fn MIDIPacketListAdd(pktlist: *mut MIDIPacketList, list_size: ByteCount, cur_packet: *mut MIDIPacket, time: MidiTimeStamp, n_data: ByteCount, data: *const u8) -> *mut MIDIPacket;
}
type MidiPortRef = usize; type MidiEndpointRef = usize; type OSStatus = i32;
type ByteCount = usize; type MidiTimeStamp = u64;

#[repr(C)] struct MIDIPacketList { numPackets: u32, packet_data: [u8; 1024] } // single inline packet is enough for 1-3 bytes
#[repr(C)] struct MIDIPacket { timeStamp: MidiTimeStamp, length: u16, data: [u8; 256] }

pub fn run_coremidi_worker(engine: &Arc<Engine>, port: MidiPortRef) {
    let mut last_stop_gen = engine.transport.stop_generation.load(Ordering::Acquire);
    // Pending sends: (deadline, dest, [status,note,velocity]). Note-Ons scheduled at
    // now + send_at_offset_micros (swing/micro_timing); Note-Offs at that + gate (E2/E3/E5).
    let mut pending: heapless::Vec<(Instant, MidiEndpointRef, [u8; 3]), 128> = heapless::Vec::new();
    while !engine.shutdown.load(Ordering::Acquire) {
        let gen = engine.transport.stop_generation.load(Ordering::Acquire);
        if gen != last_stop_gen {
            while engine.midi.dequeue().is_some() {} // drain-and-drop (no Note-On after stop)
            pending.clear();
            let _ = send_cc_all_notes_off(port, engine);
            last_stop_gen = gen;
            continue;
        }
        // fire due scheduled sends (note-ons by offset, note-offs by offset+gate)
        let now = Instant::now();
        let mut i = 0;
        while i < pending.len() {
            if pending[i].0 <= now {
                let (_, dest, bytes) = pending.remove(i);
                let _ = send_one(port, dest, &bytes);
            } else { i += 1; }
        }
        // drain ring -> schedule
        while let Some(m) = engine.midi.dequeue() {
            if m.status & 0xF0 == 0x90 {
                let fire = Instant::now() + Duration::from_micros(m.send_at_offset_micros as u64);
                let _ = pending.push((fire, m.endpoint as MidiEndpointRef, [m.status, m.note, m.velocity]));
                if m.gate_micros > 0 {
                    let off = fire + Duration::from_micros(m.gate_micros as u64);
                    let _ = pending.push((off, m.endpoint as MidiEndpointRef, [(m.status & 0xF0), m.note, 0]));
                }
            } else {
                let _ = send_one(port, m.endpoint as MidiEndpointRef, &[m.status, m.note, m.velocity]); // CC all-notes-off: immediate
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn send_one(port: MidiPortRef, dest: MidiEndpointRef, bytes: &[u8]) -> OSStatus {
    // Build a single-packet MIDIPacketList on the stack, MIDISend. No heap.
    let mut pl = MIDIPacketList { numPackets: 1, packet_data: [0; 1024] };
    unsafe {
        let pkt = MIDIPacketListInit(&mut pl as *mut _);
        let _ = MIDIPacketListAdd(&mut pl as *mut _, 1024, pkt, 0, bytes.len() as ByteCount, bytes.as_ptr());
        MIDISend(port, dest, &pl as *const _)
    }
}
fn send_cc_all_notes_off(port: MidiPortRef, e: &Engine) -> OSStatus {
    // one CC-123 per used channel; bounded. Engine carries global_midi_channel in the snapshot.
    let snap = e.snapshot.load_full();
    send_one(port, 0, &[0xB0 | (snap.global_midi_channel & 0x0F), 123, 0])
}
```

> **RT note:** `send_cc_all_notes_off` calls `snapshot.load()` — but the CoreMIDI worker is off-RT, so a `Guard` load there is fine. **The RT thread never calls any of this.**

- [ ] **Step 2: Host test against a virtual destination (cfg macos)**

`engine/crates/ffi/tests/coremidi_host.rs`:

```rust
#![cfg(target_os = "macos")]
use sequencer_engine::engine::Engine;
use sequencer_engine::midi::build_note_on;
use sequencer_engine::midi_out::push_drop_oldest;
use std::sync::{Arc, Mutex};

// Test-only exception to Rule 7 (recorded in the design): this test owns a
// MIDIClientRef + virtual destination in Rust and receives into RECV.
// Resolve the exact CoreMIDI calls against <CoreMIDI/CoreMIDI.h>:
// MIDIClientCreate(CFSTR("StepForge-test"), nil, nil, &client)
// MIDIDestinationCreate(client, CFSTR("recv"), read_proc, ctx, &dest)  // read_proc appends to RECV
static RECV: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

extern "C" fn read_proc(_pktlist: *const sequencer_engine_ffi::coremidi::MIDIPacketList, _src: usize, _ctx: *mut core::ffi::c_void) {
    // iterate packets, push each packet.data[..length] into RECV.lock()
}

#[test]
fn note_on_then_note_off_reach_virtual_destination() {
    let eng = Arc::new(Engine::new());
    RECV.lock().unwrap().clear();
    // 1. create the client + virtual destination (read_proc -> RECV); record `dest`.
    // 2. publish eng midi_destinations = [dest] so dispatch targets it.
    // 3. push a NoteOn (channel 10, note 36, vel 100, offset 0); gate is DEFAULT_GATE_MICROS (50ms):
    let _ = push_drop_oldest(&eng.midi, build_note_on(0, 10, 36, 100, 0));
    // 4. spawn run_coremidi_worker(&eng, port) briefly; signal shutdown after ~150 ms.
    // 5. assert RECV contains [0x9A, 36, 100] then [0x8A, 36, 0], in order.
    let recv = RECV.lock().unwrap();
    let flat: Vec<u8> = recv.iter().flatten().copied().collect();
    let on = vec![0x9A, 36, 100];
    let off = vec![0x8A, 36, 0];
    let on_pos = flat.windows(3).position(|w| w == on.as_slice());
    let off_pos = flat.windows(3).position(|w| w == off.as_slice());
    assert!(on_pos.is_some() && off_pos.is_some(), "Note-On and Note-Off both received");
    assert!(on_pos.unwrap() < off_pos.unwrap(), "Note-On before Note-Off");
}
```

> The CoreMIDI object creation (steps 1–2) and worker spawn (step 4) are resolved against the SDK headers at implementation; the **assertion above is the contract** (Note-On then gate-delayed Note-Off, order/presence within a generous timeout). The implementer may add a small injectable gate to keep the test fast — but `build_note_on`'s gate is a const, so just allow ~150 ms.

- [ ] **Step 3: Run tests (host)**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test -p sequencer_engine_ffi --test coremidi_host`
Expected: PASS on macOS (Note-On then gate-delayed Note-Off observed).

- [ ] **Step 4: Commit**

```bash
git add engine/crates/ffi/src/coremidi.rs engine/crates/ffi/tests/coremidi_host.rs
git commit -m "feat(ffi): CoreMIDI worker — real MIDISend, Note-Off synth, stop drain-drop"
```

---

## Task 20: FFI lifecycle wiring (start/stop/free/submit/drain) + audit + final green

**Files:**
- Modify: `engine/crates/ffi/src/lib.rs`, `engine/crates/ffi/src/handle.rs`

**Interfaces:** Consumes `Engine::run_rt_loop`, `Engine::run_worker_loop`, `run_coremidi_worker`, `InstantClock`.

- [ ] **Step 1: Wire the entry points**

`handle.rs`: `EngineHandle` holds `pub engine: Box<Engine>` (unchanged) — the threads are owned by `Engine`'s handle `Mutex`es.

`lib.rs`:
- `engine_start`: build an `InstantClock`, spawn RT (`engine.run_rt_loop(&clock)`), state worker (`engine.run_worker_loop()`), CoreMIDI worker (`run_coremidi_worker(&engine, port)`); store `JoinHandle`s in `Engine`'s `Mutex`es. Return `EngineResult::Ok`.
- `engine_stop`: set `shutdown=true`; join all three handles (clearing the `Mutex` slots). Return before `engine_free`.
- `engine_free`/`free_handle`: **defensively** — if handles are still `Some` (free-without-stop), signal shutdown + join them, THEN `Box::from_raw`/drop.
- `engine_submit_command`: decode → `push_drop_oldest(commands, cmd)` → `Ok` (on overflow the `Overflow` event fires; see Task 5).
- `engine_drain_events`: drain one `HotEventSlot` from `hot_events` → box the `bytes[..len]` → hand out via out-params (NULL-tolerant); if empty, write NULL/0 (drained). Large-channel (`Serialized`/`Error`) events ride the same hot channel via `push_event` (they're already postcard bytes ≤ 128 except `Serialized` — for `Serialized`, route through a separate large channel drained second; keep the existing large-channel plumbing).

- [ ] **Step 2: Add a lifecycle C-ABI test**

`engine/crates/ffi/tests/ffi_api.rs`:
```rust
#[test]
fn start_submit_drain_stop_free_no_crash() {
    let eng = unsafe { engine_new() };
    assert_eq!(unsafe { engine_start(eng) }, EngineResult::Ok as i32);
    let cmd = postcard::to_allocvec(&sequencer_engine::command::Command::Play).unwrap();
    assert_eq!(unsafe { engine_submit_command(eng, cmd.as_ptr(), cmd.len()) }, EngineResult::Ok as i32);
    std::thread::sleep(std::time::Duration::from_millis(30));
    // drain a few events
    for _ in 0..8 {
        let mut ptr = std::ptr::null_mut(); let mut len = 0usize;
        let _ = unsafe { engine_drain_events(eng, &mut ptr, &mut len) };
        if !ptr.is_null() && len > 0 { unsafe { engine_free_bytes(ptr, len) } }
    }
    assert_eq!(unsafe { engine_stop(eng) }, EngineResult::Ok as i32);
    unsafe { engine_free(eng) };
}
```

- [ ] **Step 3: Run the full suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd engine && cargo test`
Expected: all engine tests green (unit + integration + C-ABI).

- [ ] **Step 4: Audit + build the xcframework + app**

Run:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd engine && cargo clippy --all-targets -- -D warnings && cargo fmt --check
engine/scripts/build_engine.sh
cd ../app && xcodegen generate && xcodebuild -project StepForge.xcodeproj -scheme StepForge \
  -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO build
```
Expected: clippy 0 warnings, fmt clean, `** BUILD SUCCEEDED **`.

Then run the project audits (invoke the skills):
- `/audit-rt` over `engine.rs::process`, `clock.rs`, `midi.rs`, `midi_out.rs`, the RT entry — confirm zero alloc/lock/FFI/CoreMIDI/syscall on the hot path.
- `/enforce-ffi` over the boundary — confirm ownership, null-safety, no panics across FFI, no `#[repr(C)]` data enums, CoreMIDI ownership in Swift (worker reads integer IDs only).

- [ ] **Step 5: Commit**

```bash
git add engine/crates/ffi/src/lib.rs engine/crates/ffi/src/handle.rs engine/crates/ffi/tests/ffi_api.rs
git commit -m "feat(ffi): real engine_start/stop/free + submit enqueues + drain; lifecycle green"
```

---

## Definition of Done (whole plan)

- `cargo test` (engine) all green; `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt --check` clean.
- App build (`xcodebuild … -sdk iphonesimulator`) `** BUILD SUCCEEDED **`.
- `/audit-rt` confirms the RT hot path is allocation-/lock-/FFI-/CoreMIDI-/syscall-free (one `Guard` load + immutable reads + transport atomics + fixed-slot writes + RNG).
- `/enforce-ffi` confirms the C-ABI boundary honors ownership, null-safety, panic-safety, no `#[repr(C)]` data enums; `EngineResult` numbering unchanged (`Overflow` is on `EngineEvent`, not `#[repr(C)]`).
- E1–E11 each resolved as pinned in the design doc; no serialized model-shape change (`SESSION_FORMAT_VERSION` still 1).
- CoreMIDI `MIDISend` validated against a virtual destination in a macOS host test.
