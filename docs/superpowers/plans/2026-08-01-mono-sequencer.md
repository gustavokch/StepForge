# Mono Sequencer Implementation Plan (Plan B)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a second, independent melodic mono step sequencer — an M4L-parity CLAP plugin — on top of the `midi_kernel` extracted in Plan A. Six lanes (incl. Step-enable), five directions (incl. drunk), per-lane independent loop/direction/reset, mono voice, scale quantize, 12 patterns, MIDI-out, persistence.

**Architecture:** Four new crates: `stepforge_mono_engine` (model + RT dispatch, `#![forbid(unsafe_code)]`, consumes `midi_kernel` in-process), `stepforge_mono_editor_egui` (pure-egui tabbed lane editor, host-free testable), `stepforge_mono_clap` (nih-plug wrapper, near-clone of drum `clap_plugin` with drum residue stripped). Twin concrete engines — no generic `Engine<M>`; mono has its own `Engine` + `process()` + `render_host`.

**Tech Stack:** Rust 2021, `midi_kernel` (from Plan A), `nih-plug` + `nih-plug-egui` (pinned rev `f36931f`, same as drum), `heapless`, `postcard`, `proptest`, egui.

**Spec:** `docs/superpowers/specs/2026-08-01-mono-sequencer-engine-design.md` §3–§8, §9 (mono-scope amendments), §10 phases 2–5, §11, §12.

## Global Constraints

- **Prerequisite:** Plan A (`2026-08-01-midi-kernel-extraction.md`) fully merged and green. `midi_kernel` exports `clock`, `host` (incl. `HostRenderState<Rt>`, `HostTransport`, `MidiEvent`, `PendingMidiQueue`, `emit_midi_msg`), `midi_out` (incl. `CommandQueue<C>`, `MidiOutRing`, `HotEventChannel`, `build_note_on`, `humanize_velocity`), `scheduler`, `serde_ext` (`VersionedEnvelope<T>`, `Versioned`).
- **RT-safety (Hard Rule 1):** mono `process()` / `render_host` — fixed `[T; 64]` arrays, `pos`/`dir_state` arrays, kernel lock-free queues only. No `Vec`/`String`/`format!`/lock/FFI. `mono_clap`'s `process()` touches no Mutex/RwLock field. Audited via `/audit-rt`.
- `mono_engine` is `#![forbid(unsafe_code)]` (Hard Rule 6).
- **M4L-parity model:** 6 lanes, 5 directions, rate whole–1/32 (straight), gate 0–400 (`u16`), repeat `{0,1,2,3,4}`, global step-count 2–64 + per-lane loop/direction/reset.
- **In-process command path:** mono `Command` is sent by value via `CommandSink` → `engine.submit_command(Command)` → `CommandQueue<Command>` (typed, no codec). `EngineEvent` emitted encoded on `HotEventChannel` (fixed `[u8; N]`) → drained + postcard-decoded by the editor (event codec needed). This mirrors the drum CLAP path.
- `mono_clap` midi port is **NoteOn/NoteOff only** — CC 123 is dropped; stop emits one `NoteOff` for the sustaining mono note.
- Run engine commands from `engine/`.

---

### Task 1: `mono_engine` crate skeleton

**Files:**
- Create: `engine/crates/mono_engine/Cargo.toml`, `engine/crates/mono_engine/src/lib.rs`
- Modify: `engine/Cargo.toml` (workspace members + workspace.dependencies)

**Interfaces:**
- Produces: empty crate `stepforge_mono_engine`, `#![forbid(unsafe_code)]`, depends on `stepforge_midi_kernel`.

- [ ] **Step 1: Manifest**

```toml
# engine/crates/mono_engine/Cargo.toml
[package]
name = "stepforge_mono_engine"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
path = "src/lib.rs"

[dependencies]
stepforge_midi_kernel = { workspace = true }
serde = { workspace = true }
postcard = { workspace = true }
arc-swap = { workspace = true }
heapless = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }
```

- [ ] **Step 2: lib.rs**

```rust
// engine/crates/mono_engine/src/lib.rs
#![forbid(unsafe_code)]
//! stepforge_mono_engine — melodic mono step sequencer (M4L-parity).
//! Concrete twin of the drum sequencer_engine; shares leaf infra via
//! midi_kernel. No Swift, no FFI — consumed in-process by mono_clap.

pub mod models;
pub mod scale;
pub mod command;
pub mod event;
pub mod event_codec;
pub mod engine;
```

- [ ] **Step 3: Register + verify + commit**

```toml
# engine/Cargo.toml — members + workspace.dependencies
members = [..., "crates/mono_engine"]
# add: stepforge_mono_engine = { path = "crates/mono_engine" }
```

```bash
cd engine
cargo check -p stepforge_mono_engine  # PASS (modules created in later tasks; comment pub mod lines for now)
cd /Users/gus/Git/StepForge
git add engine/crates/mono_engine engine/Cargo.toml
git commit -m "feat(mono_engine): scaffold crate

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Model — `Session`, `Sequence`, `Lane`, enums

**Files:**
- Create: `engine/crates/mono_engine/src/models.rs`

**Interfaces:**
- Produces: `Session`, `Sequence`, `Lane<T>`, `Direction`, `LaneReset`, `Rate`, `ScaleMode`, lane value types, `Default` impls, `validate_session`.

- [ ] **Step 1: Write the model (full)**

```rust
// engine/crates/mono_engine/src/models.rs
//! M4L-parity mono sequencer model. Non-destructive global `step_count` window
//! over fixed [T; 64]; per-lane independence = loop-window + direction + reset.
use serde::{Deserialize, Serialize};

pub const MAX_STEPS: usize = 64;
pub const PATTERNS: usize = 12;
pub const LANES: usize = 6;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Session {
    pub bpm: f64,
    pub global_swing_pct: f32,
    pub rate: Rate,
    pub transpose: i8,
    pub base_note: u8,
    pub root_key: u8,            // 0..11
    pub scale_mode: ScaleMode,
    pub conform_to_scale: bool,  // dispatch-time pitch snap
    pub edit_to_scale: bool,     // editor-input constrain (consumed by editor)
    pub active_pattern: usize,
    pub patterns: [Option<Sequence>; PATTERNS],
}

impl Default for Session {
    fn default() -> Self {
        let mut patterns: [Option<Sequence>; PATTERNS] = Default::default();
        for p in patterns.iter_mut() { *p = Some(Sequence::default()); }
        Self {
            bpm: 120.0, global_swing_pct: 0.0, rate: Rate::Sixteenth,
            transpose: 0, base_note: 60, root_key: 0, scale_mode: ScaleMode::Aeolian,
            conform_to_scale: false, edit_to_scale: false,
            active_pattern: 0, patterns,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Sequence {
    pub step_count: usize,               // 2..=64 global window
    pub pitch:       Lane<Pitch>,        // Pitch = i8, -12..=12
    pub velocity:    Lane<Vel>,          // Vel  = u8, 0..=127
    pub octave:      Lane<Octave>,       // Octave = i8, -4..=4
    pub gate:        Lane<Gate>,         // Gate = u16, 0..=400 (%)
    pub repeat:      Lane<Repeat>,       // Repeat = u8, {0,1,2,3,4}
    pub step_enable: Lane<StepEnable>,   // StepEnable = bool
}

impl Sequence {
    fn default_lane<T: Copy + Default>() -> Lane<T> {
        Lane { values: [T::default(); MAX_STEPS], loop_start: 0, loop_end: 16,
               direction: Direction::Up, reset: LaneReset::Never, enabled: true }
    }
}

impl Default for Sequence {
    fn default() -> Self {
        Self {
            step_count: 16,
            pitch:       Sequence::default_lane(),   // Pitch default 0
            velocity:    Sequence::default_lane(),   // Vel — set a sensible default per-step below
            octave:      Sequence::default_lane(),
            gate:        Lane { values: [100; MAX_STEPS], ..Sequence::default_lane() },
            repeat:      Sequence::default_lane(),
            step_enable: Lane { values: [true; MAX_STEPS], ..Sequence::default_lane() },
        }
    }
}
// NOTE: the `..Sequence::default_lane()` struct-update needs a concrete Lane; write a helper
// `fn lane_with<T>(values: [T; MAX_STEPS]) -> Lane<T>` to avoid the Default-bound friction.

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Lane<T: Copy + Default> {
    pub values: [T; MAX_STEPS],
    pub loop_start: usize,
    pub loop_end: usize,
    pub direction: Direction,
    pub reset: LaneReset,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Direction { #[default] Up, Down, UpDown, Drunk, Random }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LaneReset { #[default] Never, OneMeasure, TwoMeasures, FourMeasures, MidiKey }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Rate { Whole, Half, Quarter, #[default] Eighth, Sixteenth, ThirtySecond }

impl Rate {
    /// Beats per pulse (quarter-note = 1 beat). Used by render_host's boundary loop.
    pub fn pulse_beats(self) -> f64 {
        match self { Whole => 4.0, Half => 2.0, Quarter => 1.0,
                     Eighth => 0.5, Sixteenth => 0.25, ThirtySecond => 0.125 }
    }
    /// Swing is only effective at 1/8, 1/16, 1/32 (device rule).
    pub fn swing_allowed(self) -> bool {
        matches!(self, Rate::Eighth | Rate::Sixteenth | Rate::ThirtySecond)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScaleMode {
    Ionian, Dorian, Phrygian, Lydian, Mixolydian,
    #[default] Aeolian, Locrian,
}

// Lane value newtypes (semantic clarity + future range validation):
pub type Pitch = i8;        // -12..=12
pub type Vel = u8;          // 0..=127
pub type Octave = i8;       // -4..=4
pub type Gate = u16;        // 0..=400 (% of pulse)
pub type Repeat = u8;       // {0,1,2,3,4}, 0 = none
pub type StepEnable = bool;

/// Structural validation on load (mirrors drum validate_session at engine.rs:130).
pub fn validate_session(s: &Session) -> bool {
    (20.0..=400.0).contains(&s.bpm)
        && s.root_key < 12
        && s.active_pattern < PATTERNS
        && s.patterns.iter().all(|p| p.as_ref().map_or(true, validate_sequence))
}

fn validate_sequence(seq: &Sequence) -> bool {
    (2..=MAX_STEPS).contains(&seq.step_count)
        && seq.pitch.loop_start < seq.pitch.loop_end.min(MAX_STEPS)
        // ...same check for each lane's loop window...
        && seq.gate.values.iter().all(|&g| g <= 400)
        && seq.repeat.values.iter().all(|&r| r <= 4)
        && seq.velocity.values.iter().all(|&v| v <= 127)
}
```

- [ ] **Step 2: Unit test for defaults + validate**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_session_validates() { assert!(validate_session(&Session::default())); }
    #[test]
    fn rate_pulse_beats_correct() {
        assert_eq!(Rate::Sixteenth.pulse_beats(), 0.25);
        assert_eq!(Rate::ThirtySecond.pulse_beats(), 0.125);
        assert_eq!(Rate::Whole.pulse_beats(), 4.0);
    }
    #[test]
    fn swing_only_at_eighth_sixteenth_thirtysecond() {
        assert!(Rate::Eighth.swing_allowed() && Rate::Sixteenth.swing_allowed()
                && Rate::ThirtySecond.swing_allowed());
        assert!(!Rate::Whole.swing_allowed() && !Rate::Quarter.swing_allowed());
    }
}
```

- [ ] **Step 3: Run + commit**

```bash
cd engine
cargo test -p stepforge_mono_engine models
cargo clippy -p stepforge_mono_engine -- -D warnings
cd /Users/gus/Git/StepForge
git add engine/crates/mono_engine/src/models.rs
git commit -m "feat(mono_engine): Session/Sequence/Lane model (M4L-parity, 6 lanes)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: `scale` module — allocation-free nearest-degree snap

**Files:**
- Create: `engine/crates/mono_engine/src/scale.rs`

**Interfaces:**
- Produces: `scale::snap_to_scale(semitones: i8, root_key: u8, mode: ScaleMode) -> i8` (RT-safe, no heap).

- [ ] **Step 1: Write the scale module**

```rust
// engine/crates/mono_engine/src/scale.rs
//! Allocation-free scale quantize. Each mode is a fixed 7-interval set;
//! nearest-degree snap is a hand-written O(7) min-distance scan (no iter
//! collector, no HashMap) — /audit-ir trivially clean.
use crate::models::ScaleMode;

/// Semitone intervals from the root for each degree, per mode.
const SCALE_INTERVALS: [[u8; 7]; 7] = [
    [0, 2, 4, 5, 7, 9, 11],  // Ionian (major)
    [0, 2, 3, 5, 7, 9, 10],  // Dorian
    [0, 1, 3, 5, 7, 8, 10],  // Phrygian
    [0, 2, 4, 6, 7, 9, 11],  // Lydian
    [0, 2, 4, 5, 7, 9, 10],  // Mixolydian
    [0, 2, 3, 5, 7, 8, 10],  // Aeolian (minor)
    [0, 1, 3, 5, 6, 8, 10],  // Locrian
];

fn intervals(mode: ScaleMode) -> &'static [u8; 7] {
    match mode {
        ScaleMode::Ionian => &SCALE_INTERVALS[0], ScaleMode::Dorian => &SCALE_INTERVALS[1],
        ScaleMode::Phrygian => &SCALE_INTERVALS[2], ScaleMode::Lydian => &SCALE_INTERVALS[3],
        ScaleMode::Mixolydian => &SCALE_INTERVALS[4], ScaleMode::Aeolian => &SCALE_INTERVALS[5],
        ScaleMode::Locrian => &SCALE_INTERVALS[6],
    }
}

/// Snap a semitone offset (relative to base_note) to the nearest scale degree
/// of `root_key + mode`. Returns the snapped offset. Root-relative: caller
/// subtracts root_key before passing if pitch lane is absolute; here we operate
/// on the within-octave interval.
pub fn snap_to_scale(within_octave: i8, root_key: u8, mode: ScaleMode) -> i8 {
    let intervals = intervals(mode);
    // Normalize into [0,12) relative to root.
    let mut d = within_octave.wrapping_sub(root_key as i8) % 12;
    if d < 0 { d += 12; }
    // Hand-written min-distance scan (with +12 wrap candidate for octave boundary).
    let mut best = 0i8;
    let mut best_dist = i32::MAX;
    for (i, &iv) in intervals.iter().enumerate() {
        let dist = ((d as i32) - (iv as i32)).abs();
        if dist < best_dist { best_dist = dist; best = i as i8; }
        let dist_wrap = 12 - dist;  // wrap-around candidate
        if dist_wrap < best_dist { best_dist = dist_wrap; best = i as i8; }
    }
    intervals[best as usize] as i8 + root_key as i8
}
```

- [ ] **Step 2: Test idempotence + in-scale + root-shift**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snap_is_idempotent() {
        for &root in &[0u8, 3, 7] {
            for mode in [ScaleMode::Aeolian, ScaleMode::Ionian, ScaleMode::Phrygian] {
                let s = snap_to_scale(5, root, mode);
                assert_eq!(snap_to_scale(s - root, root, mode), s);  // already in-scale → unchanged
            }
        }
    }
    #[test]
    fn snap_lands_on_a_scale_degree() {
        let intervals = SCALE_INTERVALS[5]; // Aeolian
        let snapped = snap_to_scale(1, 0, ScaleMode::Aeolian);
        assert!(intervals.contains(&(snapped as u8)));  // 1 (minor 2nd) → 1
    }
}
```

- [ ] **Step 3: Run + commit**

```bash
cd engine
cargo test -p stepforge_mono_engine scale
cargo clippy -p stepforge_mono_engine -- -D warnings
cd /Users/gus/Git/StepForge
git add engine/crates/mono_engine/src/scale.rs
git commit -m "feat(mono_engine): scale module — alloc-free nearest-degree snap

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Persistence — `VersionedEnvelope<Session>` + round-trip

**Files:**
- Create: `engine/crates/mono_engine/src/persistence.rs`
- Modify: `engine/crates/mono_engine/src/lib.rs` (add `pub mod persistence;`)

**Interfaces:**
- Produces: `MONO_SESSION_FORMAT_VERSION = 1`, `impl Versioned for Session`, `SessionEnvelope` alias, `serialize`/`deserialize`.

- [ ] **Step 1: Write persistence**

```rust
// engine/crates/mono_engine/src/persistence.rs
use crate::models::Session;
use midi_kernel::serde_ext::{Versioned, VersionedEnvelope};

pub const MONO_SESSION_FORMAT_VERSION: u8 = 1;
pub type SessionEnvelope = VersionedEnvelope<Session>;

impl Versioned for Session {
    const VERSION: u8 = MONO_SESSION_FORMAT_VERSION;
}

pub fn serialize(session: &Session) -> Vec<u8> {
    postcard::to_allocvec(&SessionEnvelope::wrap(session.clone())).expect("serialize")
}

pub fn deserialize(bytes: &[u8]) -> Result<Session, postcard::Error> {
    let env: SessionEnvelope = postcard::from_bytes(bytes)?;
    if env.version != MONO_SESSION_FORMAT_VERSION {
        return Err(postcard::Error::UnexpectedEnd);  // version mismatch — caller emits Error event
    }
    Ok(env.session)
}
```

- [ ] **Step 2: Round-trip test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_roundtrips() {
        let s = Session::default();
        let bytes = serialize(&s);
        let back = deserialize(&bytes).unwrap();
        assert_eq!(back, s);
    }
    #[test]
    fn envelope_version_byte_is_one() {
        let bytes = serialize(&Session::default());
        assert_eq!(bytes[0], MONO_SESSION_FORMAT_VERSION);
    }
}
```

- [ ] **Step 3: Run + commit**

```bash
cd engine
cargo test -p stepforge_mono_engine persistence
cd /Users/gus/Git/StepForge
git add engine/crates/mono_engine/src/persistence.rs engine/crates/mono_engine/src/lib.rs
git commit -m "feat(mono_engine): versioned persistence (MONO_SESSION_FORMAT_VERSION=1)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: `Command` + `EngineEvent` enums

**Files:**
- Create: `engine/crates/mono_engine/src/command.rs`, `engine/crates/mono_engine/src/event.rs`

**Interfaces:**
- Produces: `Command` (sent by-value via `CommandSink`), `EngineEvent` (emitted encoded on `HotEventChannel`).

- [ ] **Step 1: Command enum**

```rust
// engine/crates/mono_engine/src/command.rs
use crate::models::{Direction, LaneReset, Rate, ScaleMode};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Play, Stop,
    SetBpm(f64), SetRate(Rate), SetSwing(f32), SetTranspose(i8), SetBaseNote(u8),
    SetKey(u8), SetScale(ScaleMode), SetConform(bool), SetEditToScale(bool),
    QueuePattern { index: usize, quantize: midi_kernel::models::QuantizeGrain },
    RetriggerPattern,
    SetStepValue { lane: usize, idx: usize, value: i64 },  // value coerced per-lane
    SetLaneLoop { lane: usize, start: usize, end: usize },
    SetLaneDirection { lane: usize, dir: Direction },
    SetLaneReset { lane: usize, reset: LaneReset },
    SetLaneEnabled { lane: usize, enabled: bool },
    SetStepCount { count: usize },
    RandomizeLane { lane: usize, amount: f32 },
    ShiftLane { lane: usize, dir: ShiftDir },
    InitLane { lane: usize },
    ConformPitchToScale { lane: usize },
    Serialize, LoadSession(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftDir { Left, Right }
```

- [ ] **Step 2: Event enum**

```rust
// engine/crates/mono_engine/src/event.rs
use crate::models::{Direction, LaneReset, Rate, ScaleMode, Session};

#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    PlayStateChanged(bool),
    BpmChanged(f64), RateChanged(Rate),
    KeyChanged(u8), ScaleChanged(ScaleMode),
    StepChanged { lane: usize, idx: usize, value: i64 },
    LaneLoopChanged { lane: usize, start: usize, end: usize },
    LaneDirectionChanged { lane: usize, dir: Direction },
    LaneResetChanged { lane: usize, reset: LaneReset },
    PatternSwitched { index: usize },
    PatternQueued { index: usize, quantize: midi_kernel::models::QuantizeGrain },
    Playhead { positions: [usize; crate::models::LANES] },
    FullSnapshot(Session),
    Serialized(Vec<u8>),
    Error(String),
    Overflow,
}
```

- [ ] **Step 3: Compile + commit**

```bash
cd engine
cargo check -p stepforge_mono_engine
cargo clippy -p stepforge_mono_engine -- -D warnings
cd /Users/gus/Git/StepForge
git add engine/crates/mono_engine/src/command.rs engine/crates/mono_engine/src/event.rs
git commit -m "feat(mono_engine): Command + EngineEvent surfaces

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: `event_codec` — encode/decode events into fixed buffers (RT-safe)

Events cross the RT→editor boundary as bytes on `HotEventChannel` (fixed `[u8; MAX_EVENT_BYTES]`). Mirror drum's `encode_event_into` pattern — never `Vec` on RT.

**Files:**
- Create: `engine/crates/mono_engine/src/event_codec.rs`

**Interfaces:**
- Produces: `encode_event_into(ev, buf: &mut [u8]) -> Option<usize>` (RT, no alloc), `decode_event(bytes: &[u8]) -> Option<EngineEvent>`.

- [ ] **Step 1: Write the codec**

```rust
// engine/crates/mono_engine/src/event_codec.rs
//! RT-safe event encode into a caller-provided fixed buffer (no Vec on RT);
//! decode on the editor side. Large payloads (FullSnapshot/Serialized/Error)
//! ride the LargeEventChannel instead — only small events are encoded here.

use crate::event::EngineEvent;

/// Encode a small event into `buf`; returns the byte length, or None if it
/// does not fit (then route via LargeEventChannel).
pub fn encode_event_into(ev: &EngineEvent, buf: &mut [u8]) -> Option<usize> {
    match ev {
        EngineEvent::Playhead { positions } => postcard::to_slice(&(0u8, positions), buf).ok().map(|s| s.len()),
        EngineEvent::PlayStateChanged(b) => postcard::to_slice(&(1u8, b), buf).ok().map(|s| s.len()),
        EngineEvent::BpmChanged(x) => postcard::to_slice(&(2u8, x), buf).ok().map(|s| s.len()),
        // ...one tag byte per variant...
        _ => None,  // large events → LargeEventChannel
    }
}

pub fn decode_event(bytes: &[u8]) -> Option<EngineEvent> {
    let tag: u8 = postcard::from_bytes(bytes).ok()?;
    match tag {
        0 => { let (_, p): (u8, [usize; crate::models::LANES]) = postcard::from_bytes(bytes).ok()?; Some(EngineEvent::Playhead{positions:p}) }
        1 => { let (_, b): (u8, bool) = postcard::from_bytes(bytes).ok()?; Some(EngineEvent::PlayStateChanged(b)) }
        // ...etc...
        _ => None,
    }
}
```

(Flesh out every variant's tag; the `MAX_EVENT_BYTES` budget — reuse drum's 128 — bounds the largest small event. `Playhead{positions:[usize;6]}` ≈ 10 bytes, well within budget.)

- [ ] **Step 2: Round-trip test for the Playhead event**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playhead_roundtrips_through_fixed_buffer() {
        let ev = EngineEvent::Playhead { positions: [0,3,5,7,2,9] };
        let mut buf = [0u8; 128];
        let n = encode_event_into(&ev, &mut buf).unwrap();
        let back = decode_event(&buf[..n]).unwrap();
        assert_eq!(back, ev);
    }
}
```

- [ ] **Step 3: Run + commit**

```bash
cd engine
cargo test -p stepforge_mono_engine event_codec
cd /Users/gus/Git/StepForge
git add engine/crates/mono_engine/src/event_codec.rs
git commit -m "feat(mono_engine): RT-safe event codec (fixed-buffer encode/decode)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: `MonoRt` + lane-direction advance (property-tested)

The dispatch crux — independent per-lane position advancement.

**Files:**
- Create: `engine/crates/mono_engine/src/dispatch.rs`

**Interfaces:**
- Produces: `MonoRt`, `advance_lane(pos, dir_state, direction, loop_start, loop_end, rng)`.

- [ ] **Step 1: Write MonoRt + advance**

```rust
// engine/crates/mono_engine/src/dispatch.rs
use crate::models::{Direction, LANES};
use midi_kernel::clock::Rng;

pub struct MonoRt {
    pub rng: Rng,
    pub pos: [usize; LANES],
    pub dir_state: [i8; LANES],  // +1/-1 travel sign (UpDown + Drunk)
    pub pulse: u64,
    pub sounding: Option<SoundingNote>,
}

pub struct SoundingNote {
    pub note: u8,
    pub off_abs_sample: u64,
}

impl MonoRt {
    pub fn new(seed: u64) -> Self {
        Self { rng: Rng::new(seed), pos: [0; LANES], dir_state: [1; LANES], pulse: 0, sounding: None }
    }
}

/// Advance one lane's position by its direction within [loop_start, loop_end).
/// `len = loop_end - loop_start`. Returns the new position; updates `dir_state`
/// in place for UpDown/Drunk.
pub fn advance_lane(
    pos: usize, dir_state: &mut i8, direction: Direction,
    loop_start: usize, loop_end: usize, rng: &mut Rng,
) -> usize {
    let len = loop_end.saturating_sub(loop_start).max(1);
    let local = pos.saturating_sub(loop_start) % len;
    match direction {
        Direction::Up => loop_start + (local + 1) % len,
        Direction::Down => loop_start + (local + len - 1) % len,
        Direction::UpDown => {
            let mut d = *dir_state;
            let mut next = local as i32 + d as i32;
            if next >= len as i32 { d = -1; next = (len as i32 - 2).max(0); }
            if next < 0 { d = 1; next = 1.min(len as i32 - 1); }
            *dir_state = d;
            loop_start + next as usize
        }
        Direction::Drunk => {
            // bounded random walk: 50% flip sign, step ±1, reflect at ends
            if rng.range(0, 2) == 0 { *dir_state = -*dir_state; }
            let mut next = local as i32 + *dir_state as i32;
            if next >= len as i32 { next = (len as i32 - 2).max(0); *dir_state = -1; }
            if next < 0 { next = 1.min(len as i32 - 1); *dir_state = 1; }
            loop_start + next as usize
        }
        Direction::Random => loop_start + rng.range(0, len),
    }
}
```

(`Rng::range` + `Rng::new` are the kernel's xorshift RNG — confirm exact signatures against `midi_kernel::clock` from Plan A; adapt if `range` is `range_inclusive` or similar.)

- [ ] **Step 2: Property tests (the gate)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn position_stays_in_window(ls in 0usize..30, len in 1usize..34, dir_idx in 0u8..5) {
            let le = ls + len;
            let direction = [Direction::Up, Direction::Down, Direction::UpDown, Direction::Drunk, Direction::Random][dir_idx as usize];
            let mut rng = Rng::new(42);
            let mut pos = ls;
            let mut ds = 1i8;
            for _ in 0..1000 {
                pos = advance_lane(pos, &mut ds, direction, ls, le, &mut rng);
                prop_assert!(pos >= ls && pos < le, "pos {} out of [{},{})", pos, ls, le);
            }
        }

        #[test]
        fn palindrome_reverses_at_ends(ls in 0usize..20, len in 2usize..30) {
            let le = ls + len;
            let mut rng = Rng::new(7);
            let mut pos = ls;
            let mut ds = 1i8;
            let mut hits_top = false; let mut hits_bot = false;
            for _ in 0..10_000 {
                let prev = pos;
                pos = advance_lane(pos, &mut ds, Direction::UpDown, ls, le, &mut rng);
                if pos == le - 1 { hits_top = true; }
                if pos == ls { hits_bot = true; }
                prop_assert!(pos >= ls && pos < le);
                // never jumps more than 1
                prop_assert!(((pos as i32) - (prev as i32)).abs() <= 1);
            }
            prop_assert!(hits_top && hits_bot, "palindrome must reach both ends");
        }
    }
}
```

- [ ] **Step 3: Run + commit**

```bash
cd engine
cargo test -p stepforge_mono_engine dispatch
cargo clippy -p stepforge_mono_engine -- -D warnings
cd /Users/gus/Git/StepForge
git add engine/crates/mono_engine/src/dispatch.rs
git commit -m "feat(mono_engine): MonoRt + per-lane direction advance (proptested)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: `process()` — assemble note, scale snap, mono voice, ratchet

**Files:**
- Modify: `engine/crates/mono_engine/src/dispatch.rs` (add `process_pulse`).

**Interfaces:**
- Produces: `Engine::process_pulse(&mut self, rt, snap, ...)` that advances 6 lanes, assembles the note, snaps pitch, drives the mono voice + gate + ratchet, emits `Playhead`.

- [ ] **Step 1: Write process_pulse**

```rust
// in engine/crates/mono_engine/src/dispatch.rs (append)
use crate::models::{Gate, Lane, Pitch, Session, Sequence, Repeat};
use crate::scale::snap_to_scale;
use midi_kernel::midi::{build_note_on, humanize_velocity};
use midi_kernel::midi_out::{MidiMsg, MidiOutRing};

/// Lane indices into the Sequence's 6 lanes.
pub const PITCH: usize = 0; pub const VEL: usize = 1; pub const OCT: usize = 2;
pub const GATE: usize = 3; pub const REPEAT: usize = 4; pub const STEPEN: usize = 5;

pub fn process_pulse(
    rt: &mut MonoRt, seq: &Sequence, sess: &Session,
    midi: &mut MidiOutRing, pulse_samples: u64, gate_samples_base: u64,
    hot_events: &impl Fn(&[u8]),
) {
    // 1. advance each enabled lane (independent positions)
    advance_all(rt, seq);

    // 2. assemble from each lane's OWN position
    let interval = seq.pitch.values[rt.pos[PITCH]];
    let oct = seq.octave.values[rt.pos[OCT]];
    let mut vel = seq.velocity.values[rt.pos[VEL]];
    let gate_pct = seq.gate.values[rt.pos[GATE]];
    let reps = seq.repeat.values[rt.pos[REPEAT]];
    let on = seq.step_enable.values[rt.pos[STEPEN]];

    // 3. absolute pitch + scale snap
    let mut interval_abs = (interval as i32) + (oct as i32) * 12;
    if sess.conform_to_scale {
        interval_abs = snap_to_scale(interval_abs as i8, sess.root_key, sess.scale_mode) as i32
                       + (oct as i32) * 12;  // snap the within-octave interval, keep octave
    }
    let abs_note = (sess.base_note as i32 + sess.transpose as i32 + interval_abs).clamp(0, 127) as u8;
    vel = humanize_velocity(vel, 0.0, &mut rt.rng);  // humanize amount from session if desired

    // 4. mono voice: note-off previous sustaining note first
    //    (scheduled via kernel PendingMidiQueue in render_host; here we push note-on + off timing)
    let play_note = on && gate_pct > 0;
    if play_note {
        let reps_count = if reps == 0 { 1 } else { reps as u64 };
        let sub_samples = pulse_samples / reps_count;
        for r in 0..reps_count {
            let on_at = r * sub_samples;
            let off_at = on_at + (gate_samples_base * gate_pct as u64 / 100);
            push_note(midi, abs_note, vel, on_at, off_at);   // build_note_on via kernel
        }
    }
    rt.sounding = if play_note && gate_pct > 100 {
        Some(SoundingNote { note: abs_note, off_abs_sample: 0 /* set by render_host */ })
    } else { None };

    // 5. emit Playhead (atomic per-pulse)
    let mut buf = [0u8; 128];
    let ev = crate::event::EngineEvent::Playhead { positions: rt.pos };
    if let Some(n) = crate::event_codec::encode_event_into(&ev, &mut buf) {
        hot_events(&buf[..n]);
    }
    rt.pulse = rt.pulse.wrapping_add(1);
}

fn advance_all(rt: &mut MonoRt, seq: &Sequence) {
    // helper: iterate the 6 lanes with their loop windows + direction + dir_state.
    // (Unrolled or via a small macro — keep alloc-free.)
    unimplemented!("wire each lane: pitch/vel/oct/gate/repeat/step_enable")
}
```

(Fill `advance_all` by mapping each lane's `loop_start`/`loop_end`/`direction`/`enabled` to `advance_lane`. Gate-by-`enabled`: skip advancement for disabled lanes.)

- [ ] **Step 2: Test — rest (gate 0) suppresses note-on; disabled step skips**

```rust
#[test]
fn rest_gate_zero_no_note() { /* gate=0 → no midi note pushed */ }
#[test]
fn disabled_step_skips_note() { /* step_enable=false → no note */ }
```

- [ ] **Step 3: Run + commit**

```bash
cd engine
cargo test -p stepforge_mono_engine
cd /Users/gus/Git/StepForge
git add -A
git commit -m "feat(mono_engine): process_pulse — note assembly, scale snap, mono voice, ratchet

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: host-driven `Engine` + `render_host` (rate-derived), RT audit

**Files:**
- Create: `engine/crates/mono_engine/src/engine.rs`

**Interfaces:**
- Produces: `mono_engine::Engine` (host-driven), `Engine::render_host(&mut HostRenderState<MonoRt>, transport, midi_out, ...)`.

- [ ] **Step 1: Write the host-driven Engine + render_host (near-clone of drum engine.rs:529, rate-derived)**

```rust
// engine/crates/mono_engine/src/engine.rs
use crate::dispatch::{MonoRt, process_pulse};
use crate::models::Session;
use midi_kernel::host::{HostRenderState, HostTransport, MidiEvent, emit_midi_msg, PendingMidiQueue};
use midi_kernel::midi_out::MidiOutRing;

pub struct Engine {
    pub session: arc_swap::ArcSwap<Session>,
    pub midi: MidiOutRing,
    // hot_events + large_events channels + command queue (kernel types)
    // ...mirrors drum Engine field set, mono-typed...
}

impl Engine {
    pub fn new_host_driven() -> Self { /* construct with empty session */ }

    /// Host-driven render: fire one process_pulse per rate-boundary crossing
    /// the block. Rate-derived (NOT the drum's hardcoded 0.25).
    pub fn render_host(
        &self, rs: &mut HostRenderState<MonoRt>, transport: &HostTransport,
        midi_out: &mut [MidiEvent],
    ) -> usize {
        let snap = self.session.load();
        let block_end_beat = transport.block_start_beat + transport.block_samples as f64 / transport.sample_rate;
        let pulse_beats = snap.rate.pulse_beats();

        // play/stop transitions (mono: single NoteOff for sounding on stop)
        if transport.is_playing && !rs.was_playing { /* begin: reset rt, bar-align to rate grid */ }
        if !transport.is_playing && rs.was_playing {
            // CC123 is dropped by the mono midi port — emit one NoteOff for sounding
            if let Some(s) = &rs.rt.sounding { emit_noteoff(midi_out, s.note, 0); }
            rs.rt.sounding = None;
            rs.pending.clear();
        }
        rs.was_playing = transport.is_playing;

        // boundary loop — rate-derived increment (re-read each block for mid-playback SetRate)
        while transport.is_playing && rs.next_step_beat < block_end_beat {
            let pulse_samples = (pulse_beats * transport.sample_rate) as u64;
            let gate_samples = pulse_samples;
            process_pulse(&mut rs.rt, snap.active_sequence(), &snap, &mut self.midi.clone_ring(),
                          pulse_samples, gate_samples, &|b| self.hot_events.push(b));
            // drain midi ring → MidiEvent via kernel emit_midi_msg + PendingMidiQueue
            while let Some(msg) = self.midi.dequeue() {
                emit_midi_msg(&msg, rs, transport, midi_out);
            }
            rs.next_step_beat += pulse_beats;
        }
        // zero-crossing block: still drain due deferred note-offs
        rs.pending.drain_due(rs.sample_time, rs.sample_time + transport.block_samples as u64,
                             |ev| push_midi_event(midi_out, ev));
        rs.sample_time += transport.block_samples as u64;
        0
    }
}
```

(The exact `MidiOutRing` borrow + `emit_midi_msg` signature come from Plan A's midi_kernel — adapt the borrow to match drum's `&mut self.midi` pattern. The `hot_events.push(b)` for the encoded event mirrors drum's RT emit.)

- [ ] **Step 2: RT audit**

```bash
cd engine
# /audit-rt skill on mono_engine process_pulse + render_host.
# Manual grep:
grep -nE "Vec|String|format!|\.lock\(\)|\.read\(\)|\.write\(\)|Box::new|vec!\[" \
  engine/crates/mono_engine/src/dispatch.rs engine/crates/mono_engine/src/engine.rs
# Expected: zero hits in process_pulse / render_host bodies (test fixtures excluded).
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p stepforge_mono_engine
cargo clippy -p stepforge_mono_engine -- -D warnings
cd /Users/gus/Git/StepForge
git add engine/crates/mono_engine/src/engine.rs
git commit -m "feat(mono_engine): host-driven Engine + render_host (rate-derived pulse_beats)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 10: `mono_editor_egui` skeleton + `MonoUiState` + `apply`

**Files:**
- Create crate `engine/crates/mono_editor_egui/` (Cargo.toml, src/lib.rs, src/ui_state.rs)
- Modify: `engine/Cargo.toml`

**Interfaces:**
- Produces: `MonoUiState` (value mirror), `apply(EngineEvent)`, `CommandSink` trait.

- [ ] **Step 1: Cargo.toml + lib.rs**

```toml
# engine/crates/mono_editor_egui/Cargo.toml
[package]
name = "stepforge_mono_editor_egui"
# ...workspace fields...
[dependencies]
stepforge_mono_engine = { workspace = true }
egui = "0.27"  # match drum editor_egui's egui version exactly
[dev-dependencies]
# host-free test harness
```

```rust
// engine/crates/mono_editor_egui/src/lib.rs
pub mod ui_state;
pub use ui_state::MonoUiState;
pub trait CommandSink { fn submit(&mut self, cmd: stepforge_mono_engine::command::Command); }
pub fn render(_ctx: &egui::Context, _state: &mut MonoUiState, _sink: &mut dyn CommandSink) {}
```

- [ ] **Step 2: MonoUiState + apply**

```rust
// engine/crates/mono_editor_egui/src/ui_state.rs
use stepforge_mono_engine::event::EngineEvent;
use stepforge_mono_engine::models::Session;

#[derive(Clone, Debug, Default)]
pub struct MonoUiState {
    pub session: Session,
    pub playhead: [usize; stepforge_mono_engine::models::LANES],
    pub playing: bool,
}

impl MonoUiState {
    pub fn apply(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::PlayStateChanged(b) => self.playing = b,
            EngineEvent::Playhead { positions } => self.playhead = positions,
            EngineEvent::StepChanged { lane, idx, value } => { /* mutate session.active lane[idx] */ }
            EngineEvent::FullSnapshot(s) => self.session = s,
            // ...one arm per event variant...
            _ => {}
        }
    }
}
```

- [ ] **Step 3: Host-free test + commit**

```bash
cd engine
cargo test -p stepforge_mono_editor_egui
cd /Users/gus/Git/StepForge
git add engine/crates/mono_editor_egui engine/Cargo.toml
git commit -m "feat(mono_editor_egui): skeleton + MonoUiState mirror + apply(EngineEvent)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 11: Tabbed lane editor + loop ruler + transport

**Files:**
- Create: `engine/crates/mono_editor_egui/src/{transport.rs, lane_editor.rs, grid.rs}`

- [ ] **Step 1: Transport bar** (clone of `editor_egui/src/transport.rs`, mono controls: BPM/rate/swing/transpose/base/key/scale/conform/pattern selector+queue/play/stop).

- [ ] **Step 2: Lane tabs + step row** — 6-lane tab selector; vertical-drag cell editor per lane type; click toggles rest (gate 0) / step-enable off.

- [ ] **Step 3: Loop ruler** — per-lane loop_start/loop_end handles; global Steps; Dir; Reset dropdowns; Randomize/Shift/Init buttons.

- [ ] **Step 4: Wire `render()`** — compose transport + lane_editor + playhead overlay.

- [ ] **Step 5: Host-free egui tests + commit**

```bash
cd engine
cargo test -p stepforge_mono_editor_egui
cargo clippy -p stepforge_mono_editor_egui -- -D warnings
cd /Users/gus/Git/StepForge
git add engine/crates/mono_editor_egui/src
git commit -m "feat(mono_editor_egui): tabbed 6-lane editor + loop ruler + transport

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 12: `mono_clap` skeleton + struct + midi.rs + params (strip drum residue)

**Files:**
- Create crate `engine/crates/mono_clap/` (Cargo.toml, src/{lib.rs, midi.rs, params.rs, editor.rs})
- Modify: `engine/Cargo.toml`

- [ ] **Step 1: Cargo.toml** (mirror drum `clap_plugin/Cargo.toml`: nih-plug `f36931f`, nih-plug-egui, `assert_process_allocs` feature, deps on mono_engine + mono_editor_egui).

- [ ] **Step 2: params.rs** (`MonoParams` — `#[persist] editor_state: Arc<EguiState>`, `#[persist] session: Arc<RwLock<Vec<u8>>>`. Same shape as `clap_plugin/src/params.rs:9`).

- [ ] **Step 3: midi.rs** (clone of `clap_plugin/src/midi.rs:7` — `midi_event_to_note`, NoteOn/NoteOff only, velocity [0,1]).

- [ ] **Step 4: lib.rs struct** — strip drum residue per spec §5:

```rust
pub struct MonoSeq {
    engine: Arc<mono_engine::Engine>,
    host_render_state: midi_kernel::host::HostRenderState<mono_engine::dispatch::MonoRt>,
    sample_rate: f32,
    params: Arc<MonoParams>,
    worker_handle: Mutex<Option<JoinHandle<()>>>,
    ui_state: Arc<RwLock<mono_editor_egui::MonoUiState>>,  // mono UiState, not drum
    midi_buf: Box<[MidiEvent; 1024]>,
    was_playing: bool,
}
// CLAP_ID = "org.stepforge.mono"; CLAP_DESCRIPTION = "MIDI mono sequencer";
// CLAP_FEATURES = ["note-effect", "instrument"]; — NO demo_session (drum kick/snare seed).
```

- [ ] **Step 5: Commit**

```bash
cd engine
cargo check -p stepforge_mono_clap
cd /Users/gus/Git/StepForge
git add engine/crates/mono_clap engine/Cargo.toml
git commit -m "feat(mono_clap): skeleton + MonoSeq + params + midi.rs (drum residue stripped)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 13: nih-plug `Plugin` impl + worker + editor

**Files:**
- Modify: `engine/crates/mono_clap/src/lib.rs` (Plugin impl), `src/editor.rs`.

- [ ] **Step 1: Plugin impl** — mirror drum `clap_plugin/src/lib.rs:240/331/357`:
  - `initialize` — spawn worker via `ensure_worker()` (the PR-#12 re-activation guard at `clap_plugin/src/lib.rs:183-198`): set `shutdown=false` BEFORE the `is_none` check. Manual `std::thread::Builder::spawn(move || e.run_worker_loop())` — NOT nih-plug's `execute_background`.
  - `process()` — `context.transport()` → `map_transport` → `engine.render_host(...)` → drain `MidiEvent`s → `context.send_event(midi_event_to_note(ev))` per-event (NOT `send_note_events` — that API doesn't exist). Touch ONLY engine + host_render_state + midi_buf — never the Mutex/RwLock fields.
  - `reset()` (runs on RT thread) — `HostRenderState::<MonoRt>::new()` must be fixed-array-only (`assert_process_allocs` enforces).
  - `deactivate` — teardown worker.
  - `editor()` — open the mono egui editor; drain hot_events → decode → `ui_state.apply(ev)`.

- [ ] **Step 2: editor.rs** — decode `EngineEvent::Playhead{positions:[usize;6]}` → `ui_state.apply`; emit `Command`s via `CommandSink` (typed, by-value submit). NOT the drum `Playhead{track_idx,step_idx}`.

- [ ] **Step 3: RT-audit the process() body**

```bash
grep -nE "\.lock\(\)|\.read\(\)|\.write\(\)|Vec|String|format!" engine/crates/mono_clap/src/lib.rs
# Confirm ZERO hits inside the process() fn body.
```

- [ ] **Step 4: Build + commit**

```bash
cd engine
cargo clippy -p stepforge_mono_clap --all-targets -- -D warnings
cargo xtask bundle -p stepforge_mono_clap --release
cd /Users/gus/Git/StepForge
git add engine/crates/mono_clap/src
git commit -m "feat(mono_clap): nih-plug Plugin impl — process/render_host/send_event, worker, editor

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 14: Manual host verification

**Files:** none.

- [ ] **Step 1: Load the bundle in a host**

```bash
# Install the bundle where your host scans (Ableton Live / Bitwig), then:
# - instantiate StepForge Mono on a MIDI track feeding an instrument
# - play: verify notes out at the set rate, per-lane independent positions
# - toggle drunk/up-down/random directions: verify per-lane behavior
# - gate >100%: verify legato (sustains across pulses); gate 0: rest (no note)
# - repeat {2,3,4}: verify ratchet subdivisions within the pulse
# - conform-to-scale: verify out-of-scale pitches snap
# - pattern switch + queue: verify quantize-to-bar
# - transport stop: verify the sustaining note releases (no stuck note)
# - save/reload the project: verify session persistence round-trips
```

- [ ] **Step 2: Capture findings; fix + re-test; commit any fixes**

```bash
cd /Users/gus/Git/StepForge
git add -A && git commit -m "fix(mono_clap): host-test findings

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 15: Governing-doc amendments (mono scope)

**Files:**
- Modify: `CLAUDE.md` (Status, Workspace split, Where things live, "Two surfaces, one core", Commands)
- Modify: `docs/specs/architecture-spec.md` (§2 crate tree)
- Modify: `docs/specs/amendments.md` (E13–E15)

- [ ] **Step 1: Rewrite "Two surfaces, one core" → "shared midi_kernel + twin concrete engines"**

```markdown
**Two surfaces, two cores over a shared kernel.** The drum `core` (sequencer_engine)
and the mono `mono_engine` (stepforge_mono_engine) are twin concrete engines that
share drum-agnostic leaf infrastructure via `midi_kernel`. The CLAP plugins
(stepforge_clap, stepforge_mono_clap) wrap their respective engine in-process —
no Swift, no C ABI.
```

- [ ] **Step 2: Add mono crates to CLAUDE.md** — Status (mono edition live), Workspace split (name mono_engine/mono_editor_egui/mono_clap), Where things live, Commands (the mono cargo test/clippy/bundle commands).

- [ ] **Step 3: architecture-spec.md §2** — add the 4 mono crates (or note CLAUDE.md authoritative).

- [ ] **Step 4: amendments.md E13–E15**

```markdown
## E13 — mono_engine (2026-08-01)
M4L-parity melodic mono sequencer: 6 lanes, 5 directions (incl. drunk), per-lane
loop/direction/reset, mono voice, scale quantize. Twin concrete engine over midi_kernel.
## E14 — mono_editor_egui + mono_clap (2026-08-01)
Tabbed lane editor (pure-egui) + nih-plug CLAP wrapper (MIDI-out, persistence).
## E15 — doc reframe (2026-08-01)
"Two surfaces, one core" → "shared midi_kernel + twin concrete engines".
```

- [ ] **Step 5: Commit**

```bash
cd /Users/gus/Git/StepForge
git add CLAUDE.md docs/specs/architecture-spec.md docs/specs/amendments.md
git commit -m "docs: amend governing docs for mono surface (E13-E15)

Two surfaces → shared midi_kernel + twin concrete engines.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review (type/path consistency)

- **Lane count 6 throughout:** `LANES=6`, `pos:[usize;6]`, `dir_state:[i8;6]`, `Playhead{positions:[usize;6]}`, `MonoUiState.playhead:[usize;6]`. ✓
- **`Rate::pulse_beats()`** consumed by render_host boundary loop AND bar-align (rate-grid, not 16th). ✓
- **Mono voice + gate:** gate 0 → rest (no note); gate >100 → legato via `sounding`; CC123 dropped → single NoteOff on stop. ✓
- **Direction advance:** `dir_state` drives UpDown AND Drunk; proptest covers in-window + palindrome-end-reversal. ✓
- **In-process command path:** `Command` by value via `CommandSink` → typed `CommandQueue<Command>` (no command codec); `EngineEvent` encoded via `event_codec` on `HotEventChannel`. ✓
- **Persistence:** `VersionedEnvelope<Session>`, `MONO_SESSION_FORMAT_VERSION=1`, round-trip tested. ✓
- **clap residue stripped:** no demo_session, own CLAP_ID/description/features, mono UiState, editor decodes `Playhead{positions}`. ✓
- **nih-plug correctness:** `send_event` (not `send_note_events`), manual worker thread (not `execute_background`), `assert_process_allocs` on. ✓
- **RT-safety:** process/render_host/host_render_state::new all fixed-array; `/audit-rt` + grep gates. ✓

## Execution Handoff

Plan B depends on Plan A's `midi_kernel` (merged + green). After Plan B Task 15, the mono CLAP is feature-complete at v1 (model-parity + musical essentials). Deferred items (§12 of the spec: dotted/triplet rates, Max-pulse>1/32, MIDI-input routing, CC loop control, repeat-velocity-scaling, MIDI-thru, follow-actions) extend cleanly without rework.
