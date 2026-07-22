# Engine Plan — Design

**Status:** Draft for review (post adversarial review, rev 2) · **Date:** 2026-07-22 · **Scope successor of:** `2026-07-22-project-foundation-design.md`

This plan fills the engine stubs the foundation left and resolves open issues **E1–E11**. It turns `sequencer_engine` from a compiling skeleton with a real cross-layer contract into a working musical brain: a self-scheduled real-time (RT) thread that advances musical time, dispatches steps, and emits MIDI through a lock-free ring to a CoreMIDI worker.

> Rev 2 incorporates an adversarial 5-lens review (RT-safety, FFI/contract, no-orphans, E1–E11 completeness, testability). The headline change: the concurrency model is **copy-on-write immutable snapshots** (rev 1's seqlock-over-`Session` was unsound — a `Vec<Track>` realloc is a use-after-free a seqlock cannot prevent, and a seqlock cannot be written in `forbid(unsafe_code)` core). COW also dissolves the serialize-routing blocker and removes the need for any staging arrays.

## Goals & scope

**Definition of done (approved): host-testable musical brain.** The full RT thread + clock + step dispatch (swing/humanize/speed_ratio/micro_timing) + Roll/Vary algorithms + the RT→CoreMIDI ring + worker with **real CoreMIDI `MIDISend`**, all correct and deterministically testable in `cargo test` on the macOS host. CoreMIDI output is validated against a **virtual destination** in host tests (CoreMIDI.framework links on macOS too — this is real I/O, not a mock). Live playback tuning to a physical device is the *only* thing deferred (to a thin follow-up).

**In scope**
- The four no-op FFI entry points become real: `engine_start` spawns the threads, `engine_stop` joins them, `engine_submit_command` enqueues (today it decodes-and-discards), `engine_drain_events` produces real events.
- `engine.rs`, `clock.rs`, `midi.rs`, `scheduler.rs`, `algorithms/{roll,vary}.rs`, `clipboard.rs`, `undo.rs`, and a new `midi_out.rs` (all currently doc-comment-only stubs).
- `ffi/src/coremidi.rs` (today `flush_all_notes_off` is an empty body) becomes the real CoreMIDI bindings + worker.
- `Command::LoadSession` apply path (variant + codec exist; restore was deferred in the final review).
- Resolutions of E1–E11 (each pinned concretely below).

**Out of scope (deferred)**
- Live-playback-to-physical-device tuning and on-device jitter measurement.
- A CoreAudio/AudioUnit render-callback clock (the clock is a `Clock` trait now — this is a localized future swap).
- PLL phase-correction for external sync (mode-switch only, per spec lean).
- E7/E11 Swift-side *implementation* (`EngineBridge` / `SessionMirror` / the ~120 Hz drain `DispatchQueue`) — the **app plan** owns that; this plan commits only the event shapes that make single-MainActor-hop-per-batch + playhead coalescing possible, plus the engine-side contract the Swift layer must honor.

## What we honor (locked — do not redesign)

The foundation's contract is real, tested, and binding. Summary (full detail in the foundation design + `amendments.md` A1–A16):

- Workspace split compiler-enforced: `core` (`#![forbid(unsafe_code)]`), `ffi` (`#![allow(unsafe_code)]`, the only `unsafe`). All CoreMIDI `unsafe` and the RT-priority syscall live in `ffi/src/coremidi.rs` / the prod clock.
- FFI seam is **bytes**, not structs: `Command`/`EngineEvent` cross the C ABI as postcard bytes; no data-carrying `#[repr(C)]` enum crosses.
- The 8 `extern "C"` entry points + `#[repr(C)] EngineResult` (`Ok=0, ErrDecode=1, ErrInvalidHandle=2, ErrInvalidBuffer=3, ErrOther=4` — existing values do not change; a new value may only be *appended*); every body wrapped in `catch_unwind`; codecs total.
- Buffer ownership (`engine_free_bytes`, exactly once), drain framing (one event per `engine_drain_events`, empty/zero-length = drained, NULL out-params tolerated), `MAX_EVENT_BYTES = 128` hot-channel cap, large payloads off-RT.
- `engine_serialize` returns bytes **synchronously** via non-NULL out-params on the caller thread (this plan preserves that exactly — see Threading).
- `Track.length` is a window over a fixed `[Step; 16]`; **Roll/Vary/Cut/Trash never touch `length`/`midi_note`/`speed_ratio`; Paste carries `length`+`speed_ratio` but never `midi_note`**; clipboard carries `steps`/`length`/`speed_ratio` but not `midi_note`.
- Persistence is postcard + version-tagged (`SessionEnvelope`, `SESSION_FORMAT_VERSION = 1`). Any model-shape change bumps the version + adds a migration + updates the Swift mirror + the snapshot round-trip test. **This plan makes no serialized model-shape change** (verified per-issue below); `SESSION_FORMAT_VERSION` stays 1.

## Architecture: the timing spine

The RT thread must advance musical time allocation-free, lock-free, never cross FFI into Swift — **and** be host-testable. The decision (approved): a **pluggable `Clock` trait**.

```rust
// core/src/clock.rs — the trait + the steppable test clock live in core (safe).
pub trait Clock: Send + Sync {
    fn now_micros(&self) -> u64;                 // monotonic micros since engine start
    fn sleep_until(&self, deadline_micros: u64); // prod: coarse sleep + final spin; test: no-op
    fn elevate_priority(&self);                  // prod: unsafe RT-QoS syscall; test: no-op
}
```

- **`SteppableClock` (core):** `now_micros` returns a value the test sets; `sleep_until`/`elevate_priority` are no-ops. Tests call the dispatch math directly with virtual time.
- **`InstantClock` (ffi, *not* core):** the prod clock carries the `unsafe` `elevate_priority` (`pthread_set_qos_class_self` / `setschedparam`), so it lives in `ffi` per Rule 6. `engine_start` (in ffi) constructs it and injects it. **`elevate_priority` is called exactly once at RT-thread spawn — never inside the per-tick loop** (a per-tick syscall would poison the hot path). The RT loop computes the next step deadline from `Instant`, does a coarse `thread::sleep` to ~1 ms short of the deadline, then a tight spin-wait for sub-ms accuracy, then one `process()` tick.

Rejected alternatives: (B) CoreAudio render callback — sample-accurate but not host-testable in `cargo test`, pulls audio context in now, and tensions the RT-never-crosses-FFI rule; deferred to live-device work, where the trait makes it a localized swap. (C) hard-wired busy-wait spin with no trait — forces flaky time-based tests and burns a core; rejected.

The musical brain is factored so the clock is the *only* thing that differs between prod and test, and the per-note dispatch math is pure:

```rust
// core — the RT tick. rt_state is RT-OWNED mutable state ONLY (counters, accumulators, RNG).
// session is the IMMUTABLE COW snapshot loaded for this tick (see Threading). No &mut aliasing.
fn process(&mut rt_state: RtState,
           session: &Session, now_micros: u64,
           midi_out: &mut MidiOutRing,
           event_buf: &mut [u8; MAX_EVENT_BYTES]) -> TickOutcome { ... }
```

The per-note math (swing offset, humanize jitter, speed_ratio accumulation, ratchet expansion, gate value) is additionally exposed as **pure free functions** taking `&Track`/`&Session` scalars and writing into caller buffers — fully unit-testable with no `Engine`, no threads, no clock (see Testing).

## Threading & concurrency model (the RT-safety story) — COW snapshots

Three threads. The state worker is the **sole writer** and publishes **immutable copy-on-write snapshots**; the RT thread and `engine_serialize` are **lock-free readers** of those snapshots. No in-place mutation of a live session, no seqlock, no staging arrays.

```
                       MPSC command queue (bounded, lock-free, drop-oldest on full)
  UI / Sync / Persistence ─────────────────────────────────┐
                                                            ▼
   ┌──────────────────────────────┐        ┌──────────────────────────────┐
   │  STATE WORKER  (off-RT)      │        │  RT THREAD  (sacred)         │
   │  sole writer                 │        │  lock-free reader            │
   │  • drain MPSC commands       │ publish│  • load immutable snapshot   │
   │  • clone session → mutate    │ Arc<   │    (zero-alloc Guard)        │
   │    clone → Arc::new → store  │ Session│  • read transport atomics    │
   │  • manage undo/clipboard     │ ──────▶│  • process(): dispatch due   │
   │  • emit Serialized on large  │ atomic │    steps from &Session       │
   │    event channel             │ swap   │    → MidiOutRing + events   │
   └──────────────┬───────────────┘        └──────────────┬───────────────┘
                  │ load_full() (caller thread)           │ fixed-slot SPSC ring
                  │ = engine_serialize (sync, non-blocking)│
                  ▼                                        ▼
          (Swift frees via engine_free_bytes)  ┌──────────────────────────────┐
                                              │  COREMIDI WORKER  (off-RT,   │
                                              │  the only thread doing       │
                                              │  unsafe MIDISend)            │
                                              │  • drain ring                │
                                              │  • MIDIPacketList (stack buf)│
                                              │  • MIDISend(Note-On)         │
                                              │  • synthesize + send Note-Off│
                                              └──────────────────────────────┘
```

**Single-writer COW discipline.**

- A shared `ArcSwap<Session>` (the [`arc-swap`](https://crates.io/crates/arc-swap) crate, used via its **safe API**) holds the current snapshot. `arc-swap` is used because its `load()` returns a `Guard` that pins the current `Arc<Session>` with **no allocation and no refcount change** — dropping the `Guard` is trivial, so the RT read path allocates nothing and never frees.
- The **state worker** is the sole writer. On each command it does `let mut next = (**current).clone(); apply(&mut next, cmd); store.store(Arc::new(next));` — a clone-mutate-publish. The previous `Arc<Session>` is reclaimed by the worker (refcount→0) only once no reader holds it. Mutations are command-driven and off-RT, so the clone cost is paid off the hot path.
- The **RT thread** is a pure reader: each tick it does `let guard = store.load();` (lock-free, zero-alloc) and dispatches from `&*guard` — an **immutable** snapshot. Because the snapshot is never mutated in place, reading *every* field — including `Pattern.tracks: Vec<Track>` and `Session.midi_destinations: Vec<u32>` — is safe: there is no concurrent writer that could realloc the buffer under the read. **No staging arrays (`tracks_fixed`/`destinations_fixed`) are needed**; the torn-read/use-after-free risk that killed rev 1's seqlock cannot occur with immutable snapshots.
- **`engine_serialize` stays synchronous and inline on the caller thread** (preserving the locked contract exactly): it calls `store.load_full()` (returns an owned `Arc<Session>` — one atomic refcount bump, no heap alloc), serializes `&*arc` via postcard, boxes the bytes, hands the pointer+len out, returns. **No worker round-trip, no blocking, no shutdown deadlock.** The *asynchronous* `Command::Serialize → Serialized{Vec<u8>}` path is separate and is produced by the state worker on the off-RT large channel.
- **Transport stop is instant and lock-free:** `is_playing: AtomicBool` + `stop_generation: AtomicU32` are read directly by RT each tick (no snapshot needed). A stop flips the flag and bumps the generation.

**Why this satisfies Hard Rule 1 (RT is sacred).** On its hot path the RT thread does: one `Guard` load (atomic, zero-alloc), reads of immutable memory, reads of transport atomics, fixed-size writes (MIDI ring slots, `encode_event_into` into `[u8; MAX_EVENT_BYTES]`), and RNG advances. It allocates nothing, locks nothing, calls no FFI, touches no CoreMIDI, frees nothing. External Link/MIDI-clock timing arrives as commands consumed by the **worker**, never pushed into RT (A2). Bounded queues drop on overflow, never block.

**On floats (honest restatement):** Rule 1 forbids allocation/lock/FFI/CoreMIDI/blocking — *not* arithmetic. RT reads scalar `f32`/`f64` session fields (fixed-size, allocation-free) and does ordinary math; that is RT-safe. Fixed-point **Q16.16 is used only for the `speed_ratio` accumulator**, so it can count steps drift-free over long sessions; the rest of the per-tick math (swing, humanize, deadline from `bpm`) uses the `f32`/`f64` scalars directly.

**Ownership, stated once (resolving rev 1's contradiction):** the state worker owns the authoritative `Session` and is its sole writer; the RT thread owns `RtState` (its mutable per-tick state — step counters, per-track Q16.16 accumulators, RNG) and *reads* immutable `&Session` snapshots. `Engine` holds the shared `ArcSwap<Session>`, the queues, and the thread handles; it is not mutated through `&mut self` on the RT path.

**Three threads, not two — why.** The state worker (command application, COW clone-mutate-publish, undo/clipboard, serialize-event production) does bursty, sometimes allocating work. The CoreMIDI worker must stay light (drain ring + stack `MIDIPacketList` + `MIDISend`) to keep MIDI timing tight — it must not be stalled by a `LoadSession` clone/deserialize on the same thread. Splitting them isolates MIDI-timing latency from command-processing cost.

### Dependencies introduced (explicit decisions)

- **`arc-swap`** (core) — the COW snapshot store, safe API. The no-dependency alternative is an `AtomicPtr`-based `Arc` hand-off implemented in `ffi` (it needs `unsafe`) exposing a safe `SessionView` to core; `arc-swap` is preferred for vetted correctness.
- **`heapless`** (core) — safe, `forbid(unsafe_code)`-clean bounded queues: `heapless::spsc::Queue` for the RT→CoreMIDI ring and the hot event channel, `heapless::mpsc::MpMc` for the command MPSC. (A naïve hand-rolled `UnsafeCell<[T; N]>` ring would require `unsafe`, forbidden in core.)

## Clean shutdown & ownership (Rules 5, 6, 7)

- **`engine_stop` joins all three threads** (RT + state worker + CoreMIDI worker) and only then returns. The CoreMIDI worker drains its ring and finishes any in-flight `MIDISend` before its join completes. **Contract for the app plan:** Swift must keep the `MIDIClientRef` and endpoint set alive until *after* `engine_stop` returns (Rule 7).
- **`engine_free`/`free_handle` is defensively safe now that threads exist:** dropping a `JoinHandle` only *detaches* — it does not join. So `engine_free` signals a shutdown `AtomicBool` (polled by all three loops) and joins the handles **before** `Box::from_raw`/drop. A free-without-stop therefore degrades gracefully instead of use-after-freeing session/rings from a still-running RT thread or leaving a runaway `MIDISend`.
- **Stop race fixed:** on a `stop_generation` change the CoreMIDI worker (a) **drains and drops** every pending `MidiOutRing` entry enqueued before the bump, (b) sends all-notes-off, (c) cancels any gate-scheduled Note-Offs — so no Note-On fires after stop. ("RT flushes all-notes-off" means RT pushes a **bounded** burst of all-notes-off slots into the ring, bounded against ring depth so it cannot self-overflow — RT never touches CoreMIDI.)

## Components

**`core/src/engine.rs`** — `Engine` holds the shared `ArcSwap<Session>`, the bounded queues (command MPSC rx, hot event channel, large event channel, `MidiOutRing`), transport atomics, the shutdown flag, and the thread `JoinHandle`s. `RtState` (RT-owned mutable per-tick state) lives here. `process()` is the hot tick. Session mutation moves behind worker methods (today `session` is `pub`); the worker owns the authoritative session.

**`core/src/clock.rs`** — the `Clock` trait + `SteppableClock` (core); pure fixed-point scheduler math: a step counter wrapping at `track.length`; per-track Q16.16 `speed_ratio` accumulators (E1); swing applied at dispatch (E2); humanize jitter (E3/E4); consumption of `Step.micro_timing_offset` (E3). The prod `InstantClock` is in `ffi`.

**`core/src/scheduler.rs`** — pattern queue, quantize-grain evaluation (`NextStep`/`NextBeat`/`NextBar`/`EndOfPattern`), follow-actions, all-notes-off at pattern transitions, `RetriggerPattern` NextStep bypass.

**`core/src/midi.rs`** — pure dispatch math, no CoreMIDI/`unsafe`: velocity 0–127 mapping for Low/Mid/Accent (E4), humanize-velocity (E4), ratchet expansion (X2/X3/X4), Note-On/gate emission (E5). Produces fixed `MidiMsg` ring entries.

**`core/src/midi_out.rs`** (new — E10) — `MidiOutRing` (`heapless::spsc::Queue<MidiMsg>`) + the fixed `MidiMsg` slot layout. Pure safe Rust.

**`core/src/algorithms/{roll,vary}.rs`** — Roll sets `Step.micro_timing_offset` (and may toggle steps); Vary perturbs non-accent steps and **locks accents**, falling back to Roll when there are no accents. Both **push a per-track undo snapshot first**, and preserve `length`/`midi_note`/`speed_ratio`.

**`core/src/clipboard.rs`** — track + session clipboards; `TrackClipboard` carries `steps`/`length`/`speed_ratio`, never `midi_note`.

**`core/src/undo.rs`** — per-track, one-deep undo snapshots (max 8). Snapshot triggers are exactly Roll/Vary/Cut/Paste/Trash; a `length` change does **not** push undo.

**`ffi/src/coremidi.rs`** — real `extern "C"` CoreMIDI bindings (`MIDISend`, `MIDIPacketList` built into a stack buffer), all-notes-off, and the **CoreMIDI worker thread** (drain ring, `MIDISend` Note-On, synthesize+send Note-Off at Note-On-time + gate, drain-and-drop on stop-generation change). The prod `InstantClock` (unsafe `elevate_priority`) also lives here.

**`ffi/src/handle.rs` + `ffi/src/lib.rs`** — `engine_start` spawns RT + state + CoreMIDI threads (injecting the `InstantClock`); `engine_stop` signals shutdown + joins all three; `engine_free`/`free_handle` joins-then-drops defensively; `engine_submit_command` enqueues into the bounded MPSC; `engine_drain_events` drains the hot channel then the large channel into the out-params; `engine_serialize` reads via `load_full()` inline.

## Data flow

- **Command (Swift→Rust):** `engine_submit_command(ptr,len)` → postcard decode → bounded MPSC enqueue (non-blocking; **drop-oldest on full** + emit `EngineEvent::Overflow`, see E8) → state worker drains → clone-mutate-publish (or, for `LoadSession`, deserialize-then-publish). On a successful `LoadSession` the worker bumps a reload generation and emits `EngineEvent::FullSnapshot{session}` so the Swift mirror refreshes.
- **MIDI (Rust→device):** RT `process()` → due step hits → `midi.rs` produces fixed `MidiMsg`s → `MidiOutRing` push (RT, lock-free SPSC) → CoreMIDI worker drains → `MIDIPacketList` (stack) → `MIDISend` (Note-On); worker schedules + sends Note-Off at gate deadline; on stop-generation change, drain-and-drop + all-notes-off.
- **Events (Rust→Swift):** RT emits small events (`Playhead`, transport) via `encode_event_into` into the hot fixed-slot channel; large payloads (`Serialized`, `Error`) are produced by the **state worker** on the off-RT large channel. `engine_drain_events` yields one event per call (hot channel first, then large), empty = drained. **The locked signature has no caller buffer** (only `out_ptr`/`out_len`), so each drained event is boxed on the drain thread and freed by Swift via `engine_free_bytes` — an off-RT, bounded (~≤480 hot events/sec per E9) per-event allocation; the fixed-slot channel still pays off by keeping RT alloc-free and bounding event size.

## E1–E11 resolutions (concrete)

- **E1 — speed_ratio.** ratio > 1 = the track plays **faster** (more track steps per global step). RT advances a per-track **Q16.16 fixed-point** accumulator by `speed_ratio` each global tick and fires `⌊acc⌋` steps; the step index wraps at `track.length`, staying bounded inside `[Step;16]`. The stored `Track.speed_ratio: f32` is **unchanged** (no schema bump); RT converts the f32 → Q16.16 once per tick at the read boundary.
- **E2 — swing.** **Additive**, matching `models.rs:104` ("relative to global"): effective swing = `global_swing_pct + track.swing_pct` (track `0` = no *extra* per-track swing, not a sentinel for "inherit"). Curve = a percentage of the 16th-note interval by which off-grid steps are delayed, hard-capped at < 50 %. Swing base = **16th notes**. Fixed-point on the dispatch path.
- **E3 — micro_timing_offset.** **The stored `Step.micro_timing_offset: f32` stays `f32`** (architecture-spec §6.2 sets it as an f32 fraction via `gen_range(-0.5..0.5)`) — **no `SESSION_FORMAT_VERSION` bump, no migration**. The dispatch path converts the f32 to a fixed-point fraction-of-step at the read boundary and applies it as an **additive offset after swing**, clamped to ±½ step so it can never spill into a neighbor's window.
- **E4 — velocity + humanize.** **Fixed const table** for Low/Mid/Accent (proposed defaults `64 / 100 / 127`; pinned in implementation). No schema bump. Humanize is applied **at dispatch on RT**; the humanize-velocity magnitude is in **MIDI velocity units** (proposed ±5), scaled from the single `Session.humanize_velocity: f32` scalar (e.g., `jitter = humanize_velocity * zone_weight * ±5`). RNG is seeded deterministically: at play-start RT hashes the immutable snapshot with a fixed std hasher (no schema field) and seeds its own RNG, advanced per hit. Same session → same seed → identical playback → snapshots round-trip.
- **E5 — Note-Off / gate.** **v1 uses a single global const gate (proposed 50 ms); no `Step`/`Track` field is added** (neither has a gate field today). Per-step gate is explicitly **deferred** — introducing it later triggers the full "Model changes ripple" (bump + migration + mirror + test). RT emits Note-On + the const gate; the CoreMIDI worker synthesizes+sends the Note-Off at Note-On-time + gate. The gate value is **injectable** so host tests use a small gate and assert Note-On-then-Note-Off order/presence (not precise offsets) with a generous timeout. The worker's note-off scheduling clock is `Instant` (real-time); note-off *delivery timing* is therefore validated only in the real-time host test, while the gate *value* and ordering are unit-tested.
- **E6 — external sync.** **Mode-switch** per spec §9.2/§9.3: when `sync_source ∈ {Link, MidiClock}`, the external clock drives step advance and the internal `InstantClock` stops self-advancing. `LinkPhase{beats_since_origin,phase}` positions the bar/step; **MIDI Clock is 24 PPQN → 6 `MidiClockTick`s per 16th-note step quantum**, and per-track `speed_ratio` still subdivides that quantum under external sync. PLL correction is deferred (YAGNI).
- **E7 — MainActor hop / coalesce.** Engine plan commits only the **event shapes**. `Playhead{track_idx,step_idx}` coalesces **per-track keyed** (`track_idx → step_idx`, latest per track per frame), matching architecture-spec §3.4's per-drain set — *not* a single global last-write-wins (8 tracks each emit Playhead). Non-playhead events (transport, `Serialized`, `Error`, `Overflow`) are distinct, un-coalesced. The coalescing algorithm + single-hop-per-batch is app-plan work.
- **E8 — drop policy (all three bounded queues).** **Hot event channel: drop-oldest** (keep newest) + per-track playhead coalescing — so the latest position survives; `Serialized`/`Error` live **only** on the off-RT large channel (never the hot one). **MIDI ring: drop-oldest** (keep chronological) + emit `EngineEvent::Overflow{dropped: u32}` when a push overflows (asserted to encode ≤ `MAX_EVENT_BYTES` so it rides the hot channel). **Command MPSC: drop-oldest** on full (most-recent intent wins; UI resubmits) + the same `Overflow` diagnostic; `engine_submit_command` returns `Ok` (the `Overflow` event makes the drop observable rather than adding an `EngineResult` variant — keeping the header stable).
- **E9 — ~120 Hz budget.** Worst-case sizing (explicit assumptions; recompute if the spec permits a higher BPM ceiling):
  - BPM ceiling **300** → 20 global 16th-steps/sec; max `speed_ratio` 3.0 → 60 steps/sec on the fastest track.
  - Absurd-worst: all 8 tracks at ratio 3.0, every step a hit, every hit ratchet X4 → **1920 Note-Ons/sec** into the MIDI ring; ≤ **480 Playheads/sec** into the hot channel.
  - Hot channel (drained ~120 Hz / 8.33 ms): ≤ 4 events per drain window before per-track coalescing → **depth 32** never fills.
  - MIDI ring (CoreMIDI worker polls ~0.5–1 ms): ≤ 2 entries per poll → **depth 128** holds ~67 ms of burst, far past any worker stall. **Zero drops** under this load; real music has ~30× headroom.
- **E10 — RT→CoreMIDI ring + worker home.** `MidiOutRing` (`heapless::spsc::Queue`) in **`core/src/midi_out.rs`**; CoreMIDI worker + `MIDISend` in **`ffi/src/coremidi.rs`**; spawned at `engine_start`, joined at `engine_stop`; the worker reads endpoint IDs straight from the immutable `Session.midi_destinations` snapshot (no staging, no blocking).
- **E11 — EngineBridge isolation.** This plan commits the **engine-side contract** (the event shapes + batching from E7, and the shutdown ordering from Rules 5/7). The Swift isolation itself is app-plan work; **recommendation for the app plan:** a `final class … @unchecked Sendable` with a private serial `DispatchQueue` guarding the handle (lighter than an `actor`, matches the ~120 Hz drain cadence). `amendments.md` should be updated to record that E11's isolation *decision* is re-scoped to the app plan, so the two docs agree it is resolved-by-deferral, not dropped.

## Error handling

- FFI stays panic-safe: every `extern "C"` body wraps `catch_unwind`; malformed bytes → `ErrDecode`, never an abort; codecs remain total.
- **Drop is degradation, not an error.** Overflow on any bounded queue emits `Overflow` but does not fail playback or command acceptance.
- `engine_serialize` failures return `ErrOther` (synchronous, inline). The async `Serialized` event carries an `Error` on the large channel only if a worker-produced snapshot fails.
- `LoadSession` with a bad envelope/version returns `ErrDecode` (no publish); a good envelope publishes the new session, bumps the reload generation, and emits `FullSnapshot` so the mirror refreshes (reusing an existing variant — no new orphan).

## Testing strategy

- **Pure dispatch-math unit tests (no Engine/threads/clock):** swing offset + cap, humanize jitter + determinism/round-trip, `micro_timing_offset` clamp, speed_ratio Q16.16 accumulation, ratchet X2/X3/X4 expansion, note-off gate value — all as free-function tests.
- **`Engine` host tests (steppable clock):** step wrapping at `length`, transport stop-generation, scheduler quantize-grain + follow-actions, undo one-deep + `length`-exclusion, clipboard `no-midi_note`. Two explicit seams make these possible without spawning threads: (1) `Engine::load_session_for_test(&mut self, session)` publishes a snapshot synchronously; (2) `Engine::begin_play(&mut self)` seeds the RNG (play-start seeding is a callable method, not a side-effect of `engine_start`).
- **`proptest` for algorithms (working agreement):** Roll/Vary/Cut/Trash preserve `length`/`midi_note`/`speed_ratio`; Vary leaves accents unchanged; Paste overwrites `steps`/`length`/`speed_ratio` while preserving `midi_note`; clipboard carries no `midi_note`; each of Roll/Vary/Cut/Paste/Trash pushes exactly one undo snapshot; a `length` change pushes none.
- **C-ABI round-trips:** every added variant (`Overflow{dropped:u32}`, and any transport additions) gets a `ffi_api` round-trip + garbage-bytes-don't-panic test (no orphans).
- **Sync (E6) tests:** feed `SetSyncSource{Link}` + a synthetic `LinkPhase` stream → assert the internal clock stops self-advancing and step position tracks `beats_since_origin`; mirror for `MidiClockTick` (assert 6 ticks = one 16th step).
- **LoadSession C-ABI test:** `engine_serialize` → `engine_submit_command(LoadSession)` → assert the authoritative session swaps and the reload generation bumps and a `FullSnapshot` is emitted; a bad-version envelope returns `ErrDecode` with no swap.
- **COW concurrency test:** writer hammers `store()` (publishing correlated sentinel snapshots) while reader threads `load()` — assert every reader-observed snapshot equals some fully-published `Session`. (COW makes torn reads impossible by construction; this stress test confirms it rather than relying on `loom`.)
- **RT-safety audit:** `/audit-rt` over every RT-path file (`engine.rs::process`, `clock.rs`, `midi.rs`, `midi_out.rs`, the RT entry point) — zero alloc/lock/FFI/CoreMIDI/syscall. `/enforce-ffi` over the boundary.
- **CoreMIDI host test** (`#[cfg(target_os = "macos")]`): creates and tears down its own `MIDIClientRef` + virtual destination as an **explicit test-only exception to Rule 7**, drives the worker against the ring with a small injectable gate, and asserts the expected Note-On then gate-delayed Note-Off bytes arrive with a generous timeout (order/presence, not precise offsets). Assumption recorded: the CoreMIDI server is reachable on standard macOS CI runners (flag for sandboxed/self-hosted).

## Build order (dependency-aware)

0. **Decisions done** (this doc). Add `arc-swap` + `heapless` deps. Wire `Clock` trait + `SteppableClock` (core) + `InstantClock` (ffi) skeletons; `RtState` skeleton.
1. **RT skeleton + ownership.** `Engine` holds `ArcSwap<Session>` + queues + thread handles; `midi_out.rs` ring (`heapless`); `ffi/coremidi.rs` worker (spawn/join); `engine_submit_command` enqueues, `engine_start`/`engine_stop`/`engine_free` lifecycle (stop joins three threads; free joins-then-drops), `engine_drain_events` drains hot→large into out-params. *Milestone: commands flow, RT runs a no-op tick, events drain.*
2. **State worker + COW.** Worker drains MPSC, clone-mutate-publishes via `ArcSwap::store`, manages undo/clipboard, emits `Serialized` on the large channel. Transport atomics. `engine_serialize` reads via `load_full()` inline.
3. **Clock + dispatch math.** Fill `clock.rs` (speed_ratio E1, swing E2, humanize-timing E3, `micro_timing_offset` E3) and `midi.rs` (velocity map E4, ratchet, Note-On/gate E5). Emit `Playhead` + transport on the hot channel. Land the pure-function seams + `load_session_for_test`/`begin_play`.
4. **MIDI handoff end-to-end.** Connect dispatch → `MidiOutRing`; `ffi/coremidi.rs` `MIDISend` + Note-Off synthesis + stop drain-and-drop + all-notes-off; CoreMIDI virtual-endpoint host test green. *Validates the E9 budget empirically.*
5. **Undo + clipboard + algorithms.** `undo.rs`, `clipboard.rs`, `algorithms/{roll,vary}.rs`; wire Cut/Copy/Paste/Trash/Undo; `proptest` for all invariants.
6. **Scheduler + sync.** `scheduler.rs` (quantize, follow-actions, transitions); `LinkPhase`/`MidiClockTick` mode-switch (E6); `LoadSession` apply (+ `FullSnapshot` + reload generation).
7. **Event production + `Overflow`.** Large payloads off-RT; add `EngineEvent::Overflow{dropped:u32}` (E8) with the symmetric codec + C-ABI test.
8. **Audit + tune.** `/audit-rt`, `/enforce-ffi`; all `core/tests` + `ffi/tests` green; run the E9 budget against real depths.

(Phase 5 is independent of Phase 4 once undo/clipboard exist and can proceed in parallel.)

## Deferred / open

- CoreAudio render-callback clock (future localized swap via the `Clock` trait).
- PLL phase-correction for external sync.
- Live-playback-to-physical-device tuning + on-device jitter measurement.
- Per-step gate (deferred — triggers "Model changes ripple" if added).
- E7/E11 Swift-side implementation (app plan); `amendments.md` update recording E11's isolation decision as app-plan-scoped.
- Exact velocity constants, swing curve coefficients, and humanize magnitudes (pinned during implementation; values above are proposed defaults).
