# midi_kernel Extraction Implementation Plan (Plan A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the drum-agnostic leaf infrastructure out of `sequencer_engine` (core) into a new `midi_kernel` crate, migrate drum core onto it, and prove the shipping drum app (Swift + FFI) is byte-compatible — zero behavior change.

**Architecture:** A new `#![forbid(unsafe_code)]` crate `stepforge_midi_kernel` holds `clock`, `host`, `midi_out`, `midi`, `scheduler`, `serde` leaf modules, with three genericizations (`HostRenderState<Rt>`, `CommandQueue<C>` + `LargeEventChannel<E>`, `VersionedEnvelope<T>`). Drum `core` keeps all serialized structs (`Session`/`Pattern`/`Track`/`Step`/`Command`/`EngineEvent`) field-for-field unchanged and re-exports the kernel modules so `ffi` source stays literally untouched. This is the prerequisite gate for the mono sequencer (Plan B).

**Tech Stack:** Rust 2021, workspace `engine/`, `heapless` (lock-free queues), `postcard` (serialization), `cbindgen` (C header), `arc-swap`. iOS cross-compile via rustup toolchain.

**Spec:** `docs/superpowers/specs/2026-08-01-mono-sequencer-engine-design.md` §1, §2, §9 (midi_kernel-scope amendments), §10 phases 0–1, §11.

## Global Constraints

- Drum serialized structs (`Session`/`Pattern`/`Track`/`Step`/`Command`/`EngineEvent`/`SessionEnvelope`) stay in `core`, field-for-field unchanged — postcard bytes MUST remain byte-identical (verified by the golden test, Task 1).
- `midi_kernel` is `#![forbid(unsafe_code)]`. All `unsafe` stays in `sequencer_engine_ffi` (sole `allow(unsafe_code)` crate).
- No drum-typed symbol (`Session`/`Track`/`Step`/`Command`/`EngineEvent`/`VelocityZone`/`Ratchet`) crosses into `midi_kernel`. `EngineEvent`/`Command` enter only as generic type parameters (`E`/`C`) instantiated by drum code in `core`.
- `engine/include/sequencer_engine.h` MUST regenerate byte-identical (cbindgen `include` whitelist updated).
- `ffi` crate source MUST stay literally unchanged (achieved via `core` re-exports).
- RT-safety (Hard Rule 1) and `forbid(unsafe_code)` (Hard Rule 6) preserved throughout.
- Run engine commands from `engine/`; run `./build_install_macos.sh` / xcodebuild from repo root.

---

### Task 1: Add the byte-equality safety-net tests FIRST

Before moving anything, capture the current drum contract bytes so the extraction cannot silently break them. These tests pass on the current code (baseline) and MUST keep passing through every subsequent task.

**Files:**
- Create: `engine/crates/core/tests/envelope_bytes_baseline.rs`
- Create: `engine/crates/core/tests/header_baseline.txt` (copy of current header)

**Interfaces:**
- Consumes: `sequencer_engine::serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION}`, `sequencer_engine::models::Session`
- Produces: a golden byte vector + header snapshot that gate the rest of the plan.

- [ ] **Step 1: Write the envelope byte-equality test**

```rust
// engine/crates/core/tests/envelope_bytes_baseline.rs
//! Golden byte-equality guard for the SessionEnvelope wire format.
//! Postcard is field-ORDER sensitive (tagless varint) — a field-order or
//! field-name flip in VersionedEnvelope<T> would silently change the bytes
//! AND the existing round-trip tests would still pass (they encode+decode
//! through the same struct). This test pins the exact bytes.
use sequencer_engine::models::Session;
use sequencer_engine::serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION};

#[test]
fn session_envelope_bytes_are_pinned() {
    // A Session with non-default fields so every field contributes bytes.
    let mut sess = Session::default();
    sess.bpm = 123.0;
    sess.global_swing_pct = 12.5;
    sess.active_pattern_index = 2;
    let env = SessionEnvelope::wrap(sess);
    assert_eq!(env.version, SESSION_FORMAT_VERSION);
    let bytes = postcard::to_allocvec(&env).expect("serialize");
    // Captured baseline: version byte (1) then the Session fields in declaration order.
    // If this assertion fires, the wire format changed — do NOT just update the literal;
    // that breaks every on-disk session saved by the shipping app.
    assert_eq!(
        bytes,
        vec![
            // postcard varint of SESSION_FORMAT_VERSION (1)
            0x01,
            // The remaining bytes encode the Session struct. They are reproduced here in
            // full because their stability is the entire point of this test.
            // === REPLACE THE LINE BELOW with the actual `vec![...]` captured in Step 2 ===
            0x00,
        ]
    );
}
```

- [ ] **Step 2: Capture the real baseline bytes (make the test pass honestly)**

Run a one-shot to print the actual bytes of `SessionEnvelope::wrap(sess)` for the `sess` above, then paste them into the `vec![...]`:

```bash
cd engine
cargo test -p sequencer_engine --test envelope_bytes_baseline -- --nocapture 2>&1 | head
# The assertion will FAIL on the placeholder 0x00. To capture the real bytes, add
# a temporary `println!("{:02x?}", bytes);` before the assert, run, copy the vec literal, paste it here.
```

Expected: test PASSES once the real byte vector is pasted. Commit only after it genuinely passes (not on the placeholder).

- [ ] **Step 3: Snapshot the cbindgen header**

```bash
cd /Users/gus/Git/StepForge
cp engine/include/sequencer_engine.h engine/crates/core/tests/header_baseline.txt
```

(The header is regenerated, not hand-maintained; this snapshot is the reference for the Task 10 byte-diff gate. It is test-fixture data, not committed source-of-truth — the real header at `engine/include/sequencer_engine.h` remains source-of-truth.)

- [ ] **Step 4: Run baseline + commit**

```bash
cd engine
cargo test -p sequencer_engine --test envelope_bytes_baseline
# Expected: PASS
cd /Users/gus/Git/StepForge
git add engine/crates/core/tests/envelope_bytes_baseline.rs engine/crates/core/tests/header_baseline.txt
git commit -m "$(cat <<'EOF'
test(core): pin SessionEnvelope wire bytes + header snapshot baseline

Safety-net for the midi_kernel extraction: postcard field-order flips would
pass the existing round-trip tests silently, so pin the exact bytes + the
cbindgen header before any code moves.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Create the `midi_kernel` crate skeleton

**Files:**
- Create: `engine/crates/midi_kernel/Cargo.toml`
- Create: `engine/crates/midi_kernel/src/lib.rs`
- Modify: `engine/Cargo.toml` (workspace members)
- Modify: `engine/Cargo.toml` (`[workspace.dependencies]`)

**Interfaces:**
- Produces: empty crate `stepforge_midi_kernel`, `#![forbid(unsafe_code)]`, added to workspace.

- [ ] **Step 1: Create the crate manifest**

```toml
# engine/crates/midi_kernel/Cargo.toml
[package]
name = "stepforge_midi_kernel"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
path = "src/lib.rs"

[dependencies]
serde = { workspace = true }
postcard = { workspace = true }
arc-swap = { workspace = true }
heapless = { workspace = true }
```

- [ ] **Step 2: Create lib.rs**

```rust
// engine/crates/midi_kernel/src/lib.rs
#![forbid(unsafe_code)]
//! stepforge_midi_kernel — drum-agnostic musical-time leaf infrastructure
//! shared by all StepForge sequencer engines (drum core, future mono engine).
//! Pure Rust: no FFI, no unsafe, no platform I/O, no sequencer model.
//! Sequencer-domain types (Session/Track/Step/Command/EngineEvent) never
//! appear here except as generic type parameters instantiated by consumers.

pub mod clock;
pub mod host;
pub mod midi;
pub mod midi_out;
pub mod scheduler;
pub mod serde_ext;
```

(The `pub mod` lines will not resolve until Tasks 3–8 create the files. To keep the workspace building incrementally, add each `pub mod` line in the task that creates that file. Start with only `pub mod clock;` here, or comment the rest until created.)

- [ ] **Step 3: Register in the workspace**

```toml
# engine/Cargo.toml — extend [workspace] members
members = ["crates/core", "crates/ffi", "crates/editor_egui", "crates/clap_plugin", "crates/xtask", "crates/midi_kernel"]

# add to [workspace.dependencies]:
stepforge_midi_kernel = { path = "crates/midi_kernel" }
```

- [ ] **Step 4: Verify + commit**

```bash
cd engine
cargo check -p stepforge_midi_kernel   # Expected: PASS (empty/commented mods)
cd /Users/gus/Git/StepForge
git add engine/crates/midi_kernel engine/Cargo.toml
git commit -m "feat(midi_kernel): scaffold stepforge_midi_kernel crate

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Move `clock.rs` (whole file)

Zero drum coupling — the cleanest move; do it first to validate the move pattern.

**Files:**
- Move: `engine/crates/core/src/clock.rs` → `engine/crates/midi_kernel/src/clock.rs`
- Modify: `engine/crates/core/src/lib.rs` (drop `pub mod clock;`, add re-export)
- Modify: `engine/crates/core/Cargo.toml` (add `stepforge_midi_kernel` dep)

**Interfaces:**
- Produces: `midi_kernel::clock::{Clock, SteppableClock, to_q16_16, advance_speed_ratio, effective_swing, swing_offset_micros, micro_timing_offset_micros, Rng}` (unchanged signatures).

- [ ] **Step 1: Move the file**

```bash
cd /Users/gus/Git/StepForge
git mv engine/crates/core/src/clock.rs engine/crates/midi_kernel/src/clock.rs
```

- [ ] **Step 2: Wire core onto the kernel**

```toml
# engine/crates/core/Cargo.toml — add dependency
[dependencies]
# ...existing deps...
stepforge_midi_kernel = { workspace = true }
```

```rust
// engine/crates/core/src/lib.rs — replace `pub mod clock;` with:
pub use midi_kernel::clock;
// (keep the other `pub mod` lines for now)
```

```rust
// engine/crates/midi_kernel/src/lib.rs — ensure `pub mod clock;` is uncommented
```

- [ ] **Step 3: Repoint core internal use-paths**

In `engine/crates/core/src/`, every `use crate::clock::{...}` becomes `use midi_kernel::clock::{...}` (or `use crate::clock::{...}` keeps working via the re-export — verify which the code uses; if it already goes through `crate::clock`, the re-export in Step 2 makes it resolve with no edit). Grep:

```bash
cd engine
grep -rn "crate::clock" crates/core/src/   # if hits, the re-export covers them; leave as-is
```

- [ ] **Step 4: Verify + commit**

```bash
cargo test -p sequencer_engine            # all clock + core tests PASS
cargo test -p sequencer_engine --test envelope_bytes_baseline   # PASS (unchanged)
cargo clippy -p sequencer_engine -p stepforge_midi_kernel -- -D warnings
cd /Users/gus/Git/StepForge
git add -A
git commit -m "refactor(midi_kernel): move clock.rs (whole-file, zero drum coupling)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Split `midi.rs` — leaf fns to kernel, drum residue stays

**Files:**
- Modify-and-split: `engine/crates/core/src/midi.rs` → keep drum residue; create `engine/crates/midi_kernel/src/midi.rs` with the leaf fns.
- Modify: both `lib.rs` files.

**Interfaces:**
- Produces (kernel): `midi_kernel::midi::{build_note_on, humanize_velocity, build_all_notes_off}`.
- Residue stays (core): `velocity_for_zone`, `ratchet_count` (reference `models::VelocityZone`/`Ratchet`).

- [ ] **Step 1: Create the kernel midi module with the leaf fns**

Copy `build_note_on` (`midi.rs:45-61`), `humanize_velocity` (`midi.rs:27-34`), `build_all_notes_off` (`midi.rs:62`) verbatim into `engine/crates/midi_kernel/src/midi.rs`. Update their imports: `Rng` comes from `crate::clock` (now in kernel); `MidiMsg` from `crate::midi_out` (moved in Task 5 — for now, if `build_note_on`/`build_all_notes_off` reference `MidiMsg`, either defer them to after Task 5 or forward-declare). Concretely:

```rust
// engine/crates/midi_kernel/src/midi.rs
//! Drum-agnostic MIDI leaf helpers. Sequencer-domain helpers
//! (velocity_for_zone, ratchet_count) stay in core — they reference drum models.
use crate::clock::Rng;

pub fn humanize_velocity(vel: u8, amount: f32, rng: &mut Rng) -> u8 {
    // ...exact body from core/src/midi.rs:27-34...
}

// build_note_on + build_all_notes_off reference MidiMsg (crate::midi_out).
// They move WITH Task 5 (midi_out). Leave them commented here until Task 5,
// OR place them in midi_kernel/src/midi.rs after Task 5 lands.
```

- [ ] **Step 2: Reduce core `midi.rs` to drum residue**

`engine/crates/core/src/midi.rs` keeps ONLY `velocity_for_zone` (`:18`) and `ratchet_count` (`:36`) + their `use crate::models::{VelocityZone, Ratchet}`. The leaf fns are removed (they now live in the kernel). Re-export the leaf fns back into core for any internal callers:

```rust
// engine/crates/core/src/lib.rs — add:
pub use midi_kernel::midi;
```

```rust
// engine/crates/core/src/midi.rs — reduced to drum residue:
use crate::models::{VelocityZone, Ratchet};
pub fn velocity_for_zone(/* ... */) -> u8 { /* exact body :18 */ }
pub fn ratchet_count(/* ... */) -> u8 { /* exact body :36 */ }
```

- [ ] **Step 3: Verify + commit**

```bash
cd engine
cargo test -p sequencer_engine
cargo test -p sequencer_engine --test envelope_bytes_baseline
cargo clippy -p sequencer_engine -p stepforge_midi_kernel -- -D warnings
cd /Users/gus/Git/StepForge
git add -A && git commit -m "refactor(midi_kernel): split midi.rs — leaf fns to kernel, drum residue stays

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Move `midi_out.rs` — genericize `CommandQueue<C>` + `LargeEventChannel<E>`

The load-bearing genericization: `EngineEvent` (drum-typed) must NOT cross into the kernel as a concrete type.

**Files:**
- Move: `engine/crates/core/src/midi_out.rs` → `engine/crates/midi_kernel/src/midi_out.rs`
- Modify: both `lib.rs`; `engine/crates/core/src/engine.rs` (instantiation sites).

**Interfaces:**
- Produces (kernel):
  - `midi_kernel::midi_out::{MidiMsg, MidiOutRing, HotEventChannel, LargeEventChannel, CommandQueue, push_drop_oldest, push_event, push_large_event, ...}`
  - `pub type CommandQueue<C> = Arc<MpMcQueue<C, COMMAND_DEPTH>>;`
  - `pub type LargeEventChannel<E> = Arc<MpMcQueue<E, LARGE_EVENT_DEPTH>>;`
  - `pub fn push_event<E>(channel: &LargeEventChannel<E>, ev: &E) where E: Clone` (signature genericized)
  - `pub fn push_large_event<E: Clone>(channel: &LargeEventChannel<E>, ev: E)`

- [ ] **Step 1: Move the file**

```bash
cd /Users/gus/Git/StepForge
git mv engine/crates/core/src/midi_out.rs engine/crates/midi_kernel/src/midi_out.rs
```

- [ ] **Step 2: Genericize the four drum-coupled items**

In `engine/crates/midi_kernel/src/midi_out.rs`:

```rust
// was: pub type CommandQueue = Arc<MpMcQueue<Command, COMMAND_DEPTH>>;
pub type CommandQueue<C> = Arc<MpMcQueue<C, COMMAND_DEPTH>>;

// was: pub type LargeEventChannel = Arc<MpMcQueue<EngineEvent, LARGE_EVENT_DEPTH>>;
pub type LargeEventChannel<E> = Arc<MpMcQueue<E, LARGE_EVENT_DEPTH>>;

// was: pub fn push_event(channel: &LargeEventChannel, ev: &EngineEvent) -> bool { ... }
pub fn push_event<E: Clone>(channel: &LargeEventChannel<E>, ev: &E) -> bool {
    // ...exact body — MpMcQueue::push is already generic over T...
}

// was: pub fn push_large_event(channel: &LargeEventChannel, ev: EngineEvent) -> bool { ... }
pub fn push_large_event<E: Clone>(channel: &LargeEventChannel<E>, ev: E) -> bool {
    // ...exact body...
}
```

Remove the `use crate::command::Command;` and `use crate::event::EngineEvent;` imports from the moved file — they no longer apply (the types are now parameters). Keep `HotEventChannel` and the small-event path as-is (their `[u8; MAX_EVENT_BYTES]` slots are already drum-agnostic).

- [ ] **Step 3: Repoint core use-paths + instantiate the generics**

In `engine/crates/core/src/`, the single instantiation site is `Engine`:

```rust
// engine/crates/core/src/engine.rs:252 (and the constructor)
// was: pub commands: CommandQueue,
pub commands: CommandQueue<crate::command::Command>,
// was: pub large_events: LargeEventChannel,
pub large_events: LargeEventChannel<crate::event::EngineEvent>,
```

Re-export in core `lib.rs`:

```rust
pub use midi_kernel::midi_out;
```

Any `use crate::midi_out::{...}` in core source keeps resolving via the re-export. The `command_queue()` constructor (`midi_out.rs:52`) becomes `pub fn command_queue<C>() -> CommandQueue<C>` (generic).

- [ ] **Step 4: Verify + commit**

```bash
cd engine
cargo test -p sequencer_engine
cargo test -p sequencer_engine --test envelope_bytes_baseline
cargo clippy -p sequencer_engine -p stepforge_midi_kernel -- -D warnings
cd /Users/gus/Git/StepForge
git add -A && git commit -m "refactor(midi_kernel): move midi_out, genericize CommandQueue<C> + LargeEventChannel<E>

EngineEvent (drum-typed) no longer crosses into the kernel as a concrete type.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Move `host.rs` + `emit_midi_msg` — genericize `HostRenderState<Rt>`

**Files:**
- Move: `engine/crates/core/src/host.rs` → `engine/crates/midi_kernel/src/host.rs`
- Modify: `engine/crates/core/src/engine.rs` (move `emit_midi_msg` fn `:1356` → kernel `host.rs`; turbofish the `HostRenderState::new()` call).
- Modify: `engine/crates/ffi/src/handle.rs:85` (turbofish).

**Interfaces:**
- Produces (kernel): `HostTransport`, `MidiEvent`, `PendingMidiQueue`, `emit_midi_msg`, `HostRenderState<Rt>` (generic over `Rt`).
- Drum supplies `Rt = RtState` (stays in core).

- [ ] **Step 1: Move host.rs + relocate emit_midi_msg**

```bash
cd /Users/gus/Git/StepForge
git mv engine/crates/core/src/host.rs engine/crates/midi_kernel/src/host.rs
```

Cut the free function `emit_midi_msg` (body at `engine.rs:1356-1411`) out of `engine.rs` and paste it into `engine/crates/midi_kernel/src/host.rs`. Its signature references only kernel types (`MidiMsg`, `PendingMidiQueue`, `MidiEvent`) — no edits needed beyond the move.

- [ ] **Step 2: Genericize HostRenderState**

In `engine/crates/midi_kernel/src/host.rs`:

```rust
// was:
//   use crate::engine::RtState;
//   pub struct HostRenderState { pub rt: RtState, ... }
// becomes:
pub struct HostRenderState<Rt> {
    pub rt: Rt,
    pub pending: PendingMidiQueue,
    pub next_step_beat: f64,
    pub sample_time: u64,
    pub last_block_start_beat: f64,
    pub was_playing: bool,
    pub initialized: bool,
}

impl<Rt> HostRenderState<Rt> {
    pub fn new(rt: Rt) -> Self {  // caller supplies rt (was: RtState::new(1))
        Self {
            rt,
            pending: PendingMidiQueue::new(),
            next_step_beat: 0.0,
            sample_time: 0,
            last_block_start_beat: f64::NAN,
            was_playing: false,
            initialized: false,
        }
    }
}
```

Drop the `use crate::engine::RtState;` import from the moved file. Update the `next_step_beat` field doc to drop "16th-step" wording (it is now rate-agnostic; mono will use it per-pulse — see Plan B).

- [ ] **Step 3: Repoint core + fix the two call sites**

```rust
// engine/crates/core/src/lib.rs — add:
pub use midi_kernel::host;
```

```rust
// engine/crates/core/src/engine.rs — wherever HostRenderState::new was called
// (e.g. begin_play / Engine construction). was: HostRenderState::new()
let rs: HostRenderState<RtState> = HostRenderState::new(RtState::new(seed));
// (RtState::new takes the seed that begin_play computes; preserve exact existing logic.)
```

```rust
// engine/crates/ffi/src/handle.rs:85 — was: Box::into_raw(Box::new(HostRenderState::new()))
Box::into_raw(Box::new(HostRenderState::<RtState>::new(RtState::new(/* prior default seed */))))
```

Confirm what seed `HostRenderState::new()` previously passed to `RtState::new` (host.rs:194 used `RtState::new(1)`) — preserve that exact value at both call sites.

- [ ] **Step 4: Verify + commit**

```bash
cd engine
cargo test -p sequencer_engine
cargo test -p sequencer_engine_ffi --test ffi_api
cargo test -p sequencer_engine --test envelope_bytes_baseline
cargo clippy -p sequencer_engine -p sequencer_engine_ffi -p stepforge_midi_kernel -- -D warnings
cd /Users/gus/Git/StepForge
git add -A && git commit -m "refactor(midi_kernel): move host.rs + emit_midi_msg, genericize HostRenderState<Rt>

Core supplies Rt=RtState; ffi handle.rs turbofished.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: Move `scheduler.rs` + co-extract `QuantizeGrain`

`SchedulerClock` depends on `QuantizeGrain`; both move. `QuantizeGrain` is pure transport grain (no drum semantics) but currently lives in drum `models.rs:167`. `all_notes_off_burst` stays (references drum `Session` + `MAX_TRACKS`).

**Files:**
- Move: `engine/crates/core/src/scheduler.rs` → `engine/crates/midi_kernel/src/scheduler.rs` (minus `all_notes_off_burst`)
- Modify: `engine/crates/core/src/models.rs` (remove `QuantizeGrain`)
- Modify: `engine/crates/midi_kernel/src/models.rs` (new — `QuantizeGrain` home)
- Modify: `engine/crates/core/src/{command.rs, event.rs, engine.rs}` — `QuantizeGrain` imports repoint (via core re-export).

**Interfaces:**
- Produces (kernel): `midi_kernel::scheduler::SchedulerClock`, `midi_kernel::models::QuantizeGrain`.
- Residue (core): `all_notes_off_burst`.

- [ ] **Step 1: Create kernel models module with QuantizeGrain**

```rust
// engine/crates/midi_kernel/src/models.rs
//! Kernel-level models: drum-agnostic musical-time primitives co-located here
//! because they have no sequencer-domain semantics.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum QuantizeGrain {
    #[default]
    NextStep,
    NextBeat,
    NextBar,
    EndOfPattern,
}
```

Add `pub mod models;` to `midi_kernel/src/lib.rs`.

- [ ] **Step 2: Move scheduler.rs, dropping all_notes_off_burst**

```bash
cd /Users/gus/Git/StepForge
git mv engine/crates/core/src/scheduler.rs engine/crates/midi_kernel/src/scheduler.rs
```

Edit `engine/crates/midi_kernel/src/scheduler.rs`:
- Change `use crate::models::QuantizeGrain;` → `use crate::models::QuantizeGrain;` (still resolves — now within kernel).
- Remove `all_notes_off_burst` (the `:131` fn) — cut it; it returns to core in Step 3.
- Remove `use crate::models::{Session, MAX_TRACKS};` (only `all_notes_off_burst` used them).

- [ ] **Step 3: Restore all_notes_off_burst + QuantizeGrain re-export in core**

`engine/crates/core/src/models.rs` — delete the `QuantizeGrain` enum (`:167-173`). Add a re-export so existing drum references resolve unchanged:

```rust
// engine/crates/core/src/models.rs (top, after imports):
pub use midi_kernel::models::QuantizeGrain;
```

```rust
// engine/crates/core/src/scheduler.rs — NEW small module holding the drum residue:
use crate::models::{Session, MAX_TRACKS};
use midi_kernel::midi_out::{MidiOutRing, build_all_notes_off};  // or wherever the leaf helper landed
use midi_kernel::scheduler::SchedulerClock;

pub fn all_notes_off_burst(/* exact signature :131 */) { /* exact body */ }
```

Add `pub mod scheduler;` back to core `lib.rs` (it now holds only the residue), plus `pub use midi_kernel::scheduler;` if downstream uses the module path — verify with grep which path core/engine.rs uses (`crate::scheduler::SchedulerClock` vs `crate::scheduler::all_notes_off_burst`). Provide both via:

```rust
// engine/crates/core/src/lib.rs:
pub mod scheduler;                 // holds all_notes_off_burst
pub use midi_kernel::scheduler;    // would COLLIDE — instead:
```

**Collision note:** `pub mod scheduler` and `pub use midi_kernel::scheduler` cannot both bind the name `scheduler`. Resolution: name the core residue module `pub mod scheduler_drum` (or keep `all_notes_off_burst` in `engine.rs` / a `drum_midi.rs` module). Pick: move `all_notes_off_burst` into `engine/crates/core/src/midi.rs` (the drum-residue midi module from Task 4) and drop `core/src/scheduler.rs` entirely. Then `pub use midi_kernel::scheduler;` in core `lib.rs` is unambiguous. Apply that.

```rust
// engine/crates/core/src/midi.rs — append the drum residue fn:
pub fn all_notes_off_burst(/* exact signature */) { /* exact body */ }
```

```rust
// engine/crates/core/src/lib.rs:
pub use midi_kernel::scheduler;   // SchedulerClock resolves here
// (no pub mod scheduler; in core anymore)
```

- [ ] **Step 4: Repoint QuantizeGrain references**

`command.rs` (`QueuePattern:71`, `RetriggerPattern:75`, `SetQuantizeGrain:90`), `event.rs` (`PatternQueued:37`), `engine.rs` (`:591, :1033, :1783, :1801, :1836`) — all reference `QuantizeGrain`. Since `core::models` now re-exports it (`pub use midi_kernel::models::QuantizeGrain`), these keep compiling unchanged. Verify:

```bash
cd engine
grep -rn "QuantizeGrain" crates/core/src/   # all should resolve via models re-export
```

- [ ] **Step 5: Verify + commit**

```bash
cargo test -p sequencer_engine
cargo test -p sequencer_engine_ffi --test ffi_api
cargo test -p sequencer_engine --test envelope_bytes_baseline
cargo clippy -p sequencer_engine -p sequencer_engine_ffi -p stepforge_midi_kernel -- -D warnings
cd /Users/gus/Git/StepForge
git add -A && git commit -m "refactor(midi_kernel): move SchedulerClock, co-extract QuantizeGrain

QuantizeGrain is pure transport grain (no drum semantics); all_notes_off_burst
stays in core (drum Session + MAX_TRACKS).

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: Move `serde_ext.rs` — genericize `VersionedEnvelope<T>` (the byte-trap task)

The highest-stakes move. Postcard is field-ORDER sensitive; the generic MUST keep `version: u8` as field 0 and name the payload `session`. The golden test from Task 1 catches any slip.

**Files:**
- Move: `engine/crates/core/src/serde_ext.rs` → `engine/crates/midi_kernel/src/serde_ext.rs`
- Modify: both `lib.rs`.

**Interfaces:**
- Produces (kernel): `VersionedEnvelope<T>`, `Versioned` trait.
- Core: `type SessionEnvelope = VersionedEnvelope<Session>`; `impl Versioned for Session`; keeps `SESSION_FORMAT_VERSION`.

- [ ] **Step 1: Create the generic VersionedEnvelope in the kernel**

```rust
// engine/crates/midi_kernel/src/serde_ext.rs
//! Generic versioned serialization envelope. The payload field MUST stay at
//! field index 1 and keep its serde name stable — postcard is field-ORDER
//! sensitive (tagless varint), so reordering or renaming flips the wire bytes.
//! Consumers pin the version via the `Versioned` trait.
use serde::{Deserialize, Serialize};

pub trait Versioned {
    const VERSION: u8;
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VersionedEnvelope<T> {
    pub version: u8,
    pub session: T,
}

impl<T: Versioned> VersionedEnvelope<T> {
    pub fn wrap(session: T) -> Self {
        Self { version: T::VERSION, session }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn envelope_roundtrips() {
        #[derive(Versioned // -- cannot derive; implement manually below)]
        // (Versioned has no derive macro — implement it in the test type)
        struct Dummy;
        impl Versioned for Dummy { const VERSION: u8 = 7; }
        // ...round-trip via postcard...
    }
}
```

(Clean up the test: `Versioned` is a manual trait, not derivable — write `impl Versioned for Dummy { const VERSION: u8 = 7; }` directly.)

- [ ] **Step 2: Reduce core serde_ext to the drum alias + version constant**

Delete `engine/crates/core/src/serde_ext.rs` (the move target). Recreate it as a thin shim:

```rust
// engine/crates/core/src/serde_ext.rs
//! Drum session persistence. On-disk format is postcard with a version tag.
//! The envelope type + pattern live in midi_kernel; core binds it to Session.
use crate::models::Session;
use midi_kernel::serde_ext::{Versioned, VersionedEnvelope};

/// Wire/disk format version for the drum Session. Bump + add a migration when
/// Session changes shape.
pub const SESSION_FORMAT_VERSION: u8 = 1;

impl Versioned for Session {
    const VERSION: u8 = SESSION_FORMAT_VERSION;
}

/// Drum session envelope. Byte-identical to the pre-extraction shape
/// `{ version: u8, session: Session }`.
pub type SessionEnvelope = VersionedEnvelope<Session>;
```

Add `pub use midi_kernel::serde_ext;` (or keep `pub mod serde_ext;` in core `lib.rs` since the shim file still exists). Verify which the `ffi/tests/ffi_api.rs` import expects: it uses `sequencer_engine::serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION}` — both still resolve from the shim. Keep `pub mod serde_ext;`.

- [ ] **Step 3: THE GATE — verify byte-identity**

```bash
cd engine
cargo test -p sequencer_engine --test envelope_bytes_baseline
```

Expected: PASS. If this FAILS, the generic's field order/name diverged from the original — fix `VersionedEnvelope<T>` (field 0 = `version`, payload field named `session`, no `#[serde(rename)]`) until it passes. Do NOT update the golden bytes.

- [ ] **Step 4: Verify the rest + commit**

```bash
cargo test -p sequencer_engine
cargo test -p sequencer_engine_ffi --test ffi_api
cargo clippy -p sequencer_engine -p sequencer_engine_ffi -p stepforge_midi_kernel -- -D warnings
cd /Users/gus/Git/StepForge
git add -A && git commit -m "refactor(midi_kernel): move serde_ext, genericize VersionedEnvelope<T>

Field order pinned (version: u8 field 0, payload named session) — golden byte
test confirms drum wire format unchanged. SESSION_FORMAT_VERSION stays in core.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: Confirm core re-exports make `ffi` source compile unchanged

The `ffi` crate imports `sequencer_engine::{clock, host, midi_out, midi, ...}` — these must all resolve via core re-exports with zero `ffi` source edits.

**Files:**
- Verify: `engine/crates/core/src/lib.rs` re-exports are complete.
- Verify: `engine/crates/ffi/src/{coremidi.rs:5, handle.rs:68, lib.rs:19/280/285}` and `engine/crates/ffi/tests/coremidi_host.rs:571/659` compile unchanged.

**Interfaces:**
- Produces: `sequencer_engine::{clock, host, midi_out, midi, scheduler, serde_ext}` all resolvable (re-exported from `midi_kernel`).

- [ ] **Step 1: Assemble the full re-export surface in core lib.rs**

```rust
// engine/crates/core/src/lib.rs (final shape):
#![forbid(unsafe_code)]
//! sequencer_engine — musical-time core for StepForge (drum).
//! Leaf infrastructure lives in stepforge_midi_kernel; this crate holds the
//! drum sequencer model + dispatch and re-exports the kernel modules so the
//! ffi crate's `sequencer_engine::<module>::*` paths resolve unchanged.

pub mod algorithms;
pub mod clipboard;
pub mod command;
pub mod engine;
pub mod event;
pub mod models;
pub mod undo;

// Leaf infrastructure re-exported from midi_kernel (drum residue fns for midi
// live in core/src/midi.rs; everything else is kernel-owned):
pub use midi_kernel::{clock, host, midi_out, scheduler, serde_ext};
pub mod midi;  // drum residue: velocity_for_zone, ratchet_count, all_notes_off_burst
```

(`midi` stays a core module because it holds drum residue; the leaf midi fns are reachable via `midi_kernel::midi` where core internals need them — verify core/src/engine.rs use of `build_note_on` etc. resolves. If engine.rs uses `crate::midi::build_note_on`, add `pub use midi_kernel::midi::*;` inside core `midi.rs`, or repoint those calls.)

- [ ] **Step 2: Verify ffi compiles with NO source edits**

```bash
cd engine
cargo check -p sequencer_engine_ffi
# Expected: PASS with zero edits to ffi/src/**
cargo test -p sequencer_engine_ffi                 # all ffi tests
cargo test -p sequencer_engine_ffi --test coremidi_host   # the test that imports moved symbols
```

If any `ffi` file fails to resolve a symbol, the missing re-export is the cause — add it to core `lib.rs`. Do NOT edit `ffi` source.

- [ ] **Step 3: Commit**

```bash
cd /Users/gus/Git/StepForge
git add -A && git commit -m "refactor(core): complete midi_kernel re-exports — ffi source unchanged

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 10: cbindgen `include` whitelist — header byte-identical

Moving `HostTransport`/`MidiEvent` (the `#[repr(C)]` structs) to `midi_kernel` breaks header generation unless cbindgen parses the new defining crate.

**Files:**
- Modify: `engine/cbindgen.toml`

**Interfaces:**
- Produces: regenerated `engine/include/sequencer_engine.h` byte-identical to `header_baseline.txt`.

- [ ] **Step 1: Add midi_kernel to the cbindgen include whitelist**

```toml
# engine/cbindgen.toml — [parse] section
[parse]
parse_deps = true
include = ["sequencer_engine", "stepforge_midi_kernel"]
```

- [ ] **Step 2: Regenerate + assert byte-identity**

```bash
cd /Users/gus/Git/StepForge
engine/scripts/build_engine.sh
git diff --exit-code -- engine/include/sequencer_engine.h && echo "HEADER BYTE-IDENTICAL"
# Also diff against the Task-1 snapshot for a second opinion:
diff engine/include/sequencer_engine.h engine/crates/core/tests/header_baseline.txt && echo "MATCHES BASELINE"
```

Expected: both report identical. If `git diff` shows changes, cbindgen is emitting `HostTransport`/`MidiEvent` as opaque — confirm `stepforge_midi_kernel` is spelled exactly as the package name in `engine/crates/midi_kernel/Cargo.toml` (`name = "stepforge_midi_kernel"`).

- [ ] **Step 3: Commit**

```bash
git add engine/cbindgen.toml
git commit -m "build(cbindgen): add stepforge_midi_kernel to include whitelist

HostTransport/MidiEvent moved to midi_kernel; without parsing that crate,
cbindgen emits them opaque. Header regenerated byte-identical.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 11: Governing-doc amendments (midi_kernel scope)

Plan A owns only the amendments the extraction changes. The "two surfaces, one core" rewrite stays true until mono_engine exists (Plan B).

**Files:**
- Modify: `CLAUDE.md` (Workspace split, Where things live, Hard Rule 6)
- Modify: `docs/specs/architecture-spec.md` (§1.3 Rule 6 mirror, §2 crate tree)
- Modify: `docs/specs/amendments.md` (E12 entry)

**Interfaces:**
- Produces: governing docs describe `midi_kernel` (no mono crates yet).

- [ ] **Step 1: Reword Hard Rule 6 to cover all musical-time crates**

In `CLAUDE.md` (Hard rules §6) and `docs/specs/architecture-spec.md` (§1.3):

```markdown
6. **Unsafe isolation** — all musical-time crates stay `#![forbid(unsafe_code)]`
   (`sequencer_engine`, `stepforge_midi_kernel`, and future engines like
   `stepforge_mono_engine`); all `unsafe` lives in `sequencer_engine_ffi`,
   reviewed line-by-line.
```

- [ ] **Step 2: Add midi_kernel to CLAUDE.md Workspace split + Where things live**

Update the "Workspace split" paragraph to name `midi_kernel` as the shared leaf (drum `core` depends on it; `ffi` still the only unsafe crate). Add `midi_kernel` to the "Where things live" crate list. Do NOT yet change "Two surfaces, one core" — it remains accurate (midi_kernel is leaf infra, not a core/surface).

- [ ] **Step 3: Update architecture-spec.md §2 crate tree**

Add `midi_kernel` (leaf infra, `#![forbid(unsafe_code)]`) to the §2 Crate Structure tree; note `core` now depends on it. (If §2 is badly stale already — single-crate model — add a pointer: "Authoritative crate topology: see CLAUDE.md > Architecture. This section is being superseded.")

- [ ] **Step 4: Add amendments.md E12**

```markdown
## E12 — midi_kernel extraction (2026-08-01)

Extracted drum-agnostic leaf infrastructure (clock, host, midi_out, midi,
scheduler, serde envelope) from `sequencer_engine` into a new
`#![forbid(unsafe_code)]` crate `stepforge_midi_kernel`. Drum `core` migrates
onto it via three genericizations (HostRenderState<Rt>, CommandQueue<C> +
LargeEventChannel<E>, VersionedEnvelope<T>) and re-exports the kernel modules
so the ffi crate source is unchanged. Drum postcard wire format byte-identical
(golden test). architecture-spec §1.1/§2 superseded by CLAUDE.md for crate
topology. Hard Rule 6 reworded to cover all musical-time crates. Design:
docs/superpowers/specs/2026-08-01-mono-sequencer-engine-design.md.
```

- [ ] **Step 5: Commit**

```bash
cd /Users/gus/Git/StepForge
git add CLAUDE.md docs/specs/architecture-spec.md docs/specs/amendments.md
git commit -m "docs: amend governing docs for midi_kernel extraction (E12)

Rule 6 covers all musical-time crates; workspace/where-things-live add
midi_kernel; architecture-spec §2 supersession noted. 'Two surfaces, one core'
unchanged — still true until mono_engine lands (Plan B).

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 12: Full Phase-1 gate verification

The merge gate. Every drum path green + iOS cross-compile + Swift app build + header byte-identity.

**Files:** none (verification only).

- [ ] **Step 1: Run the complete engine gate**

```bash
cd engine
cargo test                                              # ALL Rust tests
cargo test -p sequencer_engine_ffi --test ffi_api       # C-ABI roundtrip
cargo test -p sequencer_engine_ffi --test coremidi_host # moved-symbol imports
cargo test -p sequencer_engine --test envelope_bytes_baseline  # byte-identity
cargo clippy --workspace --all-targets -- -D warnings   # whole workspace lint
cargo fmt --check
```

- [ ] **Step 2: iOS cross-compile (rustup toolchain, not Homebrew)**

```bash
export PATH="$HOME/.cargo/bin:$PATH"   # avoid Homebrew rust shadow
cd engine
cargo check --target aarch64-apple-ios
# Expected: PASS
```

- [ ] **Step 3: cbindgen header byte-identity (re-confirm after all changes)**

```bash
cd /Users/gus/Git/StepForge
engine/scripts/build_engine.sh
git diff --exit-code -- engine/include/sequencer_engine.h && echo "HEADER CLEAN"
```

- [ ] **Step 4: Swift app builds (drum app untouched)**

```bash
cd /Users/gus/Git/StepForge
./build_install_macos.sh
# AND/OR:
cd app && xcodegen generate
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge \
  -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build
```

Expected: both succeed; the drum app behaves identically.

- [ ] **Step 5: Final commit (if any verification surfaced fixes) + summary**

```bash
cd /Users/gus/Git/StepForge
git status   # clean working tree expected
git log --oneline docs/mono-sequencer-engine-design..HEAD   # the extraction commits
```

---

## Self-Review (type/path consistency)

- **Generic instantiations:** `CommandQueue<Command>` (engine.rs:252), `LargeEventChannel<EngineEvent>` (engine.rs), `HostRenderState<RtState>` (engine.rs + ffi/handle.rs:85), `VersionedEnvelope<Session>` (core serde_ext shim). All four generics defined in midi_kernel; all four instantiated in core/ffi. ✓
- **Re-export surface:** core `lib.rs` re-exports `clock`, `host`, `midi_out`, `scheduler`, `serde_ext` from midi_kernel; keeps `midi` (drum residue). ffi imports all resolve. ✓
- **Byte-identity guards:** Task 1 golden test (SessionEnvelope bytes) + Task 10 header diff. The two highest-stakes invariants are independently pinned. ✓
- **QuantizeGrain path:** defined in `midi_kernel::models`; re-exported from `core::models`; command.rs/event.rs/engine.rs resolve unchanged. ✓
- **`all_notes_off_burst` home:** moved to core `midi.rs` (drum residue) to avoid the `pub mod scheduler` / `pub use midi_kernel::scheduler` name collision. ✓
- **emit_midi_msg:** relocated from engine.rs:1356 → midi_kernel::host (references only kernel types). ✓
- **Hard Rule 1 / 6:** preserved — midi_kernel is `#![forbid(unsafe_code)]`; no RT-path change (pure move). ✓

## Execution Handoff

Plan A is the prerequisite gate for Plan B (`2026-08-01-mono-sequencer.md`). Do not start Plan B until Task 12 is fully green. After Plan A merges, Plan B authors the mono engine/editor/clap on top of the now-stable `midi_kernel`.
