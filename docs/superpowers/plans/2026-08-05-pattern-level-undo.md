# Pattern-Level Undo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make whole-pattern `CutPattern` / `PastePattern` / `ClearPattern` undoable — one undo per pattern slot — across the Rust core, the CLAP egui editor, and the iOS app (closes #34).

**Architecture:** Extend the existing per-track `Undo` struct with a `pattern_slots: [Option<PatternSnapshot>; PATTERN_SLOTS]` array (mirrors how `Clipboard` already unifies track + pattern). Push a `PatternSnapshot` (tracks + follow_action, no `id`) before each mutating pattern op; a new `Command::UndoPattern { index }` restores it. Always-on UI (no new `EngineEvent`); restore publishes `FullSnapshot` on the existing path. `LoadSession` drains pattern slots (#30 parity). Hardening: a pattern op on the active pattern clears the now-stale per-track undo slots.

**Tech Stack:** Rust (`sequencer_engine` `#![forbid(unsafe_code)]`, `sequencer_engine_ffi`), `proptest`, `postcard`, egui 0.31.1, SwiftUI, Xcode/xcodebuild.

## Global Constraints

- `#![forbid(unsafe_code)]` in `sequencer_engine` preserved; no `unsafe` touched. All undo/clipboard lock sites stay on the worker thread (`apply_command`), never RT.
- No new `EngineEvent`; no `SESSION_FORMAT_VERSION` bump (undo stays in-memory engine state). The C header (`engine/include/sequencer_engine.h`) stays byte-identical — `Command` variants cross as postcard bytes, not as header entries.
- `Command::UndoPattern { index }` is **appended** to the enum (postcard tag 39) — never inserted, never reshaped (wire-format + golden-fixture stability).
- Cross-layer symmetry, no orphans: every Command variant gets Rust + codec + C-ABI round-trip, then Swift mirror + encoder + golden, plus the editor button.
- Run cargo commands from `engine/`; run `git`/`gh`/`xcodebuild` from the repo root. Branch is `feat/pattern-level-undo` (off `origin/main`). Re-verify `HEAD` ancestry against `origin/main` before each dispatch.
- iOS guard: `cargo check -p sequencer_engine --target aarch64-apple-ios` with `$HOME/.cargo/bin` on `PATH` (rustup toolchain, NOT Homebrew `rustc`).
- Spec: `docs/superpowers/specs/2026-08-05-pattern-level-undo-design.md`.

---

## Task 1: `PatternSnapshot` + extend `Undo` (core)

**Files:**
- Modify: `engine/crates/core/src/undo.rs` (imports, `PatternSnapshot`, `Undo` field + Default, five new methods)
- Test: `engine/crates/core/src/undo.rs` (new unit tests + a proptest in the `#[cfg(test)] mod tests` block)

**Interfaces:**
- Consumes: `crate::models::{Session, Step, Track, MAX_TRACKS, STEP_COUNT}` (already imported); adds `FollowAction, Pattern, PATTERN_SLOTS`.
- Produces: `PatternSnapshot { tracks: Vec<Track>, follow_action: FollowAction }` with `PatternSnapshot::of(&Pattern) -> Self`; `Undo::push_pattern(&mut self, index: usize, p: &Pattern)`; `Undo::undo_pattern(&mut self, s: &mut Session, index: usize) -> bool`; `Undo::available_pattern(&self, index: usize) -> bool`; `Undo::take_occupied_patterns(&mut self) -> [bool; PATTERN_SLOTS]`; `Undo::clear_tracks(&mut self)`. Task 3 consumes all of these.

- [ ] **Step 1: Write the failing unit tests + proptest**

In `engine/crates/core/src/undo.rs`, add to the existing `#[cfg(test)] mod tests` block (the block already exists with `take_occupied_marks_indices_and_clears` etc.):

```rust
    use crate::clipboard::Clipboard;
    use crate::models::{Pattern, PATTERN_SLOTS, VelocityZone};

    fn session_with_pattern_at(idx: usize) -> Session {
        // Session::default leaves pattern slots None; place a Pattern with one
        // active step + a non-default follow_action so a snapshot is observable.
        let mut s = Session::default();
        let mut p = Pattern::default();
        p.tracks[0].steps[3] = crate::models::Step {
            active: true,
            velocity_zone: VelocityZone::Accent,
            ..Default::default()
        };
        p.follow_action = crate::models::FollowAction {
            after_loops: 5,
            action: crate::models::FollowActionType::PlayNext,
        };
        s.patterns[idx] = Some(p);
        s
    }

    #[test]
    fn push_then_undo_pattern_restores_tracks_and_follow_action() {
        // #34: a pre-mutation PatternSnapshot restores tracks + follow_action,
        // preserves the slot's id, and keeps the slot Some (one-deep).
        let mut s = session_with_pattern_at(2);
        let id_before = s.patterns[2].as_ref().unwrap().id;
        let tracks_before = s.patterns[2].as_ref().unwrap().tracks.clone();
        let fa_before = s.patterns[2].as_ref().unwrap().follow_action.clone();

        let mut u = Undo::default();
        u.push_pattern(2, s.patterns[2].as_ref().unwrap());
        assert!(u.available_pattern(2), "snapshot present after push");

        // Mutate (clear), then undo.
        Clipboard::clear_pattern(&mut s, 2);
        assert!(u.undo_pattern(&mut s, 2), "undo must restore");

        let got = s.patterns[2].as_ref().expect("slot stays Some");
        assert_eq!(got.tracks, tracks_before, "tracks restored");
        assert_eq!(got.follow_action, fa_before, "follow_action restored");
        assert_eq!(got.id, id_before, "id preserved");
        assert!(!u.available_pattern(2), "snapshot consumed (one-deep)");
        // A second undo is a no-op.
        assert!(!u.undo_pattern(&mut s, 2), "second undo is a no-op");
    }

    #[test]
    fn undo_pattern_is_total_for_bad_index_or_empty_slot() {
        // OOB index, missing snapshot, and a None slot must all return false,
        // never panic.
        let mut u = Undo::default();
        let mut s = Session::default();
        assert!(!u.undo_pattern(&mut s, PATTERN_SLOTS), "OOB index is a no-op");
        assert!(!u.undo_pattern(&mut s, 0), "missing snapshot is a no-op");
        // Slot 0 is None in a default Session → no-op even with a snapshot push
        // at a different index.
        u.push_pattern(1, &Pattern::default()); // push needs a real Pattern
        assert!(!u.undo_pattern(&mut s, 1), "None target slot is a no-op");
    }

    #[test]
    fn take_occupied_patterns_drains_and_clears() {
        // LoadSession parity (#30): drain all occupied pattern slots, marking
        // which indices were Some (now cleared). Stack `[bool; PATTERN_SLOTS]`.
        let mut u = Undo::default();
        u.push_pattern(1, &Pattern::default());
        u.push_pattern(7, &Pattern::default());
        let occupied = u.take_occupied_patterns();
        assert!(occupied[1] && occupied[7], "marked indices 1 and 7");
        assert!(!u.available_pattern(1) && !u.available_pattern(7), "cleared");
        // Idempotent.
        assert!(
            u.take_occupied_patterns().iter().all(|f| !f),
            "second drain is empty"
        );
    }

    #[test]
    fn clear_tracks_empties_per_track_slots() {
        // D6 helper: a whole-pattern change on the active pattern clears the
        // (now stale) per-track snapshots.
        let mut u = Undo::default();
        let t = crate::models::Track::default();
        u.push(0, &t);
        u.push(3, &t);
        assert!(u.available(0) && u.available(3));
        u.clear_tracks();
        assert!(!u.available(0) && !u.available(3), "per-track slots cleared");
    }

    proptest::proptest! {
        /// #34 invariant (working agreement: algorithm changes get property
        /// tests). After push_pattern → mutate → undo_pattern, the pattern's
        /// tracks + follow_action are restored and the id is unchanged.
        #[test]
        fn prop_pattern_undo_restores_state(
            n_active in 0usize..16,
            after_loops in 1u32..=16,
        ) {
            let mut s = session_with_pattern_at(4);
            {
                let p = s.patterns[4].as_mut().unwrap();
                for st in p.tracks[0].steps.iter_mut().take(n_active) {
                    st.active = true;
                }
                p.follow_action.after_loops = after_loops;
            }
            let before = s.patterns[4].clone().unwrap();

            let mut u = Undo::default();
            u.push_pattern(4, &before);
            Clipboard::clear_pattern(&mut s, 4);
            prop_assert!(u.undo_pattern(&mut s, 4));

            let got = s.patterns[4].as_ref().unwrap();
            prop_assert_eq!(got.tracks, before.tracks);
            prop_assert_eq!(got.follow_action, before.follow_action);
            prop_assert_eq!(got.id, before.id, "id invariant");
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run (from `engine/`):
```bash
cargo test -p sequencer_engine push_then_undo_pattern_restores
cargo test -p sequencer_engine prop_pattern_undo_restores_state
```
Expected: compile error — `PatternSnapshot`, `push_pattern`, `undo_pattern`, `available_pattern`, `take_occupied_patterns`, `clear_tracks` not defined.

- [ ] **Step 3: Add the imports + `PatternSnapshot`**

In `engine/crates/core/src/undo.rs`, change the imports line (line 10):

```rust
use crate::models::{FollowAction, Pattern, Session, Step, Track, MAX_TRACKS, PATTERN_SLOTS, STEP_COUNT};
```

Then, immediately after the `TrackSnapshot` impl (after line 28, before `pub struct Undo`), add:

```rust
/// What one pattern-level undo snapshot captures (#34): the full `tracks` plus
/// the `follow_action`. `id` is excluded — every mutating pattern op preserves
/// the target pattern's `id` (mirrors `PatternClipboard`), so restore overwrites
/// only `tracks` + `follow_action`. Heap-backed (`Vec<Track>`) → worker-thread
/// only, never RT. In-memory only; never serialized (no `SESSION_FORMAT_VERSION`
/// bump).
#[derive(Clone)]
pub struct PatternSnapshot {
    pub tracks: Vec<Track>,
    pub follow_action: FollowAction,
}

impl PatternSnapshot {
    pub fn of(p: &Pattern) -> Self {
        Self {
            tracks: p.tracks.clone(),
            follow_action: p.follow_action.clone(),
        }
    }
}
```

- [ ] **Step 4: Extend `Undo` with `pattern_slots` + Default**

Replace the `Undo` struct + `Default` (lines 30-39):

```rust
pub struct Undo {
    slots: [Option<TrackSnapshot>; MAX_TRACKS],
    pattern_slots: [Option<PatternSnapshot>; PATTERN_SLOTS],
}
impl Default for Undo {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            pattern_slots: std::array::from_fn(|_| None),
        }
    }
}
```

- [ ] **Step 5: Add the five new methods to `impl Undo`**

Add to `impl Undo` (after `take_occupied`, before the closing `}` of the impl block — line 91):

```rust
    /// Snapshot a whole pattern (tracks + follow_action) before a pattern-level
    /// mutation (#34). One-deep: overwrites any prior snapshot for `index`.
    /// Bounds-checked; OOB is a no-op. Mirrors the per-track `push`.
    pub fn push_pattern(&mut self, index: usize, p: &Pattern) {
        if index < PATTERN_SLOTS {
            self.pattern_slots[index] = Some(PatternSnapshot::of(p));
        }
    }

    /// Restore slot `index`'s tracks + follow_action if a snapshot exists
    /// (one-deep: the snapshot is consumed). Leaves the pattern's `id` and the
    /// slot's `Some`-ness untouched. Total: OOB index, missing snapshot, or a
    /// `None` slot → returns false, no panic.
    pub fn undo_pattern(&mut self, s: &mut Session, index: usize) -> bool {
        if index >= PATTERN_SLOTS {
            return false;
        }
        // Resolve the target slot first so a `None` slot never consumes a
        // snapshot (doesn't arise today — pattern ops keep slots Some — but
        // stay total).
        let Some(p) = s
            .patterns
            .get_mut(index)
            .and_then(|opt| opt.as_mut())
        else {
            return false;
        };
        let Some(snap) = self.pattern_slots[index].take() else {
            return false;
        };
        p.tracks = snap.tracks;
        p.follow_action = snap.follow_action;
        true
    }

    /// Whether slot `index` has a pattern snapshot. (Tests only — the UI is
    /// always-on, so this is not read by any surface.)
    pub fn available_pattern(&self, index: usize) -> bool {
        index < PATTERN_SLOTS && self.pattern_slots[index].is_some()
    }

    /// Drain all occupied pattern slots, marking which indices were `Some` (now
    /// cleared). Used by `LoadSession` to reset pattern undo on a wholesale
    /// reload (#30 parity with `take_occupied`). Stack `[bool; PATTERN_SLOTS]`.
    pub fn take_occupied_patterns(&mut self) -> [bool; PATTERN_SLOTS] {
        let mut out = [false; PATTERN_SLOTS];
        for (i, slot) in self.pattern_slots.iter_mut().enumerate() {
            if slot.is_some() {
                out[i] = true;
                *slot = None;
            }
        }
        out
    }

    /// Clear every per-track slot. Used when a whole-pattern change targets the
    /// active pattern: the per-track snapshots are now stale w.r.t. that pattern
    /// (D6).
    pub fn clear_tracks(&mut self) {
        for slot in self.slots.iter_mut() {
            *slot = None;
        }
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run (from `engine/`):
```bash
cargo test -p sequencer_engine undo
```
Expected: all PASS (the four new unit tests + the proptest + the existing `take_occupied` / `push` tests).

- [ ] **Step 7: clippy + iOS guard + header diff**

Run (from `engine/`, with `$HOME/.cargo/bin` on `PATH`):
```bash
cargo clippy -p sequencer_engine --all-targets -- -D warnings
cargo check -p sequencer_engine --target aarch64-apple-ios
```
Then (from repo root):
```bash
git diff --exit-code -- engine/include/sequencer_engine.h
```
Expected: no warnings; iOS check succeeds; header byte-identical (exit 0).

- [ ] **Step 8: Commit**

```bash
git add engine/crates/core/src/undo.rs
git commit -m "$(cat <<'EOF'
feat(engine): PatternSnapshot + extend Undo for pattern undo (#34)

Add a one-deep per-slot pattern undo store mirroring the per-track Undo:
PatternSnapshot { tracks, follow_action } (no id — preserved by every
mutating pattern op, mirroring PatternClipboard). Undo gains pattern_slots
+ push_pattern/undo_pattern/available_pattern/take_occupied_patterns/
clear_tracks. Worker-thread only; in-memory only; no serde bump.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `Command::UndoPattern` + codec + C-ABI round-trip (contract)

**Files:**
- Modify: `engine/crates/core/src/command.rs` (append variant + add to the `commands_roundtrip` test)
- Modify: `engine/crates/core/src/engine.rs` (add a temporary no-op arm so the new variant compiles into the exhaustive `apply_command` match; replaced by Task 3)
- Modify: `engine/crates/ffi/src/command_codec.rs` (add to the `commands_roundtrip` test array)
- Modify: `engine/crates/ffi/tests/ffi_api.rs` (new C-ABI round-trip test)

**Interfaces:**
- Produces: `Command::UndoPattern { index: usize }` (postcard tag 39). Task 3 wires its engine behavior; Task 4 the editor button; Task 5 the Swift encoder.

- [ ] **Step 1: Append the variant**

In `engine/crates/core/src/command.rs`, append after `ClearPattern { index: usize, }` (the last variant, before the closing `}` of the enum — line 144):

```rust
    /// Undo the last whole-pattern op (Cut/Paste/Clear) on slot `index`.
    /// One-deep; no-op if no snapshot. Always-on in the UI (no availability
    /// event). Restore publishes a FullSnapshot. In-memory snapshot, never
    /// serialized (#34).
    UndoPattern {
        index: usize,
    },
```

- [ ] **Step 2: Add a temporary no-op engine arm**

In `engine/crates/core/src/engine.rs`, immediately after the `CutPattern | PastePattern | ClearPattern` arm's closing `}` (line 1088) and before the `QueuePattern` arm, add:

```rust
            // #34: pattern-level undo. Temporary no-op — the real restore logic
            // lands in Task 3. Kept explicit so the variant is not silently
            // swallowed by the `other` catch-all and so the match stays
            // exhaustive as the variant is added.
            UndoPattern { .. } => {}
```

- [ ] **Step 3: Add the variant to the core round-trip test**

In `engine/crates/core/src/command.rs`, in the `commands_roundtrip` test array (after `Command::ClearPattern { index: 5 },` — line 167), add:

```rust
            Command::UndoPattern { index: 6 },
```

- [ ] **Step 4: Add the variant to the ffi codec round-trip test**

In `engine/crates/ffi/src/command_codec.rs`, in the `commands_roundtrip` test array (after `Command::ClearPattern { index: 5 },` — line 38), add:

```rust
            Command::UndoPattern { index: 6 },
```

- [ ] **Step 5: Add the C-ABI round-trip test**

In `engine/crates/ffi/tests/ffi_api.rs`, immediately after the `pattern_clipboard_commands_are_accepted_over_c_abi` test (after its closing `}` — line 63), add:

```rust
#[test]
fn undo_pattern_command_roundtrips_over_c_abi() {
    // #34: Command::UndoPattern round-trips the postcard codec + the C-ABI
    // submit path — accepted (Ok), never a fatal decode error (Hard Rule 3).
    use sequencer_engine::command::Command;
    let h = sequencer_engine_ffi::engine_new();
    let bytes = command_codec::encode_command(&Command::UndoPattern { index: 4 }).unwrap();
    let res =
        unsafe { sequencer_engine_ffi::engine_submit_command(h, bytes.as_ptr(), bytes.len()) };
    assert_eq!(res, EngineResult::Ok, "UndoPattern must be accepted over the C ABI");
    unsafe { sequencer_engine_ffi::engine_free(h) };
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run (from `engine/`):
```bash
cargo test -p sequencer_engine commands_roundtrip
cargo test -p sequencer_engine_ffi --test ffi_api undo_pattern_command_roundtrips
```
Expected: both PASS (the variant serializes, round-trips, and is accepted across the C ABI; the no-op arm keeps the engine compiling).

- [ ] **Step 7: Full core + ffi suites, clippy, iOS guard, header diff**

Run (from `engine/`, with `$HOME/.cargo/bin` on `PATH`):
```bash
cargo test -p sequencer_engine
cargo test -p sequencer_engine_ffi
cargo clippy -p sequencer_engine --all-targets -- -D warnings
cargo clippy -p sequencer_engine_ffi --all-targets -- -D warnings
cargo check -p sequencer_engine --target aarch64-apple-ios
```
Then (from repo root):
```bash
git diff --exit-code -- engine/include/sequencer_engine.h
```
Expected: all PASS / no warnings / iOS check succeeds / header byte-identical.

- [ ] **Step 8: Commit**

```bash
git add engine/crates/core/src/command.rs engine/crates/core/src/engine.rs \
        engine/crates/ffi/src/command_codec.rs engine/crates/ffi/tests/ffi_api.rs
git commit -m "$(cat <<'EOF'
feat(engine): Command::UndoPattern variant + codec round-trip (#34)

Append Command::UndoPattern { index } (postcard tag 39) + a temporary
no-op engine arm. Round-trips the postcard codec and the C-ABI submit
path. Wire-format only — no header change (commands cross as bytes), no
serde bump. Restore behavior lands in the next commit.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Engine behavior — snapshot, restore, LoadSession drain, D6

**Files:**
- Modify: `engine/crates/core/src/engine.rs`:
  - the `CutPattern | PastePattern | ClearPattern` arm (~lines 1054-1088) — push before mutation + D6 clear
  - the temporary `UndoPattern { .. } => {}` arm (Task 2) — replace with the real restore arm
  - the `LoadSession` success branch (~lines 920-938) — drain pattern slots
- Test: `engine/crates/core/src/engine.rs` (new tests in the `#[cfg(test)] mod tests` block)

**Interfaces:**
- Consumes (from Task 1): `Undo::push_pattern`, `Undo::undo_pattern`, `Undo::take_occupied_patterns`, `Undo::clear_tracks`, `PatternSnapshot`.
- Produces: the observable contract — `CutPattern`/`PastePattern`/`ClearPattern` push a one-deep pattern snapshot before mutating; `UndoPattern { index }` restores it (publishes `FullSnapshot`), preserves `id`, keeps the slot `Some`; `CopyPattern` pushes nothing; `LoadSession` clears all pattern snapshots; a pattern op on the active pattern clears the per-track undo.

- [ ] **Step 1: Write the failing engine tests**

Add to `engine/crates/core/src/engine.rs` in the `#[cfg(test)] mod tests` block (near the other `load_session_*` / pattern tests). These use the existing `Engine::new()`, `apply_command`, `snapshot_arc()`, `undo.lock()`, `large_events.dequeue()` accessors already used by the `load_session_clears_stale_undo_and_emits_unavailable` test:

```rust
    /// #34: ClearPattern pushes a pattern-undo snapshot; UndoPattern restores
    /// the prior pattern (tracks + follow_action + id) and publishes a
    /// FullSnapshot.
    #[test]
    fn pattern_undo_restores_after_clear_and_publishes_snapshot() {
        use crate::models::VelocityZone;
        let e = Engine::new();
        // Seed an active step on the active pattern (slot 0).
        e.apply_command(Command::SetStep {
            track_idx: 0,
            step_idx: 0,
            zone: VelocityZone::Accent,
        });
        let before = e.snapshot_arc().patterns[0].clone().unwrap();
        assert!(before.tracks[0].steps[0].active);

        // Clear pushes a snapshot.
        e.apply_command(Command::ClearPattern { index: 0 });
        assert!(
            e.undo.lock().unwrap().available_pattern(0),
            "ClearPattern must push a pattern snapshot"
        );
        assert!(
            !e.snapshot_arc().patterns[0].as_ref().unwrap().tracks[0].steps[0].active,
            "Clear resets steps"
        );

        // Drain any FullSnapshot the Clear emitted.
        while e.large_events.dequeue().is_some() {}

        // Undo restores.
        e.apply_command(Command::UndoPattern { index: 0 });
        let got = e.snapshot_arc().patterns[0].as_ref().unwrap();
        assert!(
            got.tracks[0].steps[0].active,
            "UndoPattern must restore the cleared step"
        );
        assert_eq!(got.id, before.id, "UndoPattern preserves id");
        assert!(
            !e.undo.lock().unwrap().available_pattern(0),
            "snapshot consumed (one-deep)"
        );
        // A FullSnapshot was published on restore.
        let mut saw_snapshot = false;
        while let Some(bytes) = e.large_events.dequeue() {
            let ev: EngineEvent = postcard::from_bytes(&bytes).unwrap();
            if matches!(ev, EngineEvent::FullSnapshot { .. }) {
                saw_snapshot = true;
            }
        }
        assert!(saw_snapshot, "UndoPattern must publish a FullSnapshot");
    }

    /// #34: CopyPattern is clipboard-only — it must NOT push a pattern snapshot.
    #[test]
    fn copy_pattern_pushes_no_undo_snapshot() {
        let e = Engine::new();
        e.apply_command(Command::CopyPattern { index: 0 });
        assert!(
            !e.undo.lock().unwrap().available_pattern(0),
            "CopyPattern must not push an undo snapshot"
        );
    }

    /// #34: a UndoPattern with no prior mutating op is a silent no-op (always-on
    /// UI) — no state change, no FullSnapshot.
    #[test]
    fn undo_pattern_with_no_snapshot_is_a_silent_noop() {
        let e = Engine::new();
        let before = e.snapshot_arc();
        e.apply_command(Command::UndoPattern { index: 0 });
        assert_eq!(
            e.snapshot_arc().patterns[0],
            before.patterns[0],
            "no-op Undo must not mutate"
        );
        assert!(
            e.large_events.dequeue().is_none(),
            "no-op Undo must not publish a FullSnapshot"
        );
    }

    /// #34: LoadSession drains pattern-undo slots (#30 parity) — a stale snapshot
    /// cannot restore an old-session pattern onto a reloaded one.
    #[test]
    fn load_session_drains_pattern_undo_slots() {
        use crate::serde_ext::SessionEnvelope;
        let e = Engine::new();
        e.apply_command(Command::ClearPattern { index: 3 });
        assert!(e.undo.lock().unwrap().available_pattern(3));
        // Load a fresh session.
        let env = SessionEnvelope::wrap(Session::default());
        let bytes = postcard::to_allocvec(&env).unwrap();
        e.apply_command(Command::LoadSession { bytes });
        assert!(
            !e.undo.lock().unwrap().available_pattern(3),
            "LoadSession must drain pattern-undo slots"
        );
    }

    /// #34 / D6: a ClearPattern on the ACTIVE pattern clears the per-track undo
    /// (now stale w.r.t. that pattern). A Roll pushed a per-track snapshot;
    /// after the whole-pattern Clear it must be gone.
    #[test]
    fn clear_pattern_on_active_clears_per_track_undo() {
        use crate::models::MAX_TRACKS;
        let e = Engine::new();
        e.apply_command(Command::Roll { track_idx: 0, strength: 0.5 });
        assert!(
            e.undo.lock().unwrap().available(0),
            "Roll pushes a per-track snapshot for track 0"
        );
        e.apply_command(Command::ClearPattern { index: 0 }); // active pattern
        for t in 0..MAX_TRACKS {
            assert!(
                !e.undo.lock().unwrap().available(t),
                "per-track slot {t} cleared by D6"
            );
        }
    }
```

> **Accessor note:** `snapshot_arc()`, `undo`, `large_events` are all `pub` engine fields/methods already used by the `load_session_clears_stale_undo_*` test. `EngineEvent` and `postcard` are already in scope in the test module (the existing test decodes `large_events` the same way). If `VelocityZone` / `MAX_TRACKS` are not in scope, the `use` lines above import them locally.

- [ ] **Step 2: Run the tests to verify they fail**

Run (from `engine/`):
```bash
cargo test -p sequencer_engine pattern_undo_restores_after_clear
cargo test -p sequencer_engine copy_pattern_pushes_no_undo_snapshot
cargo test -p sequencer_engine load_session_drains_pattern_undo_slots
cargo test -p sequencer_engine clear_pattern_on_active_clears_per_track_undo
```
Expected: FAIL — `pattern_undo_restores_after_clear` (the temporary no-op arm does nothing, so the snapshot is never consumed / step never restored); `copy_pattern_pushes_no_undo_snapshot` may already PASS (Copy is in a separate arm); the others fail because no snapshot is pushed / drained / cleared yet. (At minimum `pattern_undo_restores_after_clear` must fail.)

- [ ] **Step 3: Push a snapshot before mutation + D6 clear in the pattern arm**

In `engine/crates/core/src/engine.rs`, replace the `CutPattern | PastePattern | ClearPattern` arm body (lines ~1057-1087 — the block starting `let index = *index;` through the arm's closing `}`):

```rust
                let index = *index;
                if index < crate::models::PATTERN_SLOTS {
                    let mut s = (*self.snapshot.load_full()).clone();
                    // #34: push the pattern-undo snapshot BEFORE the mutation
                    // (mirrors the per-track push). One-deep; the slot is always
                    // Some for these ops, so the snapshot is always captured.
                    if let Some(p) = s.patterns.get(index).and_then(|opt| opt.as_ref()) {
                        self.undo.lock().unwrap().push_pattern(index, p);
                    }
                    // Inner match is exhaustive over the three variants the outer
                    // guard admits. A stray variant here is a logic bug → loud
                    // panic, not a silent drop.
                    let mutated = match cmd {
                        CutPattern { .. } => {
                            self.clipboard.lock().unwrap().cut_pattern(&mut s, index)
                        }
                        PastePattern { .. } => {
                            self.clipboard.lock().unwrap().paste_pattern(&mut s, index)
                        }
                        ClearPattern { .. } => {
                            crate::clipboard::Clipboard::clear_pattern(&mut s, index)
                        }
                        _ => unreachable!(
                            "inner match constrained to Cut/Paste/Clear by outer guard"
                        ),
                    };
                    if mutated {
                        // D6: a whole-pattern change invalidates the active
                        // pattern's per-track snapshots (now stale). Clear them
                        // when the target is the active pattern.
                        if index == s.active_pattern_index {
                            self.undo.lock().unwrap().clear_tracks();
                        }
                        self.publish(s);
                        let snap = self.snapshot.load_full();
                        crate::midi_out::push_large_event(
                            &self.large_events,
                            EngineEvent::FullSnapshot {
                                session: (*snap).clone(),
                            },
                        );
                    }
                }
```

- [ ] **Step 4: Replace the temporary no-op `UndoPattern` arm with the restore arm**

In `engine/crates/core/src/engine.rs`, replace the temporary arm from Task 2:

```rust
            UndoPattern { .. } => {}
```

with:

```rust
            // #34: pattern-level undo. Restore the pre-mutation PatternSnapshot
            // for slot `index` (one-deep). Always-on UI — a missing snapshot is
            // a silent no-op (no publish). Restore overwrites tracks +
            // follow_action, leaves the pattern's id, keeps the slot Some.
            // D6: clearing the active pattern's per-track undo applies here too.
            UndoPattern { ref index } => {
                let index = *index;
                let mut s = (*self.snapshot.load_full()).clone();
                let restored = self.undo.lock().unwrap().undo_pattern(&mut s, index);
                if restored {
                    if index == s.active_pattern_index {
                        self.undo.lock().unwrap().clear_tracks();
                    }
                    self.publish(s);
                    let snap = self.snapshot.load_full();
                    crate::midi_out::push_large_event(
                        &self.large_events,
                        EngineEvent::FullSnapshot {
                            session: (*snap).clone(),
                        },
                    );
                }
            }
```

- [ ] **Step 5: Drain pattern slots in the LoadSession success branch**

In `engine/crates/core/src/engine.rs`, in the `LoadSession` success branch, immediately after the per-track `UndoAvailable { false }` burst loop (the `for (track_idx, was_occupied) in occupied.iter().enumerate() { … }` block, ~lines 921-938), add:

```rust
                        // #34: drain pattern-undo slots too — a wholesale reload
                        // invalidates them (else a later UndoPattern would restore
                        // an old-session pattern). No event burst: the mirror does
                        // not track pattern-undo (always-on UI).
                        let _ = self.undo.lock().unwrap().take_occupied_patterns();
```

- [ ] **Step 6: Run the engine tests to verify they pass**

Run (from `engine/`):
```bash
cargo test -p sequencer_engine pattern_undo_restores_after_clear
cargo test -p sequencer_engine copy_pattern_pushes_no_undo_snapshot
cargo test -p sequencer_engine undo_pattern_with_no_snapshot_is_a_silent_noop
cargo test -p sequencer_engine load_session_drains_pattern_undo_slots
cargo test -p sequencer_engine clear_pattern_on_active_clears_per_track_undo
```
Expected: all PASS.

- [ ] **Step 7: Full core suite + clippy + iOS guard + header diff**

Run (from `engine/`, with `$HOME/.cargo/bin` on `PATH`):
```bash
cargo test -p sequencer_engine
cargo clippy -p sequencer_engine --all-targets -- -D warnings
cargo check -p sequencer_engine --target aarch64-apple-ios
```
Then (from repo root):
```bash
git diff --exit-code -- engine/include/sequencer_engine.h
```
Expected: all PASS / no warnings / iOS check succeeds / header byte-identical. Also re-run `cargo test -p sequencer_engine_ffi` to confirm the C-ABI round-trip still passes with the real arm.

- [ ] **Step 8: Commit**

```bash
git add engine/crates/core/src/engine.rs
git commit -m "$(cat <<'EOF'
feat(engine): wire pattern-level undo snapshot/restore (#34)

Push a PatternSnapshot before Cut/Paste/Clear; Command::UndoPattern
restores it (publishes FullSnapshot, preserves id, keeps slot Some).
CopyPattern stays clipboard-only (no snapshot). LoadSession drains
pattern slots (#30 parity). Hardening: a pattern op or UndoPattern on
the active pattern clears the now-stale per-track undo (D6).

Always-on UI — a missing snapshot is a silent no-op. No new EngineEvent;
no serde bump; header unchanged.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: CLAP editor — Undo button in PatternOptionsSheet

**Files:**
- Modify: `engine/crates/editor_egui/src/pattern_options.rs`:
  - extend the test-only `ActionClip` enum with `Undo`
  - add an always-on Undo button to the clipboard row + record its rect
- Test: `engine/crates/editor_egui/src/pattern_options.rs` (extend `clipboard_buttons_emit_expected_commands_and_stay_open`)

**Interfaces:**
- Consumes: `Command::UndoPattern { index }` (Task 2), the existing `clip_btn` helper, `CommandSink`, the `#[cfg(test)]` rect-recording harness (`clip_rect_id`, `ActionClip`, `clip_center`, `open_for`, `Harness`).
- Produces: an always-on Undo button in the editor's PatternOptionsSheet.

- [ ] **Step 1: Extend `ActionClip` with `Undo`**

In `engine/crates/editor_egui/src/pattern_options.rs`, add `Undo` to the test-only enum (lines 70-76):

```rust
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionClip {
    Cut,
    Copy,
    Paste,
    Clear,
    Undo,
}
```

- [ ] **Step 2: Extend the failing test to cover Undo**

Replace the `for clip in [ … ]` array in `clipboard_buttons_emit_expected_commands_and_stay_open` (lines 831-836) and add the `Undo` arm to its `match` (lines 846-852). The new array:

```rust
        for clip in [
            ActionClip::Cut,
            ActionClip::Copy,
            ActionClip::Paste,
            ActionClip::Clear,
            ActionClip::Undo,
        ] {
```

And add the Undo arm inside the `let ok = match (clip, &cmds[0]) { … }`:

```rust
            (ActionClip::Undo, Command::UndoPattern { index }) => *index == 2,
```

- [ ] **Step 3: Run the test to verify it fails**

Run (from `engine/`):
```bash
cargo test -p stepforge_editor_egui clipboard_buttons_emit_expected_commands_and_stay_open
```
Expected: FAIL — `clip_center(&h.ctx, ActionClip::Undo)` panics ("clip Undo rect recorded") because the Undo button does not exist yet.

- [ ] **Step 4: Add the always-on Undo button**

In `engine/crates/editor_egui/src/pattern_options.rs`, in the clipboard `ui.horizontal` block, immediately after the `clear` button block (after the `if clear.clicked() { … }` close, before the `ui.horizontal`'s closing `})` — line ~348), add a fifth button:

```rust
                        let undo = clip_btn(ui, "Undo");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionClip, Rect)>>(clip_rect_id())
                                .push((ActionClip::Undo, undo.rect))
                        });
                        if undo.clicked() {
                            // #34: pattern-level undo. Always-on (no gating) —
                            // the engine no-ops when no snapshot. The sheet stays
                            // open; the per-frame re-seed (#42) re-syncs the
                            // follow_action draft from the restored pattern next
                            // frame.
                            sink.push(Command::UndoPattern { index: pattern_idx });
                        }
```

- [ ] **Step 5: Run the test to verify it passes**

Run (from `engine/`):
```bash
cargo test -p stepforge_editor_egui clipboard_buttons_emit_expected_commands_and_stay_open
```
Expected: PASS (Undo emits `Command::UndoPattern { index: 2 }`, sheet stays open).

- [ ] **Step 6: Full editor suite + clippy**

Run (from `engine/`):
```bash
cargo test -p stepforge_editor_egui
cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings
```
Expected: all PASS / no warnings.

- [ ] **Step 7: Commit**

```bash
git add engine/crates/editor_egui/src/pattern_options.rs
git commit -m "$(cat <<'EOF'
feat(editor): Undo button in PatternOptionsSheet (#34)

Add an always-on Undo tile next to Cut/Copy/Paste/Clear. Emits
Command::UndoPattern { index }; the sheet stays open and the #42 re-seed
re-syncs the follow_action draft after the restore. Always-on — the
engine no-ops when no snapshot (no gating event, matching the design).

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: iOS — `.undoPattern` + encoder + golden + PatternOptionsSheet UI

**Files:**
- Modify: `app/StepForge/Engine/Command.swift` (append case + tag + encoder arm)
- Modify: `app/StepForgeTests/PostcardTests.swift` (variant-order assertion + golden test + generate the fixture)
- Modify: `app/StepForge/Features/Performance/PatternOptionsSheet.swift` (Undo tile, stop auto-dismiss after Cut/Paste/Clear, update the Clear confirm text)
- Create: `app/StepForgeTests/Fixtures/cmd_undopattern_6.bin` (engine-generated golden)

**Interfaces:**
- Consumes: `Command::UndoPattern { index }` postcard tag 39 (Task 2); the existing `TileButton`, `bridge.submit`, `EngineBridge`, `Haptics`.
- Produces: `Command.undoPattern(index: Int)` (tag 39) + an always-on Undo tile in the iOS PatternOptionsSheet.

- [ ] **Step 1: Add the variant + tag + encoder arm**

In `app/StepForge/Engine/Command.swift`:

(a) Append the case after `case clearPattern(index: Int) // 38` (line 57):

```swift
    case undoPattern(index: Int)                                         // 39
```

(b) Add the tag to the `tag` switch (after `case .clearPattern: 38` — line 71):

```swift
        case .undoPattern: 39
```

(c) Add the encoder arm. In `encode()`, change the combined clipboard line (line 103):

```swift
        case .copyPattern(let i), .cutPattern(let i), .pastePattern(let i), .clearPattern(let i):
            w.writeUInt(UInt(i))
```

to:

```swift
        case .copyPattern(let i), .cutPattern(let i), .pastePattern(let i), .clearPattern(let i), .undoPattern(let i):
            w.writeUInt(UInt(i))
```

- [ ] **Step 2: Generate the engine golden fixture**

The golden `.bin` is generated by the Rust engine (zero hand-transcription). Add a temporary ignored test in `engine/crates/ffi/src/command_codec.rs` (inside the `#[cfg(test)] mod tests` block):

```rust
    /// Temporary: writes the #34 UndoPattern golden fixture consumed by the
    /// Swift PostcardTests. Removed after the fixture is committed.
    #[test]
    #[ignore]
    fn gen_undopattern_fixture() {
        let bytes = encode_command(&Command::UndoPattern { index: 6 }).unwrap();
        let path = format!(
            "{}/../../../app/StepForgeTests/Fixtures/cmd_undopattern_6.bin",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::write(&path, &bytes).expect("write fixture");
        eprintln!("wrote {} bytes to {}", bytes.len(), path);
    }
```

Run it from `engine/`:
```bash
cargo test -p sequencer_engine_ffi gen_undopattern_fixture -- --ignored --nocapture
```
Then remove the temporary test from `command_codec.rs` (delete the `gen_undopattern_fixture` fn). Verify the file exists:
```bash
ls -la ../app/StepForgeTests/Fixtures/cmd_undopattern_6.bin
```
Expected: a 2-byte file (postcard tag 39 = `0x27`, varint 6 = `0x06`).

- [ ] **Step 3: Add the PostcardTests assertions**

In `app/StepForgeTests/PostcardTests.swift`:

(a) Add a golden test after `testCommandPatternClipboard` (after line 60):

```swift
    // Pattern-level undo (issue #34).
    func testCommandUndoPattern() {
        XCTAssertEqual(Command.undoPattern(index: 6).encode(), load("cmd_undopattern_6"))
    }
```

(b) Add the variant-order assertion in `testCommandVariantOrder` (after `XCTAssertEqual(Command.clearPattern(index: 0).tag, 38)` — line 158):

```swift
        XCTAssertEqual(Command.undoPattern(index: 0).tag, 39)
```

- [ ] **Step 4: Add the iOS Undo tile + stop auto-dismiss after Cut/Paste/Clear**

In `app/StepForge/Features/Performance/PatternOptionsSheet.swift`, replace the whole-pattern `Section("Pattern")` block (lines 27-55) with:

```swift
                // Whole-pattern clipboard (issue #33) + undo (#34). Engine-side
                // clipboard; Copy emits no event, Paste/Cut/Clear publish a
                // FullSnapshot only on mutation. The sheet STAYS OPEN after
                // Cut/Paste/Clear/Undo so the user can tap Undo immediately and
                // see the restored state; Copy still dismisses (non-mutating,
                // no undo). Clear confirms before mutating; it is now undoable.
                Section("Pattern") {
                    HStack(spacing: 6) {
                        TileButton("Cut",   "scissors")         { bridge.submit(.cutPattern(index:   patternIdx)) }
                        TileButton("Copy",  "doc.on.doc")       { bridge.submit(.copyPattern(index:  patternIdx)); dismiss() }
                        TileButton("Paste", "doc.on.clipboard") { bridge.submit(.pastePattern(index: patternIdx)) }
                        TileButton("Clear", "trash")            { showClearConfirm = true }
                        TileButton("Undo",  "arrow.uturn.backward") { bridge.submit(.undoPattern(index: patternIdx)) }
                    }
                    .confirmationDialog(
                        "Clear pattern \(patternIdx + 1)?",
                        isPresented: $showClearConfirm,
                        titleVisibility: .visible
                    ) {
                        Button("Clear", role: .destructive) {
                            bridge.submit(.clearPattern(index: patternIdx))
                            Haptics.confirm()
                        }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("Removes all programmed steps. The slot stays. Undo is available from this sheet.")
                    }
                }
```

Changes vs. the original: Cut/Paste/Clear/Undo no longer call `dismiss()` (only Copy does); Clear's confirm action drops `dismiss()`; the confirm message is updated; a fifth `TileButton("Undo", …)` is added.

- [ ] **Step 5: Build the engine, then the iOS app**

From `engine/`:
```bash
./scripts/build_engine.sh
```
Then from repo root:
```bash
cd app && xcodegen generate
xcodebuild -project StepForge.xcodeproj -scheme StepForge \
  -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build
```
Expected: the engine builds; `xcodegen generate` regenerates the project; the app builds (the new `Command.undoPattern` case + encoder compile; `PatternOptionsSheet` compiles with the new tile).

> If `xcodebuild` fails on a missing xcframework, run `engine/scripts/build_engine.sh` first (the preBuildScript cannot self-bootstrap it — see project memory).

- [ ] **Step 6: Run the PostcardTests (wire-format verification)**

From the `app/` directory:
```bash
xcodebuild -project StepForge.xcodeproj -scheme StepForge \
  -destination 'platform=iOS Simulator,name=iPhone 16' \
  -only-testing:StepForgeTests/PostcardTests test
```
Expected: `testCommandUndoPattern`, `testCommandVariantOrder`, and the existing tests PASS (the Swift encoder matches the engine-generated golden byte-for-byte; tag 39 verified).

- [ ] **Step 7: Commit**

```bash
git add app/StepForge/Engine/Command.swift \
        app/StepForgeTests/PostcardTests.swift \
        app/StepForgeTests/Fixtures/cmd_undopattern_6.bin \
        app/StepForge/Features/Performance/PatternOptionsSheet.swift
git commit -m "$(cat <<'EOF'
feat(ios): pattern-level undo in PatternOptionsSheet (#34)

Add Command.undoPattern(index:) (postcard tag 39) + an engine-generated
golden fixture + a variant-order assertion. The sheet gains an always-on
Undo tile and stops auto-dismissing after Cut/Paste/Clear/Undo (Copy
still dismisses); Clear's confirm text now notes Undo is available.

Symmetric with the CLAP editor and the core Command::UndoPattern arm.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Final verification (before opening the PR)

- [ ] From `engine/` (with `$HOME/.cargo/bin` on `PATH`):
```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo check -p sequencer_engine --target aarch64-apple-ios
```
- [ ] From repo root — the C header is byte-identical (no Command/Event shape change):
```bash
git diff --exit-code -- engine/include/sequencer_engine.h
```
- [ ] From `app/` — iOS build + PostcardTests pass.
- [ ] From `engine/` — CLAP editor tests pass (`cargo test -p stepforge_editor_egui`).
- [ ] RT audit: `grep -n "self.undo.lock()" engine/crates/core/src/engine.rs` — confirm every hit is inside `apply_command` (worker thread), none on the RT path.

## Open the PR

```bash
cd /Users/gus/Git/StepForge
git -C .claude/worktrees/pattern-undo push -u origin feat/pattern-level-undo
gh pr create --title "feat: pattern-level undo for whole-pattern Cut/Paste/Clear (#34)" \
  --body "Closes #34. One-deep per-slot undo across core + CLAP editor + iOS. See docs/superpowers/specs/2026-08-05-pattern-level-undo-design.md."
```

Then, if the worktree is no longer needed: remove it from the main checkout with `git worktree remove .claude/worktrees/pattern-undo` (the branch persists on the remote).
