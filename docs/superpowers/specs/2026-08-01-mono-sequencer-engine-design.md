# Mono Sequencer Engine + `midi_kernel` Extraction — Design

> Companion to the implementation plan at `docs/superpowers/plans/2026-08-01-mono-sequencer-engine.md`.
> Origin plan (reviewed): `~/.claude/plans/i-d-like-to-extract-velvety-falcon.md`.
> Source sequencer: `mono_seq/mono sequencer_popup_mod.amxd` (Max for Live device).

## Context

StepForge's Rust musical-time core (`engine/crates/core`, crate `sequencer_engine`) is a **drum** sequencer: `Session → Pattern → Vec<Track> → [Step; 16]`, with `process()` indexing the grid and firing one MIDI note per track. There is no sequencer-model abstraction — the drum model is baked into `Engine`, `RtState`, `process()`, and ~20 command arms.

This design does two things:

1. **Extract** the drum-agnostic **leaf infrastructure** (~60% of `core`) into a reusable `midi_kernel` crate, and migrate the drum `core` onto it — **byte-compatible** with the shipping drum app (Swift + FFI).
2. **Build** a second, independent **melodic mono step sequencer** on top of `midi_kernel`, shipped as a CLAP plugin. The model is a parity extraction of the Max for Live Mono Sequencer at `mono_seq/mono sequencer_popup_mod.amxd` (a fully parseable `ampf` container — patcher JSON begins at byte offset 48).

The two sequencers share leaf infra (`midi_kernel`) but keep **twin concrete engines** — no generic `Engine<M>` refactor of the RT path. Mono dispatch (per-lane independent counters, mono voice, scale snap) is fundamentally different from the drum grid.

### Decisions (locked)

| Decision | Choice |
|---|---|
| Architecture | **Shared `midi_kernel` + twin concrete engines.** Drum `core` migrates onto the kernel; its Swift/FFI contract stays byte-identical. Mono gets its own concrete `mono_engine`. No generic-`Engine` refactor of the RT path. |
| Surfaces | **CLAP plugin only** (desktop host). Pure Rust. No Swift, no FFI for mono. |
| M4L model | **True device parity** for the sequencing model: 6 lanes (incl. Step-enable), 5 directions (incl. `drunk`), straight-note rate whole–1/32, gate 0–400%, scale/key + Conform + Edit-to-scale, 12 patterns, per-lane loop+direction+reset, global step-count. |
| v1 feature tier | **Model-parity + musical essentials** (+ swing, transpose). See §12 for the explicit deferred list. |
| Editor layout | **Tabbed single-lane** (M4L view-selector style). |
| Governing docs | **Amended as part of this work** — `amendments.md`, `CLAUDE.md`, `architecture-spec.md` (see §9). |

---

## 1. Crate topology

New workspace members under `engine/crates/`; `engine/Cargo.toml` `[workspace] members` extended.

```
midi_kernel        stepforge_midi_kernel       NEW  shared leaf infra, #![forbid(unsafe_code)]
core               sequencer_engine            drum — depends on midi_kernel; FFI byte-compat preserved
ffi                sequencer_engine_ffi        drum Swift/FFI — source unchanged via core re-exports
mono_engine        stepforge_mono_engine       NEW  mono Session+dispatch, #![forbid(unsafe_code)]
mono_editor_egui   stepforge_mono_editor_egui  NEW  lane editor, pure-egui, host-free, testable
mono_clap          stepforge_mono_clap         NEW  nih-plug wrapper, MIDI-out, persistence
clap_plugin / editor_egui / xtask                          drum — unchanged
```

Thin shells (`mono_clap`, editor transport/theme) are **cloned**, not over-abstracted, for v1. Honest reuse lives in `midi_kernel`. Extract a shared `editor_shell`/`clap_shell` only if a 3rd sequencer arrives.

---

## 2. `midi_kernel` — extraction from `core`

Drum-agnostic leaf only. The extraction is **internal to the drum build** — no serialized struct moves, so postcard bytes are invariant (verified: `models.rs` structs embed zero kernel types).

| kernel module | source in core | contents moved | drum residue left in `core` |
|---|---|---|---|
| `clock` | `core/src/clock.rs` | `Clock`, `SteppableClock`, `to_q16_16`, `advance_speed_ratio`, `effective_swing`, `swing_offset_micros`, `micro_timing_offset_micros`, `Rng` | — (whole file moves) |
| `host` | `core/src/host.rs` | `HostTransport`, `MidiEvent`, `PendingMidiQueue`, `emit_midi_msg` (from `engine.rs:1356`), **generic `HostRenderState<Rt>`** | — (HostRenderState genericized: only field `rt: RtState` referenced a drum type) |
| `midi_out` | `core/src/midi_out.rs` | `MidiMsg`, `MidiOutRing`, `HotEventChannel`, **generic `CommandQueue<C>` + generic `LargeEventChannel<E>` + `push_event<E>` + `push_large_event<E>`** | — (EngineEvent coupling resolved by genericizing the event channel too; drum instantiates `E = EngineEvent`) |
| `midi` | `core/src/midi.rs` | `build_note_on`, `humanize_velocity`, `build_all_notes_off` | `velocity_for_zone`, `ratchet_count` (these reference `VelocityZone`/`Ratchet`, which live in `models.rs` and stay in core) |
| `scheduler` | `core/src/scheduler.rs` | `SchedulerClock` **+ `QuantizeGrain`** (co-extracted; pure transport grain, no drum semantics) | `all_notes_off_burst` (drum `MAX_TRACKS`) |
| `serde` | `core/src/serde_ext.rs` | **generic `VersionedEnvelope<T>`** + the versioned-envelope pattern | `SessionEnvelope` → `type SessionEnvelope = VersionedEnvelope<Session>`; core keeps `SESSION_FORMAT_VERSION` |
| `models` (kernel-side) | — | `QuantizeGrain` (relocated here from drum `models.rs:167`) | drum `Session`/`Pattern`/`Track`/`Step`/`VelocityZone`/`Ratchet` stay in core `models.rs` |

**External-clock stays in `core`.** `ExternalClock`, `run_link_poller`, `set_link_session` (engine.rs:165/457/787) are drum-session-coupled. Mono is CLAP host-driven and has no standalone sync path.

### Genericization specifics (load-bearing)

- **`HostRenderState<Rt>`** — one type parameter. Fields `rt: Rt`, `pending: PendingMidiQueue`, `next_step_beat`, `sample_time`, `last_block_start_beat`, `was_playing`, `initialized` are all drum-agnostic. Drum supplies `Rt = RtState`; mono supplies `Rt = MonoRt`.
- **`CommandQueue<C>`** — `pub type CommandQueue<C> = Arc<MpMcQueue<C, COMMAND_DEPTH>>;` (one-line genericization of the alias at `midi_out.rs:38`). Single drum instantiation site (`engine.rs:252`).
- **`LargeEventChannel<E>` / `push_event<E>` / `push_large_event<E>`** — genericized over event type so the drum `EngineEvent` does not cross into the kernel. Drum instantiates `E = EngineEvent`; mono instantiates `E = mono_engine::EngineEvent`. (Verification found the prior draft left these concrete — a drum symbol leak.)
- **`VersionedEnvelope<T>`** — pinned field order and name to preserve drum postcard bytes:

  ```rust
  pub struct VersionedEnvelope<T> {
      pub version: u8,   // field 0 — MUST stay first (postcard is field-ORDER sensitive, tagless)
      pub session: T,    // named `session` to keep existing bytes identical (no #[serde(rename)])
  }
  ```

  Version binding via a trait (so `wrap()` knows the constant without the kernel depending on drum types):

  ```rust
  pub trait Versioned {
      const VERSION: u8;
  }
  impl VersionedEnvelope<T> {
      pub fn wrap(session: T) -> Self where T: Versioned { Self { version: T::VERSION, session } }
  }
  ```

  Drum: `impl Versioned for Session { const VERSION: u8 = SESSION_FORMAT_VERSION; }` and `pub type SessionEnvelope = VersionedEnvelope<Session>;`.

### Drum `core` impact = internal refactor only (byte-compatible)

- `Session`/`Pattern`/`Track`/`Step`/`Command`/`EngineEvent`/`SessionEnvelope` **stay in core, field-for-field unchanged** → postcard bytes byte-identical → Swift `SessionMirror`, the cbindgen header, and `ffi_api` round-trip tests unaffected.
- `Engine` struct keeps all fields; `use` paths repoint (`crate::clock::…` → `midi_kernel::clock::…`). `CommandQueue<Command>` becomes a generic instantiation.
- **`core/src/lib.rs` adds re-exports** so the `sequencer_engine::<module>::*` paths the `ffi` crate and tests use keep resolving unchanged:

  ```rust
  pub use midi_kernel::{clock, host, midi_out, midi, scheduler, serde_ext as serde};
  ```

  (Verification found `ffi/src/{coremidi.rs:5, handle.rs:68, lib.rs:19/280/285}` and `ffi/tests/coremidi_host.rs:571/659` import these — re-exports keep ffi source literally untouched.)
- `ffi/src/handle.rs:85` `HostRenderState::new()` → `HostRenderState::<RtState>::new()` (turbofish, once genericized).
- `ffi` crate and `engine/include/sequencer_engine.h` source unchanged.

### cbindgen header — `include` whitelist fix

`HostTransport` and `MidiEvent` are the two `#[repr(C)]` structs the cbindgen header emits with full field bodies. `engine/cbindgen.toml` whitelists `include = ["sequencer_engine"]` with an explicit comment that cbindgen "cannot see their fields and emits them as incomplete types" without parsing the defining crate. Moving them to `midi_kernel` would make cbindgen emit them opaque, breaking the `engine_render` C ABI (the AUv3 host-driven surface — **not** the drum postcard contract).

**Fix:** add `"stepforge_midi_kernel"` to `[parse] include` in `engine/cbindgen.toml`. Then re-run `engine/scripts/build_engine.sh` and assert the regenerated header is byte-identical:

```bash
git diff --exit-code -- engine/include/sequencer_engine.h
```

(This is a Phase-1 gate check — see §10.)

---

## 3. `mono_engine` — model (M4L-parity)

```rust
pub const MAX_STEPS: usize = 64;   // GLOBAL step capacity, shared across all lanes (device: "total number of steps available to all data lanes", 2..=64)
pub const PATTERNS:   usize = 12;
pub const LANES:      usize = 6;   // Pitch, Velocity, Octave, Gate, Repeat, StepEnable

pub struct Session {
    pub bpm: f64,
    pub global_swing_pct: f32,     // rate-gated: only effective at 1/8, 1/16, 1/32 (device rule)
    pub rate: Rate,                // Whole | Half | Quarter | Eighth | Sixteenth | ThirtySecond (straight only)
    pub transpose: i8,             // global semitone offset (-80..=80, device range)
    pub base_note: u8,             // MIDI note for pitch=0/oct=0 (default 60 = C4)
    pub root_key: u8,              // 0..11
    pub scale_mode: ScaleMode,     // Ionian | Dorian | Phrygian | Lydian | Mixolydian | Aeolian | Locrian
    pub conform_to_scale: bool,    // one-shot snap of pitch lane to scale (device "Conform to scale")
    pub edit_to_scale: bool,       // live constrain pitch editing to scale (device "Edit to scale")
    pub active_pattern: usize,
    pub patterns: [Option<Sequence>; PATTERNS],
}

pub struct Sequence {
    pub step_count: usize,         // 2..=64 GLOBAL window over [T; MAX_STEPS] (non-destructive; values beyond window preserved)
    pub pitch:     Lane<Pitch>,    // Pitch    = i8, -12..=+12 bipolar semitones (device pitch bound 12)
    pub velocity:  Lane<Vel>,      // Vel      = u8, 0..=127
    pub octave:    Lane<Octave>,   // Octave   = i8, -4..=+4 (device octave extra1_max 4; v1 range, finalize on Max load)
    pub gate:      Lane<Gate>,     // Gate     = u16, 0..=400 (percent of pulse; device "Adjustable from 0-400%")
    pub repeat:    Lane<Repeat>,   // Repeat   = u8, {0,1,2,3,4} (0=none; best-effort closed set — see note)
    pub step_enable: Lane<StepEnable>, // StepEnable = bool (device 6th lane; per-step gate)
}

pub struct Lane<T: Copy + Default> {
    pub values: [T; MAX_STEPS],
    pub loop_start: usize,         // per-lane loop window (≤ step_count)
    pub loop_end:   usize,         // per-lane loop window
    pub direction:  Direction,     // Up | Down | UpDown | Drunk | Random
    pub reset:      LaneReset,     // Never | OneMeasure | TwoMeasures | FourMeasures | MidiKey
    pub enabled:    bool,
}
```

**Non-destructive global `step_count`** window over a fixed `[T; 64]` — mirrors drum's length-window philosophy. Per-lane independence = **loop-window (start/end within the shared grid) + direction + reset** — *not* an independent per-lane step count (the device's step count is global).

### Notes on parity-vs-uncertainty

- **`drunk` direction** — bounded random walk, semantically distinct from `random` (jumping). Device annotation: "Drunk=stagger from step to step."
- **`repeat` set `{0,1,2,3,4}`** — the repeat lane is internal data (`parameter_type: 3`), not exposed as params, so the exact closed set is not enumerable from the `.amxd`. The device's "Repeat quantize" menu (3 non-redundant modes: all / 2x only / 2x-and-3x) implies value `1` must exist, and `extra1_max = 16`. v1 pins `{0,1,2,3,4}` as a documented best-effort; finalize by loading the device in Max before v1 ships (§12 deferred-list tracks this).
- **`Gate = u16`** (not `u8`) — 0–400% does not fit in a byte.
- **Rate** — straight note values only (whole, 1/2, 1/4, 1/8, 1/16, 1/32). Dotted/triplet variants and Max-pulse >1/32 are deferred (§12).

### Command surface (mono)

`Play | Stop | SetBpm | SetRate | SetSwing | SetTranspose | SetBaseNote | SetKey | SetScale | SetConform | SetEditToScale | QueuePattern{index,quantize} | RetriggerPattern | SetStepValue{lane, idx, value} | SetLaneLoop{lane, start, end} | SetLaneDirection{lane, dir} | SetLaneReset{lane, reset} | SetLaneEnabled{lane, bool} | SetStepCount{count} | RandomizeLane{lane, amount} | ShiftLane{lane, dir} | InitLane{lane} | ConformPitchToScale{lane} | Serialize | LoadSession{bytes}`

`lane` is an enum index over the 6 lanes. `SetLaneLoop` replaces the prior `SetLaneLength` (there is no per-lane length; the loop window is `loop_start`/`loop_end`).

### Event surface (mono)

`PlayStateChanged | BpmChanged | RateChanged | KeyChanged | ScaleChanged | StepChanged{lane, idx, value} | LaneLoopChanged | LaneDirectionChanged | LaneResetChanged | PatternSwitched{index} | PatternQueued{index, quantize} | Playhead{ positions: [usize; 6] } | FullSnapshot{session} | Serialized{bytes} | Error | Overflow`

`Playhead` carries all 6 lane positions **atomically per pulse** — one event per pulse, no coalescing needed (unlike drum's per-track stream).

---

## 4. `mono_engine` — dispatch (the crux)

Mono `process()` runs **per pulse** (host-driven: one pulse per rate-boundary crossing the audio block). Reuses kernel `swing_offset_micros`, `build_note_on`, `humanize_velocity`, `MidiOutRing`, `Rng`.

```rust
pub struct MonoRt {
    pub rng: Rng,
    pub pos: [usize; 6],          // one index per lane
    pub dir_state: [i8; 6],       // travel sign (+1/-1) for UpDown palindrome AND Drunk walk
    pub pulse: u64,
    pub sounding: Option<SoundingNote>,  // the currently sustaining mono note + its absolute note-off sample
}
```

`dir_state` is required (verification): `pos` alone cannot drive a palindrome after a length/loop edit lands mid-window, when `len == 1`, or after a Random/seek relocate. It is a fixed stack array — alloc-free.

Per pulse:
1. **Advance each enabled lane** by its `direction` within `[loop_start, loop_end)`:
   - `Up`: `pos = loop_start + ((pos - loop_start + 1) % len)`
   - `Down`: reverse
   - `UpDown`: step by `dir_state`; flip `dir_state` at `loop_start`/`loop_end`
   - `Drunk`: with probability 0.5 flip `dir_state`; `pos = reflect(pos + dir_state, loop_start, loop_end)` — a bounded random walk that staggers ±1 and reflects at the window ends (never jumps; distinct from `Random`'s uniform jump to any index)
   - `Random`: `pos = rng.range(loop_start, loop_end-1)`
2. **Assemble note** from each lane's *own* current position — lanes advance independently (per-lane direction/loop/reset is the signature feature), so each reads its own index: `interval = pitch[ pos[PITCH] ]`, `oct = octave[ pos[OCTAVE] ]`, `vel = velocity[ pos[VEL] ]`, `gate_pct = gate[ pos[GATE] ]`, `reps = repeat[ pos[REPEAT] ]`, `on = step_enable[ pos[STEPENABLE] ]`.
3. **Absolute pitch** = `base_note + transpose + oct*12 + interval`; if `conform_to_scale`, snap `interval` to the nearest degree of `root_key + scale_mode` via the `scale` module.
4. **Mono voice**: if a note is sounding, emit its note-off first; then — unless `gate == 0` (rest) or `step_enable == false` — note-on(absolute_pitch, vel). `gate_pct` maps to note-off offset: `<100` releases within the pulse (staccato); `>100` sustains across pulses (legato) — held in `sounding` until the next step's note-on. `reps > 0` subdivides the pulse into `reps` ratchets, each a note-on/off pair within the pulse.
5. **Emit `Playhead{ positions }`** (per-lane current indices).

### `scale` module (new, mono_engine)

Nearest-degree snap, **allocation-free**. A scale mode is a fixed 7-interval set; `match ScaleMode { … => &[…; 7] }` returns a `&'static [u8; 7]`, a hand-written min-distance loop (with a `+12` wrap candidate for the octave boundary) picks the nearest. O(7) fixed-array scan + integer arithmetic only — no `Vec`/`HashMap`/`String`. Mirrors the existing `velocity_for_zone`/`ratchet_count` table-lookup pattern (`midi.rs:18-43`). Implementation pinned to a hand-written loop (not `.iter().min_by_key().copied()`) so the `/audit-rt` pass is trivial.

### RT-safety (Hard Rule 1 — CLAP audio path)

Fixed `[T; 64]` arrays + `pos`/`dir_state`/`[usize;6]`/`[i8;6]` + kernel lock-free queues only. No `Vec`/`String`/`format!`/lock/FFI in `process()` or `render_host`. Audited before merge (`/audit-rt`).

The kernel queues (`HotEventChannel`/`LargeEventChannel`/`CommandQueue`) are **safe on the audio thread** — the drum `clap_plugin` already runs `Engine::render_host` on the host audio callback today (`clap_plugin/src/lib.rs:315`); the queues are cadence-independent (drop-on-overflow, never block). `assert_process_allocs` (enabled in `clap_plugin/Cargo.toml`) runtime-enforces no-alloc on the RT thread.

### Host-driven `Engine::render_host` (mono)

A clone of core's (`engine.rs:529`), **not** a free copy — two drum hardcodes must become rate-derived (verification):

- **Boundary increment**: drum hardcodes `rs.next_step_beat += 0.25` (16th). Mono reads `Rate` from the snapshot:

  ```rust
  let pulse_beats = match rate {
      Whole => 4.0, Half => 2.0, Quarter => 1.0, Eighth => 0.5, Sixteenth => 0.25, ThirtySecond => 0.125,
  };
  // in the boundary loop:
  rs.next_step_beat += pulse_beats;   // and respond to SetRate mid-playback (re-read each block)
  ```

- **Bar-align math**: drum's `sixteenths = (into_bar * 4.0)` and `global_step = sixteenths % STEP_COUNT` are 16th-grid-specific. Mono snaps to the **rate grid** (pulses-per-bar derived from `pulse_beats`), not 16ths.

Otherwise reuses kernel `HostRenderState<MonoRt>`, `HostTransport`, `MidiEvent`, `emit_midi_msg`, `PendingMidiQueue`, `MidiOutRing`. `mono_engine` has **no** Link/CoreMIDI/external-clock — host-driven only.

### Stop / seek transitions

The mono midi port is **NoteOn/NoteOff only** (clone of `clap_plugin/src/midi.rs:7`, which drops CC at `:9-14`). So `render_host`'s CC-123 all-notes-off never reaches the host. Drum survives this via a 128×16 NoteOff burst; mono is monophonic, so:

- **On play→stop**: emit one `NoteOff` for `MonoRt.sounding` (if `Some`), then `rs.pending.clear()` and `rs.rt.sounding = None`.
- **On seek/jump** (`engine.rs:609` `jumped`): clear `rs.rt.sounding = None` (a relocate invalidates the prior sustaining note's absolute note-off sample), alongside the existing pending-queue handling.

(Note: drum's `begin_play` wholesale-replaces `RtState` on play-start (`engine.rs:359`), so `sounding` in `rs.rt` is reinitialized on stop→play; the explicit clear is still required for the seek case and for defensive clarity.)

---

## 5. `mono_clap` — near-clone of drum `clap_plugin`

The nih-plug **lifecycle + RT plumbing** is generic and clone-ready (`midi.rs`, `transport.rs`, `params.rs` shape, `ensure_worker` + the PR-#12 re-activation guard, `reset`/`deactivate`, the all-notes-off burst → replaced by mono's single NoteOff). But `lib.rs`/`editor.rs` carry drum residue the clone must strip (verification):

```rust
pub struct MonoSeq {
    engine: Arc<mono_engine::Engine>,
    host_render_state: midi_kernel::host::HostRenderState<MonoRt>,
    sample_rate: f32,
    params: Arc<MonoParams>,
    worker_handle: Mutex<Option<JoinHandle<()>>>,
    ui_state: Arc<RwLock<mono_editor_egui::UiState>>,   // mono UiState, not drum
    midi_buf: Box<[MidiEvent; 1024]>,
    was_playing: bool,
}
```

`mono_clap` must:
- **(a)** omit/replace `demo_session()` (drum kick/snare/hat seed) — mono seeds a default `Sequence`.
- **(b)** set its own `CLAP_ID` / `CLAP_DESCRIPTION` / `CLAP_FEATURES` (drum's is `"MIDI drum sequencer"`).
- **(c)** swap the `ui_state` field type and `editor.rs` for `mono_editor_egui` (drum `editor.rs` decodes `EngineEvent::Playhead{track_idx, step_idx}`; mono decodes `Playhead{positions:[usize;6]}`).
- `process()` / `reset` / `deactivate` / `ensure_worker` carry over (process touches only `engine.render_host` + `midi_buf` + `context.transport()` — never the Mutex/RwLock fields; the `/audit-rt` pass greps `process()` for `.lock()`/`.read()`/`.write()` and asserts zero hits).

`MonoParams`: `#[persist] editor_state: Arc<EguiState>`, `#[persist] session: Arc<RwLock<Vec<u8>>>` (postcard `VersionedEnvelope<Session>`). Same shape as `clap_plugin/src/params.rs:9`.

### nih-plug API notes (verification corrections)

- The send API is **`context.send_event(event)`** per-event — there is no `send_note_events`. Drum calls it at `lib.rs:302/324`.
- The worker is a **manual** `std::thread::Builder::spawn(move || e.run_worker_loop())` (`lib.rs:191`), not nih-plug's `execute_background`. `ensure_worker` (`lib.rs:183`) mirrors drum.
- `#[persist]` on `Vec<u8>` serializes via serde_json as a **JSON number array** (≈4× bloat for a 12×64×6 mono session). If state-file size matters, store the session bytes base64-encoded in a `String` field instead. v1 ships the plain `Vec<u8>` (parity with drum); base64 is a §12 candidate.

`cargo xtask bundle -p stepforge_mono_clap --release` → `engine/target/bundled/stepforge_mono_clap.clap`.

---

## 6. `mono_editor_egui` — tabbed single-lane editor

Pure-egui, host-free, testable (mirrors `editor_egui`). Own `MonoUiState` mirror + `apply(EngineEvent)` (drum `UiState` is track-shaped, not reusable — PR #20's FeelBar made it more drum-specific). `CommandSink` trait re-cloned from `editor_egui/src/lib.rs:13`.

Layout (M4L view-selector style):

```
┌─ StepForge Mono ────────────────────────── Pat 03/12   Rate ▼1/16   Swing 54% ──┐
│ [▶ Play] [■ Stop]  BPM 120.0   Sync: Host   Key ▼C  Scale ▼Aeolian  [☑ Conform] │
│ Xpose -3   Base C4                                                              │
├─────────────────────────────────────────────────────────────────────────────────┤
│ Lanes:  [ Pitch ] Velocity  Octave  Gate  Repeat  StepEnable   ◄ Pat 1/2/3 ►   │
├─────────────────────────────────────────────────────────────────────────────────┤
│ Loop  │◄━━━━━━━━━━●━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━►│  Steps 16  Dir ▼ Up    │
│       │  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16  │  Reset ▼Never         │
│ Pitch │  +7  .  +3  .  . -5  .  +12 .  .  0  .  .  +2  . │  [Rand 40%][◄ ►][Init]│
│       │  ▮      ▮        ▮        ▮         ▮           ▮                      │ ← playhead
├─────────────────────────────────────────────────────────────────────────────────┤
│ Step 5:  pitch +3   (drag ↔ to edit)                                            │
└─────────────────────────────────────────────────────────────────────────────────┘
```

- **Lane tabs** switch the visible lane row (6 lanes; StepEnable toggles per-step on/off).
- **Step row**: per-step cell; vertical drag edits the lane's value (pitch ±semitones, vel 0–127, oct, gate %, repeat, step-enable bool); click toggles rest (gate 0) / disable.
- **Loop ruler** per lane: start/end handles; `Steps` (global) + `Dir` + `Reset` dropdowns; `Randomize{amount}` / `Shift◄►` / `Init` buttons.
- **Global**: transport bar (play/stop/BPM/sync badge — re-cloned from `editor_egui/src/transport.rs`), rate, swing, transpose, base-note, key, scale, Conform + Edit-to-scale toggles, pattern selector (1–12) + queue.
- **Playhead**: per-lane current position overlay, drained from `Playhead` events.

---

## 7. Persistence

`VersionedEnvelope<Session>` (postcard), own `MONO_SESSION_FORMAT_VERSION = 1` (`impl Versioned for Session { const VERSION = 1 }` in mono_engine). CLAP host state via `#[persist] session: Vec<u8>` in `MonoParams`. `Serialize` / `LoadSession{bytes}` round-trip through the envelope (validate version + structural invariants on load, mirroring core's `validate_session` at `engine.rs:130`).

---

## 8. Testing + RT audit

- **`midi_kernel` extraction (zero behavior change):** `cargo test -p sequencer_engine` (all core tests), `cargo test -p sequencer_engine_ffi --test ffi_api` (C-ABI roundtrip), `cargo test -p sequencer_engine_ffi --test coremidi_host` (this one imports moved symbols — must pass via the core re-exports), `cargo check --target aarch64-apple-ios` (iOS via rustup toolchain), `cargo clippy --workspace --all-targets -- -D warnings`, **cbindgen header byte-diff** (`git diff --exit-code -- engine/include/sequencer_engine.h`), then the Swift macOS/iOS app build.
- **VersionedEnvelope byte-equality test**: serialize a fixed `Session` through the old `SessionEnvelope` shape, capture the byte vector, and assert the post-extraction `VersionedEnvelope<Session>` produces identical bytes. Guards the field-order/rename invariant that no other test catches (verification: all existing envelope tests are self-consistent round-trips that would pass silently after a field-order flip).
- **`mono_engine` property tests (`proptest`):** lane direction keeps `pos ∈ [loop_start, loop_end)`; palindrome reverses exactly at ends and stays in-window across length/loop edits (uses `dir_state`); `drunk` stays in-window; scale-quantize is idempotent and lands in-scale; non-destructive `step_count` (values beyond window untouched); ratchet count ∈ {0,1,2,3,4}; `random` stays in-window.
- **RT audit** on mono `process()` + `render_host` (no alloc/lock/FFI) — `/audit-rt`. Includes the `process()`-no-locks grep on `mono_clap/src/lib.rs`.
- **`mono_editor_egui`** host-free egui tests (lane render, drag-edit, loop ruler, 6-lane tab switch), `cargo clippy --all-targets -- -D warnings`.
- **End-to-end:** `cargo xtask bundle -p stepforge_mono_clap --release`, load `.clap` in a host (Ableton Live / Bitwig), verify MIDI notes out + transport sync + persistence.

---

## 9. Governing-doc amendments (part of this work)

Per `amendments.md:3` ("Authoritative list of changes to the two source specs") and `CLAUDE.md:9` (source-of-truth), the refactor diverges from governing docs and must amend them — not follow the lax CLAP-crates precedent (PR #12 skipped this, violating stated process).

- **`docs/specs/amendments.md`** — add entries (E12+) recording: (a) `midi_kernel` extraction + leaf-inventory; (b) twin-concrete-engine architecture; (c) mono CLAP surface; (d) `architecture-spec.md` §1.1/§2 supersession by `CLAUDE.md` for crate topology.
- **`CLAUDE.md`**:
  - *Status* — add the mono edition (in progress in `engine/crates/{mono_engine,mono_editor_egui,mono_clap}`).
  - *Architecture > Workspace split* — enumerate `midi_kernel` as shared leaf; name `mono_engine`/`mono_editor_egui`/`mono_clap`.
  - *Architecture > Two surfaces, one core* — rewrite to **"shared `midi_kernel` + twin concrete engines"** (the current "one core" is literally false post-refactor: `mono_clap` wraps `mono_engine`, not `sequencer_engine`).
  - *Commands > Plugin* — add the mono cargo test/clippy/bundle commands.
  - *Where things live* — add the 4 new crates.
  - *Hard Rule 6* — reword to forbid `unsafe` in **all** musical-time crates (`sequencer_engine`, `midi_kernel`, `mono_engine`), keeping `sequencer_engine_ffi` as the sole `allow(unsafe_code)` crate. (Current text names only `sequencer_engine`; the new forbid crates uphold the spirit but not the letter.)
- **`docs/specs/architecture-spec.md`** §2 Crate Structure — update the crate tree (currently a stale single-`sequencer_engine`/`ffi.rs`-as-module model), or add a pointer to `CLAUDE.md` as authoritative for crate topology. Mirror the Rule-6 reword in §1.3.

---

## 10. Implementation phases

0. **Governing-doc amendments.** Write the `amendments.md` entries + `CLAUDE.md`/`architecture-spec.md` updates (§9). Commit before code lands, so the source-of-truth leads the refactor.
1. **Extract `midi_kernel`.** Create `engine/crates/midi_kernel`, move leaf modules per §2 (incl. `QuantizeGrain` co-extract + `LargeEventChannel<E>`/`push_event<E>`/`push_large_event<E>` genericization + `VersionedEnvelope<T>` pinned). Migrate `core` onto it (re-exports + turbofish). **Gate:** all drum tests + `ffi_api` + `coremidi_host` + iOS + clippy + **cbindgen header byte-diff** + **VersionedEnvelope byte-equality test** + Swift app build — all green before touching mono.
2. **`mono_engine` model.** `Session`/`Sequence`/`Lane`/`Direction`/`LaneReset`/`Rate`/`ScaleMode`/`Command`/`Event` + `scale` module + `VersionedEnvelope` + `validate`. Property tests (no dispatch yet).
3. **`mono_engine` dispatch.** `MonoRt` (with `dir_state`), `process()` (per-lane direction advance incl. drunk, scale quantize, mono voice + gate + ratchet), host-driven `Engine` + `render_host` (rate-derived `pulse_beats` + rate-grid bar-align), stop/seek note-off handling. RT audit.
4. **`mono_editor_egui`.** `MonoUiState` + `apply()`, tabbed 6-lane editor + loop ruler + scale/pattern/transpose controls. Editor tests.
5. **`mono_clap`.** nih-plug wrapper (strip drum residue per §5), `MonoParams` persistence, MIDI-out, worker lifecycle. Bundle `.clap`; manual host test.

---

## 11. Verification (end-to-end)

```bash
# Phase 1 gate — drum unchanged
cd engine
cargo test -p sequencer_engine
cargo test -p sequencer_engine_ffi --test ffi_api
cargo test -p sequencer_engine_ffi --test coremidi_host
cargo check --target aarch64-apple-ios          # via rustup toolchain, not Homebrew cargo
cargo clippy --workspace --all-targets -- -D warnings
# cbindgen header byte-identical
git diff --exit-code -- engine/include/sequencer_engine.h
# Swift app still builds (from repo root):
./build_install_macos.sh                         # or xcodebuild iOS sim

# Mono
cargo test -p stepforge_mono_engine              # + proptest invariants
cargo test -p stepforge_mono_editor_egui
cargo clippy -p stepforge_midi_kernel -p stepforge_mono_engine -p stepforge_mono_editor_egui -p stepforge_mono_clap --all-targets -- -D warnings
cargo xtask bundle -p stepforge_mono_clap --release
# -> engine/target/bundled/stepforge_mono_clap.clap  → load in host, verify MIDI + transport + state
```

---

## 12. Deferred (explicit, revisitable)

Out of v1, layered to extend cleanly later:

- **Rate variants**: dotted + triplet (device "Pulse value" enum has 18 entries); Max-pulse >1/32 (to 1/128).
- **`repeat` closed set**: pin from the device by loading it in Max (v1 uses `{0,1,2,3,4}` best-effort).
- **Octave range**: finalize ±4 vs device `extra1_max=4` on Max load.
- **Host routing**: 7 MIDI-input modes, CC loop control (CC1/2 all loops, CC3/4 pitch, …), MIDI-thru.
- **Musical extras**: repeat-velocity-scaling (up/down/tri/flat), per-lane "MIDI key" reset trigger, follow-actions.
- **`#[persist]` state size**: base64-encode the session `Vec<u8>` if the JSON number-array bloat matters.
- **3rd-sequencer shells**: extract a shared `editor_shell`/`clap_shell` only when warranted.

---

## Appendix: verification findings → resolution

This design was reviewed against the codebase by a 7-dimension adversarial verification pass (32 agents). Material findings and how each is addressed:

- **VersionedEnvelope byte-trap** (postcard field-order sensitive; no golden test) → §2 pins field 0 = `version`, payload named `session`, `Versioned` trait; §8 adds a byte-equality test.
- **cbindgen `include` whitelist** (HostTransport/MidiEvent would emit opaque) → §2 adds `stepforge_midi_kernel` to `include`; §10 gate asserts header byte-diff.
- **Drum symbol leak** (`EngineEvent` in midi_out; `QuantizeGrain` blocks SchedulerClock) → §2 genericizes `LargeEventChannel<E>`/`push_event<E>`/`push_large_event<E>` and co-extracts `QuantizeGrain`.
- **ffi re-exports + turbofish** → §2 core `pub use midi_kernel::{…}` + `handle.rs:85` turbofish.
- **M4L model errors** (4 directions→5 incl. drunk; rate 1/8→whole; gate 200→400; repeat set unprovable; 5→6 lanes; per-lane length→global) → §3 parity model.
- **render_host "near-copy" understated** (hardcoded `+=0.25`, 16th bar-align) → §4 rate-derived `pulse_beats` + rate-grid bar-align.
- **CC123 contradiction** (midi port drops CC) → §4 single `NoteOff` for `sounding` on stop/seek.
- **Rule 6 letter gap** → §9 reword.
- **"Two surfaces, one core" false** → §9 rewrite.
- **clap_plugin not 100% generic** (demo_session, CLAP_DESCRIPTION, drum ui_state, drum editor.rs) → §5 strip list.
- **nih-plug imprecision** (`send_event` not `send_note_events`; manual worker not `execute_background`; `#[persist]` Vec<u8> JSON bloat) → §5 notes.
- **MonoRt palindrome `dir_state`** → §4 `dir_state:[i8;6]`.
- **RT queue-reuse on audio thread** — cleared: drum already runs `render_host` on the host audio callback; queues are cadence-independent drop-on-overflow. No action.
- **Drum postcard byte-compat** — confirmed solid: `models.rs` structs embed zero kernel types. No action.
