# Sync RT + FFI Audit — 2026-07-24

**Scope:** RT-thread path + FFI boundary of the merged sync work (Ableton Link +
inbound MIDI Clock + changed Command/Event API), branch `app-plan` @ `c391739`
(merged to `main` via PRs #2/#3/#4).

**Method:** `/audit-rt` (Hard Rule 1 — no alloc/lock/FFI/CoreMIDI on the RT hot
path) and `/enforce-ffi` (the 7 C-ABI boundary rules), plus E9 drain-rate
validation. Read-only audit; fixes applied separately (see *Resolution*).

**Status:** findings logged → fixes in progress.

---

## BLOCKERS (RT-safety; Hard Rule 1)

### B1 — `eprintln!` on the RT hot path
`engine/crates/core/src/midi_out.rs:86` (and `:107`, worker-only).

`push_event` is reached from the RT thread every step per track for `Playhead`
(via `process` → `engine.rs:1010`) and for `LinkPeersChanged` (`engine.rs:392`).
`eprintln!("[Rust Engine] push_event: {:?}", ev)` does a stderr write (syscall)
and `Debug`-formats the event (heap alloc/format) — both forbidden on RT.
Unconditional (not debug-gated).

**Fix:** delete the `eprintln!` in `push_event` and `push_large_event`. Any
future RT diagnostics must route through a lock-free bounded log ring drained
off-RT, never an unconditional `eprintln!`.

### B2 — RT thread calls ableton-link-rs directly
`engine/crates/core/src/engine.rs:380–399` (inside `run_rt_loop`,
`SyncSource::Link` arm).

Every Link tick (~500 µs): `self.external_clock.link.try_lock()` →
`link.capture_app_session_state()` (allocates an owned `SessionState` each tick
and contends with Link's internal thread) → `clock().micros()` →
`beat_at_time(..)` → `num_peers()`. Per Ableton's own guidance only `clock()` is
RT-safe; `capture_app_session_state()` is not. This allocates and touches a
library that can lock internally on the RT hot path, and contradicts the file's
own design comment (`engine.rs:141`: "the RT loop reads via atomic loads").

The compliant mechanism is **already scaffolded but dead**: `ExternalClock`
fields `link_beats_micros: AtomicU64` and `link_enabled: AtomicBool`
(`engine.rs:153/155`) are declared and documented as the RT-read path, yet
`link_beats_micros` is **never written and never read** — the RT loop ignores it.
(The `MidiClock` arm at `:364–377` shows the correct pattern: the worker writes
`midi_step_pulses`, the RT loop reads it via `swap(0, AcqRel)`.)

**Fix:** move all Link access off the RT thread. A low-priority Link-poller
thread calls `capture_app_session_state()` + `clock()` + `num_peers()` at a
bounded rate (only while `link_enabled`) and publishes target-beat →
`link_beats_micros` and peers → a new `link_peers` atomic; it also emits
`LinkPeersChanged` off-RT via the (alloc-free) hot channel. The RT Link arm then
mirrors the `MidiClock` arm — read `link_beats_micros`, compute the target 16th
step via a pure function, run `process_one` to catch up — and never touches
`link` or its `Mutex`. The `Mutex<Link>` remains for the worker's
`enable`/`disable` only.

### C3 — `Mutex::try_lock()` on the RT thread
`engine.rs:380`. Non-blocking so it cannot stall RT, but (a) Hard Rule 1 says RT
"never locks," and (b) while the worker holds the mutex during
`link.enable()`/`disable()` (`:534–540`, which `block_on`s — slow), RT's
`try_lock` fails and Link timing silently stalls. **Eliminated entirely by B2's
fix.**

## MEDIUM / MINOR (not blockers)

- **Rule 7 deviation** — Rust owns its own send-`MIDIClientRef`
  (`coremidi.rs:328` `create_client_and_port`). Pre-existing (engine plan),
  rationalized by the handoff ("the engine owns its own internal send-client +
  port"); discovery still correctly lives in Swift (Rust has no
  `MIDIGetDestination`/`MIDIObjectGetStringProperty`). Not sync-introduced.
  Flagged, not fixed here.
- **No BPM clamp** — `engine.rs:711` `SetBpm { bpm } => s.bpm = bpm` with no
  bounds; the UI BPM field is free text. Unbounded BPM makes E9's worst case
  unbounded. **Fix:** clamp to `[20, 400]` in `apply`.
- **FFI debug spam** — `println!`/`eprintln!` at `lib.rs:247/292/311` and
  `coremidi.rs:363`. Not RT (Swift-driven / one-shot threads), but production
  noise. **Fix:** remove or gate behind a feature.

## CLEAN (verified, no action)

- FFI boundary — all 7 enforce-ffi rules hold: every `extern "C"` body is
  `catch_unwind`-wrapped returning `EngineResult`; NULL-safe everywhere
  (including the documented `engine_serialize` non-NULL asymmetry); only raw
  pointers/primitives cross; postcard bytes only (no data-carrying `#[repr(C)]`
  enum); buffer ownership `into_boxed_slice`→`forget`→`engine_free_bytes` with
  `cap == len`; CFStrings released exactly once after CoreMIDI retains
  (`coremidi.rs:338/352/365`); `MIDISend` only on the CoreMIDI worker; RT never
  calls FFI; discovery in Swift.
- Codecs total over **all** sync variants (`command_codec`/`event_codec` are
  thin `postcard` wrappers; `Command`/`EngineEvent` derive ser/de — no manual
  match to miss).
- `engine.rs:1` `#![forbid(unsafe_code)]` intact.
- `clock.rs` (atomics + value-type math), `midi.rs` (pure dispatch),
  `midi_out.rs` queues (`heapless` lock-free, drop-oldest), the `MidiClock` RT
  arm (`:364`, atomic `swap` — the correct model), the `Free` RT arm (`:359`),
  `event_codec.rs` (alloc-free), and the Engine `Mutex`es for
  `undo`/`clipboard`/handles/coremidi (worker-only, never RT).

## E9 — drain-rate validation: PASS (contingent on B1 + BPM clamp)

Worst-case hot-channel production = `Playhead` events: `MAX_TRACKS (8) ×
(BPM/15)` per second. At 300 BPM ≈ 160/s; at 600 BPM ≈ 320/s. The ~120 Hz drain
+ depth-32 hot channel + drop-oldest + **per-track `Playhead` coalescing**
handles this with ~10–20× margin. Ratchet ×4 multiplies **MIDI-ring** traffic
(depth 128, drained by its own worker thread), not the event channel. State
events (`StepChanged`, …) come from the worker applying commands — rate-bounded
by user input + the depth-64 command queue. Large events (`Serialized`/
`FullSnapshot`) ride the separate depth-8 large channel.

Caveat: B1's per-event `eprintln!` is itself an E9 timing threat at high event
rates (unbounded-latency stderr syscalls on RT), and the BPM clamp bounds the
worst case definitively. E9 holds once B1 is fixed and BPM is clamped.

---

## Resolution

All audit-driven fixes applied on `app-plan` (TDD where behavior changed):

- [x] **B1** — removed the RT `eprintln!` from `push_event`/`push_large_event`
  (`midi_out.rs`). Re-audit: the `run_rt_loop` body is now free of any Link call,
  lock, print, or allocation.
- [x] **BPM clamp** — `SetBpm` now clamps to `[MIN_BPM=20, MAX_BPM=400]`
  (`models.rs`/`engine.rs`) and emits the clamped value. TDD:
  `set_bpm_clamps_to_sane_range`.
- [x] **FFI/CoreMIDI debug spam** — removed the per-call prints from
  `engine_submit_command`/`engine_drain_events` (`lib.rs`) and the per-message
  prints from the CoreMIDI worker + `MIDISourceCreate` path (`coremidi.rs`).
  (One legitimate decode-error log retained in `lib.rs`.)
- [x] **B2** — Ableton Link moved off the RT thread. A new off-RT
  `run_link_poller` thread (spawned in `engine_start`, joined in `engine_stop`)
  is the sole caller of `capture_app_session_state`/`clock`/`num_peers`; it
  publishes beat position to the (previously-dead) `link_beats_micros` atomic
  plus a new `link_peers` atomic, and emits `LinkPeersChanged` off-RT. The RT
  `Link` arm now reads one atomic + a pure `target_step_from_link_beats` fn
  (TDD: `link_beats_micros_maps_to_16th_step_target`) — no Link call, no
  `Mutex`, no allocation on the hot path (C3 eliminated).
- [x] **Verify** — `cargo test` green (42 lib + all integration; the lifecycle
  test now spans the 4th poller thread); re-`/audit-rt` clean on the RT path;
  clippy clean on all touched code.

**Outstanding (outside this audit's scope):**
- Pre-existing clippy debt in the merged sync code (clone-on-`Copy`, redundant
  casts, `map_or`, etc.) — the `-D warnings` gate was already red before this
  session. Recommend a separate `clippy --fix` cleanup pass.
- Rule 7 deviation (engine owns its own send-`MIDIClientRef`) — pre-existing,
  rationalized; revisit only if decoupling the engine from CoreMIDI.
