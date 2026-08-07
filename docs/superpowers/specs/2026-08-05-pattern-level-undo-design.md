# Pattern-Level Undo — Design

- **Date:** 2026-08-05
- **Status:** Approved in brainstorming; pending implementation
- **Closes:** #34 — *Pattern-level undo for whole-pattern Copy/Cut/Paste/Clear*
- **Branch:** `feat/pattern-level-undo` (off `origin/main`, post-bug-sweep)
- **Scope:** Make whole-pattern `CutPattern` / `PastePattern` / `ClearPattern` undoable, one undo per pattern slot, across the Rust core, the Swift app, and the CLAP egui editor.

---

## 1. Problem

The per-track `Undo` (`engine/crates/core/src/undo.rs`) is one-deep, in-memory, keyed by `track_idx` into `[Option<TrackSnapshot>; MAX_TRACKS]`. It restores one track on the active pattern. The whole-pattern clipboard ops added in T12 (PR #32) — `CutPattern` / `PastePattern` / `ClearPattern` — mutate an entire pattern slot (any of the 9), publish a `FullSnapshot`, and **push no undo snapshot today**. The engine states this explicitly:

> `engine.rs` pattern-clipboard arm comment: *"No per-track undo (undo is track-scoped) — pattern ops are not undoable."*

So a destructive `ClearPattern` (wipes every track's steps) or a `PastePattern` (overwrites every track wholesale) is permanent. The iOS `PatternOptionsSheet` Clear confirm dialog even warns the user: *"the steps are not undoable."* `CopyPattern` is clipboard-only (no mutation, no publish) and correctly excluded from undo by symmetry with per-track `Copy`.

## 2. Goals

- `UndoPattern { index }` restores a pattern slot to its pre-mutation state after a `CutPattern` / `PastePattern` / `ClearPattern` on that slot.
- One undo per slot (one-deep), mirroring the per-track undo depth.
- Symmetric across all three surfaces (core + Swift + CLAP editor) — no orphans.
- RT-safety preserved (Hard Rule 1); no new `EngineEvent`; no `SESSION_FORMAT_VERSION` bump; C header byte-identical.

## 3. Non-goals

- Multi-step (stack) history. One-deep only, matching per-track undo.
- Undo for `CopyPattern` (non-mutating; excluded by precedent).
- Persisting undo snapshots to disk. The store stays in-memory engine state, like per-track undo.
- Fixing the pre-existing cross-pattern-switch staleness in per-track undo (a snapshot taken on pattern A can be restored onto pattern B after a switch). Documented separately; out of scope.

## 4. Decisions

| # | Decision | Rationale |
|---|---|---|
| D1 | **One-deep per slot.** Each of the 9 pattern slots holds its single last pre-mutation `PatternSnapshot`. | Parity with the per-track `Undo` (also one-deep). Smallest cross-layer surface. |
| D2 | **Always-on undo button; no new `EngineEvent`.** The button is always enabled; the engine no-ops when the slot has no snapshot. | Smallest ripple (no event codec, no mirror field). Matches iOS per-track undo behavior. Accepted trade-off: a tap with nothing to undo is a silent no-op. |
| D3 | **In-sheet Undo tile on both surfaces; iOS stops auto-dismissing after Cut/Paste/Clear.** | Editor sheet already stays open (#39). iOS `PatternOptionsSheet` currently dismisses after every clipboard op — an in-sheet undo tile would vanish before tap. Aligning iOS with the editor's stay-open makes undo one tap. The Clear confirm text is updated (steps are now undoable). |
| D4 | **Extend the existing `Undo` struct** with a pattern-slot array (Approach A). | Mirrors the `Clipboard` struct, which already unifies `track` + `pattern`. One `Mutex<Undo>`, one LoadSession drain, a cohesive undo module. Rejected: a sibling `PatternUndo` struct (two mutexes, two drains, no benefit). |
| D5 | **`PatternSnapshot { tracks, follow_action }` — no `id`.** | Mirrors `PatternClipboard`. `id` is preserved by every mutating op (`Cut`/`Clear` don't touch it; `Paste` preserves the target's `id`), so storing it is redundant. Restore overwrites `tracks` + `follow_action`, leaves `id`. |
| D6 | **Hardening: clear per-track `Undo.slots` when a pattern op or `UndoPattern` targets the active pattern.** | A whole-pattern change makes the active pattern's per-track snapshots stale. Clearing them prevents a per-track undo from resurrecting a stale track across a whole-pattern change. Few lines; removes a real footgun. The cross-switch staleness (non-active target) stays out of scope. |

## 5. Data model

### `PatternSnapshot` (new, `undo.rs`)

```rust
/// What one pattern-level undo snapshot captures: the full `tracks` plus the
/// `follow_action`. `id` is excluded — every mutating pattern op preserves the
/// target pattern's `id` (mirrors `PatternClipboard`). Heap-backed (`Vec<Track>`)
/// → worker-thread only, never RT.
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

### `Undo` extended (`undo.rs`)

```rust
pub struct Undo {
    slots: [Option<TrackSnapshot>; MAX_TRACKS],
    pattern_slots: [Option<PatternSnapshot>; PATTERN_SLOTS],
}
```

New methods (mirror the per-track trio + the LoadSession drain):

- `push_pattern(&mut self, index: usize, p: &Pattern)` — bounds-checked (`index < PATTERN_SLOTS`); clones into a `PatternSnapshot`; overwrites (one-deep, last-write-wins). Called BEFORE mutation, mirroring the per-track `push`.
- `undo_pattern(&mut self, s: &mut Session, index: usize) -> bool` — takes the snapshot (slot → `None`); if present, overwrites `patterns[index]`'s `tracks` + `follow_action`, **leaves `id`, keeps the slot `Some`**; returns true. Total: OOB index, missing snapshot, or `None` slot → false, no panic.
- `available_pattern(&self, index: usize) -> bool` — included for tests; not required by the UI (always-on).
- `take_occupied_patterns(&mut self) -> [bool; PATTERN_SLOTS]` — drains all pattern slots; stack-allocated flags (no heap). LoadSession parity with `take_occupied()`.
- `clear_tracks(&mut self)` — sets every per-track slot to `None`. Used by D6 when a whole-pattern change targets the active pattern. (Reuses the per-track drain logic without needing the return value.)

### `Command::UndoPattern` (new variant, appended — `command.rs`)

```rust
/// Undo the last whole-pattern op (Cut/Paste/Clear) on slot `index`.
/// One-deep; no-op if no snapshot. Always-on in the UI.
UndoPattern {
    index: usize,
},
```

Appended at the end of the `Command` enum. Wire-safe (postcard tag appended; the C header does not enumerate `Command` variants, so it stays byte-identical).

### No new `EngineEvent`

Restore publishes `EngineEvent::FullSnapshot { session }` on the existing path (same as the mutating pattern ops). No `PatternUndoAvailable` variant — D2.

## 6. Snapshot / restore flow

### Snapshot point (engine.rs, `CutPattern | PastePattern | ClearPattern` arm)

After the COW clone and before the mutation `match`, push the pre-mutation pattern snapshot — unconditionally for the three mutating ops, mirroring the per-track idiom (which pushes before mutation for `Cut`/`Paste`/`Trash`; a `Paste` with an empty clipboard leaves a phantom snapshot there too — same parity):

```rust
CutPattern { ref index }
| PastePattern { ref index }
| ClearPattern { ref index } => {
    let index = *index;
    if index < crate::models::PATTERN_SLOTS {
        let mut s = (*self.snapshot.load_full()).clone();
        // #34: push the pattern-undo snapshot BEFORE the mutation (mirrors the
        // per-track push). One-deep; clones the pre-mutation pattern. The slot
        // is always Some for these ops, so the snapshot is always captured.
        if let Some(p) = s.patterns.get(index).and_then(|opt| opt.as_ref()) {
            self.undo.lock().unwrap().push_pattern(index, p);
        }
        let mutated = match cmd {
            CutPattern { .. } => self.clipboard.lock().unwrap().cut_pattern(&mut s, index),
            PastePattern { .. } => self.clipboard.lock().unwrap().paste_pattern(&mut s, index),
            ClearPattern { .. } => crate::clipboard::Clipboard::clear_pattern(&mut s, index),
            _ => unreachable!("inner match constrained to Cut/Paste/Clear by outer guard"),
        };
        if mutated {
            // D6: a whole-pattern change invalidates the active pattern's
            // per-track snapshots (now stale). Clear them when the target is
            // the active pattern.
            if index == s.active_pattern_index {
                self.undo.lock().unwrap().clear_tracks();
            }
            self.publish(s);
            let snap = self.snapshot.load_full();
            crate::midi_out::push_large_event(
                &self.large_events,
                EngineEvent::FullSnapshot { session: (*snap).clone() },
            );
        }
    }
}
```

### Restore arm (engine.rs, new)

```rust
UndoPattern { ref index } => {
    let index = *index;
    let mut s = (*self.snapshot.load_full()).clone();
    let restored = self.undo.lock().unwrap().undo_pattern(&mut s, index);
    if restored {
        // D6: same active-pattern per-track clearing as the mutating ops.
        if index == s.active_pattern_index {
            self.undo.lock().unwrap().clear_tracks();
        }
        self.publish(s);
        let snap = self.snapshot.load_full();
        crate::midi_out::push_large_event(
            &self.large_events,
            EngineEvent::FullSnapshot { session: (*snap).clone() },
        );
    }
    // No snapshot → silent no-op (always-on UI, D2). No publish.
}
```

`CopyPattern` is a separate arm and pushes nothing (non-mutating).

## 7. RT-safety (Hard Rule 1)

- Every `self.undo.lock()` site is inside `apply_command`, which runs only on the worker thread (`run_worker_loop`, wrapped in `catch_unwind`). `run_rt_loop` keeps zero undo/clipboard references.
- `PatternSnapshot` owns a `Vec<Track>` (heap). The clone happens on the worker thread, in the same arm that already deep-clones the whole `Session` — so the marginal cost is strictly cheaper than work already on the path. The RT path never touches it.
- The `Mutex` is never held across an FFI call or on the RT path. The new lock sites are short, non-reentrant, and worker-only — matching the existing contract.

## 8. LoadSession parity (#30)

A wholesale reload invalidates prior snapshots (else a later undo restores an old-session pattern onto the new one). In the LoadSession success branch, drain the pattern slots alongside the existing per-track drain:

```rust
// Existing per-track drain (#30) — unchanged.
let occupied = self.undo.lock().unwrap().take_occupied();
for (track_idx, was_occupied) in occupied.iter().enumerate() {
    if *was_occupied {
        crate::midi_out::push_event(
            &self.hot_events,
            &EngineEvent::UndoAvailable { track_idx, available: false },
        );
    }
}
// #34: drain pattern-undo slots too. No event burst — the mirror does not
// track pattern-undo (always-on, D2), so there is nothing to tell it.
let _ = self.undo.lock().unwrap().take_occupied_patterns();
```

## 9. Cross-layer ripple (no orphans)

Per the working agreement, the cross-layer change is symmetric.

### Rust core (`sequencer_engine`)
- `undo.rs`: `PatternSnapshot`; extend `Undo` with `pattern_slots` + `push_pattern` / `undo_pattern` / `available_pattern` / `take_occupied_patterns` / `clear_tracks`; tests.
- `command.rs`: append `Command::UndoPattern { index }`.
- `command_codec` (`ffi`): encode/decode the new variant.
- `engine.rs`: snapshot-before-mutate in the shared arm; new `UndoPattern` arm; LoadSession drain; D6 active-pattern clearing.
- Tests + proptest (§11).

### Rust FFI (`sequencer_engine_ffi`)
- `command_codec` round-trip test for `UndoPattern`.
- **C header unchanged** — `engine_submit_command(ptr, len)` carries postcard bytes; the header does not enumerate `Command` variants.

### Swift app (`app/StepForge`)
- `Command.swift`: `.undoPattern(index: Int)` appended; encoder tag (parallel to `.cutPattern`/`.pastePattern`/etc.).
- `PostcardTests`: update the golden fixture for the new tag.
- `Features/Performance/PatternOptionsSheet.swift`: add an Undo `TileButton`. Dismiss rule: the sheet stays open after Cut / Paste / Clear / Undo (so the user can tap Undo immediately, then see the restored state); `dismiss()` is kept only after Copy (non-mutating; Copy has no undo) and on the existing close gesture. Update the Clear confirm text from *"the steps are not undoable"* to reflect that the op is now undoable.

### CLAP editor (`stepforge_editor_egui`)
- `pattern_options.rs`: add an always-on Undo button (template: the per-track Undo in `action_drawer.rs`, minus the gating), pushing `Command::UndoPattern { index: pattern_idx }` through the `CommandSink`. The per-frame re-seed (#42) already re-syncs the `follow_action` draft after the restore — no extra wiring. The #42 re-seed does not touch steps (steps show in the grid via `FullSnapshot`).

## 10. Bounds, invariants, edge cases

- `PATTERN_SLOTS = 9`, `MAX_TRACKS = 8` (distinct domains; the pattern store is sized `[Option<_>; 9]`).
- All pattern ops target `Some` slot (a `None` slot makes `paste_pattern` / `clear_pattern` return false — no mutation, no snapshot). The restore path keeps the slot `Some`; it never nulls a slot.
- `Pattern.id` is preserved on restore (D5): the snapshot excludes `id`; restore overwrites `tracks` + `follow_action` only.
- OOB `index` (≥ `PATTERN_SLOTS`) → no-op, no panic (bounds-checked, mirroring `Undo::undo` / `available`).
- `CutPattern` leaves the cut content in the pattern clipboard; undoing a cut restores the source steps but leaves the clipboard populated — asymmetric, identical to per-track `Cut` + `Undo` today. Accepted.

## 11. Testing

### Unit (`undo.rs`)
- `push_pattern` / `undo_pattern` / `take_occupied_patterns` semantics.
- Restore preserves `id`, slot stays `Some`, `tracks` + `follow_action` match the pre-snapshot.
- One-deep: a second `undo_pattern` on the same slot is a no-op (snapshot consumed).
- `CopyPattern` does **not** push a pattern snapshot.
- OOB index → no-op, no panic.

### Engine (`engine.rs`)
- `CutPattern` / `PastePattern` / `ClearPattern` each push a snapshot; `UndoPattern` restores the prior `Pattern` and publishes a `FullSnapshot`.
- `CopyPattern` pushes nothing; a following `UndoPattern` is a no-op (no `FullSnapshot`).
- LoadSession clears pattern-undo slots (no stale restore across a reload).
- D6: a `ClearPattern` on the active pattern clears the per-track undo for that pattern.

### Property test (`proptest`)
After any sequence of `CutPattern` / `PastePattern` / `ClearPattern` on a fixed slot, `UndoPattern` restores the slot's pre-mutation `Pattern`. Invariants preserved across the sequence:
- `patterns[index].id` unchanged by any op or by the undo.
- `patterns[index]` stays `Some`.
- `UndoPattern` with no prior mutating op is a no-op.
- `CopyPattern` anywhere in the sequence does not create a snapshot.

### FFI (`ffi`)
- `UndoPattern` byte round-trip (encode → `engine_submit_command` → decode → arm hit).

### Swift
- `PostcardTests` golden fixture for `.undoPattern(index:)`.

### CLAP editor
- Existing `pattern_options` harness: the Undo button emits `Command::UndoPattern { index }` with the correct index; the sheet stays open.

## 12. Delivery

One PR on `feat/pattern-level-undo`: core + ffi + Swift + editor together (no orphans). The branch is already off `origin/main` with the bug-sweep fixes present. The PR body references this design doc and closes #34.

## 13. References

- Per-track undo: `engine/crates/core/src/undo.rs`, `engine.rs` `Command::Undo` arm, `EngineEvent::UndoAvailable`.
- Whole-pattern ops: `engine/crates/core/src/clipboard.rs` (`copy_pattern` / `cut_pattern` / `paste_pattern` / `clear_pattern`), `engine.rs` `CopyPattern` + `CutPattern|PastePattern|ClearPattern` arms.
- LoadSession drain precedent: `engine.rs` LoadSession success branch (#30).
- iOS surface: `app/StepForge/Features/Performance/PatternOptionsSheet.swift`, `Features/Editing/ActionDrawer.swift`, `Engine/Command.swift`, `Engine/SessionMirror.swift`.
- Editor surface: `engine/crates/editor_egui/src/pattern_options.rs`, `action_drawer.rs`.
- Bug-sweep design (retired the related debt): `docs/superpowers/specs/2026-08-04-bug-sweep-design.md`.
