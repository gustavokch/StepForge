# Bug-Sweep — Correctness Debt Retirement

**Date:** 2026-08-04
**Status:** Design approved (revised after code read); pending implementation plan
**Phase:** Pre-T13 debt sweep (synth-ranked first in the roadmap sequence)

## Goal

Retire the open correctness + safety debt from the T12 whole-pattern clipboard
and PerformanceView work **before** opening any new surface area. Six issues
across the Rust core and the CLAP egui editor. All fixes are localized,
introduce no new `Command` / `EngineEvent`, cross no FFI or Swift-mirror
boundary, and carry no orphan risk.

## Non-goals (explicitly deferred or closed)

- **#34 — pattern-level undo.** Structural: undo is track-scoped (`undo.rs`,
  `Command::Undo { track_idx }`), so whole-pattern Clear/Cut/Paste have no
  undo slot. Needs an undo-system design decision (pattern-level stack vs
  generalized `Undo`) + a symmetric cross-layer change. Folded into the later
  **T12 parity** phase.
- **#40 — pattern paste overwrites per-track `midi_note` / `Track.id`.**
  **Closed as by-design.** `paste_pattern` replaces the target pattern's
  entire `tracks` Vec, so each track's `midi_note` and `id` come from the
  source. The doc-comment at `clipboard.rs:16-19` explicitly owns the
  midi_note divergence ("a whole-pattern clone is meant to overwrite the
  target's midi_notes too, unlike a track paste"), and the test
  `pattern_copy_then_paste_preserves_target_id_overwrites_tracks` asserts the
  overwrite. Whole-pattern paste is a **new operation** that the track-scoped
  "Paste never midi_note" rule does not govern. Issue #40 itself was filed
  "low/conventions… if deemed in scope." Decision: not in scope.
- **#37 — PerformanceView retrigger on active-but-empty slot.** **Closed as
  latent/unreachable.** `cell_state` returns `Playing` for the active slot
  regardless of filled, but the active-slot-is-always-`Some` invariant
  (`engine.rs:83`, validated on `LoadSession`; `clear_pattern` keeps the slot
  `Some`) makes an active `None` slot unreachable today. Pure dead-code
  hardening, not a reachable bug.
- **#7 — cross-platform desktop.** Separate distribution effort.
- DAW smoke testing. No fix touches RT-thread timing or host I/O.

## Scope — two PRs by surface

### PR 1 — core cluster (`fix/core-clipboard-undo`)

| Issue | Root cause (verified) | Fix | Size |
|---|---|---|---|
| **#30** LoadSession leaves stale undo snapshots | `engine.rs:903-922` LoadSession arm publishes, bumps the reload generation, cancels the scheduler, and emits `FullSnapshot` — but never clears the per-track undo slots (`self.undo`) nor emits any `UndoAvailable` event. Old-session snapshots survive a mid-session reload, so a later `Undo { track_idx }` restores an old-session track onto the new session. Unmasked by #29 (which made `FullSnapshot` stop clearing `undo_available`). | Add `Undo::take_occupied(&mut self) -> Vec<usize>` (returns indices that were `Some`, clears them). In the LoadSession arm, after `publish`, emit `UndoAvailable { track_idx, available: false }` for each occupied slot (bounded by `MAX_TRACKS = 8`), then the slots are already cleared. Reuses the existing event — no new variant, no Swift-mirror change. | S |
| **#41** Pattern-clipboard arm clones the whole `Session` even for clipboard-only Copy | `engine.rs:1019` does `let mut s = (*self.snapshot.load_full()).clone();` **unconditionally** for all four variants before the inner match. `CopyPattern` only reads one pattern (`mutated=false`), so the full 9-pattern deep clone is paid and dropped on every Copy click. | Restructure so the session clone is taken only on the mutating path. `CopyPattern` reads a single pattern via `self.snapshot.load_full()` (shared borrow) without the full clone; Cut/Paste/Clear keep the clone they need. Gate strictly on the variant. | S |

### PR 2 — editor_egui cluster (`fix/editor-perfview-options`)

| Issue | Root cause (verified) | Fix | Size |
|---|---|---|---|
| **#42** Clipboard buttons `close(ctx)`-on-emit masks a stale `PatternOptions` draft | `pattern_options.rs` `PatternOptionsState` is seeded from the target pattern once on `open()` and never re-synced while the sheet is open. An external `SetFollowAction` (preset, automation, undo swap) leaves the draft stale and the sheet lying. The clipboard buttons' `close(ctx)` masks the staleness for the paste case. | Re-sync `PatternOptionsState` from the target pattern **every frame** rather than only on `open()`, skipping the field currently holding edit focus so an in-progress draft edit is not clobbered mid-interaction. | M |
| **#36** After-Loops Slider (1..=16) desyncs from the unbounded u32 model field | `after_loops` is `u32`; the slider is range-bounded `1..=16`, but the draft is seeded with the raw value and the label prints it, so an out-of-range session (e.g. `after_loops=32`) shows slider=16, label=32, state=32. | Clamp `after_loops` to `1..=16` in the per-frame re-seed path (#42). | S |
| **#39** Paste on an empty clipboard is a silent no-op but still closes the sheet | `paste_pattern` returns `false` on an empty clipboard but the Paste button calls `close(ctx)` unconditionally, so an empty paste dismisses the sheet with no feedback. The editor cannot see clipboard state (no event carries it). | **Drop `close(ctx)` from all four clipboard buttons** (Cut/Copy/Paste/Clear). With #42's per-frame re-seed, the original reason to close (paste overwrites follow_action → stale draft) is gone — re-seed repairs the draft next frame. An empty paste becomes a visible no-op (sheet stays open, draft intact). **Accepted divergence:** the CLAP sheet no longer auto-dismisses after a clipboard action, unlike iOS. | S |
| **#38** `pulse_alpha` propagates NaN/inf unguarded into playing-cell fill/stroke | `pulse_alpha(time) = (time * TAU * 0.75).sin() * 0.5 + 0.5` feeds `cell_fill` `gamma_multiply` and `cell_stroke` width on the active cell with no `is_finite` guard. A non-finite wall-clock (suspend/resume, platform clock glitch) yields a NaN fill (saturates to black) and a NaN stroke width (tessellation artifact). | `is_finite` guard; fall back to a neutral alpha (`0.5`) and neutral stroke pulse (`0.0`) when the input is non-finite. | S |

## Build order

PR 1 (core) first, then PR 2 (editor). The PRs are independent and may merge
in any order, but core is sequenced first: #30 is the highest-severity item
(medium, cross-session undo corruption) and was unmasked by already-merged #29.

Within PR 2, #42 (per-frame re-seed) is implemented first — #36's clamp falls
out of the re-seed path, and #39's drop-close-on-emit is only safe once the
re-seed exists (otherwise paste leaves a stale draft). #38 is independent
within the PR.

## Verification bar (per PR)

Headless + iOS-sim, no DAW smoke (no fix touches RT-thread timing or host I/O).

- **PR 1 (core):**
  - New unit tests in `sequencer_engine`:
    - #30 — `publish` session A → `Roll { 0 }` (pushes undo) → `LoadSession`
      valid envelope for session B → assert `undo.available(0) == false` →
      `apply_command(Undo { 0 })` → session B track 0 unchanged. Also assert
      the `UndoAvailable { available: false }` events are emitted for the
      previously-occupied slots.
    - #41 — `CopyPattern` does not perform a full-session deep clone (assert
      via behavior: Copy emits no `FullSnapshot` and `reload_generation` /
      session identity are unchanged; the clone-skip is an internal detail).
  - `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`.
  - iOS guard: `cargo check --target aarch64-apple-ios` with the rustup
    toolchain on `PATH` (`$HOME/.cargo/bin`), **not** Homebrew `rustc`.
  - cbindgen header byte-diff:
    `git diff --exit-code -- engine/include/sequencer_engine.h` — expected
    clean (no `Command` / `EngineEvent` shape change).
  - Swift app build (`xcodebuild` iOS-Simulator) — `SessionMirror` untouched.
- **PR 2 (editor_egui):**
  - `cargo test -p stepforge_editor_egui` — new headless editor tests:
    - #42 — open the sheet, apply an external `SetFollowAction` to the target
      pattern, settle, assert the draft re-syncs.
    - #36 — seed a pattern with `after_loops = 32`, open the sheet, assert the
      draft clamps to 16.
    - #39 — open the sheet, click each clipboard button, assert the command is
      emitted AND the sheet stays open (`target` still `Some`).
    - #38 — `pulse_alpha(f64::NAN)`, `pulse_alpha(f64::INFINITY)`, and
      `pulse_alpha(f64::NEG_INFINITY)` each return a finite value in `[0,1]`;
      `cell_fill(Playing, nan_pulse)` and `cell_stroke(Playing, _, nan_pulse)`
      are finite.
  - `cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings`.

## Branch strategy

Two branches off `origin/main`. The working tree currently sits on
`feat/ios-pattern-clipboard` with untracked files (`.agents/`, `mono_seq/`,
handoff docs, `build_install_macos.sh`, `skills-lock.json`) — these are left
in place; each fix branch is cut clean off `origin/main`.

Project memory flags a **concurrent-session hazard**: the user runs multiple
Claude sessions in the shared tree, and one can merge or switch `HEAD` mid-run
(PR #22 split one plan across two branches). Mitigations:

- Re-verify `HEAD` ancestry against `origin/main` before each dispatch.
- Use absolute paths and `git -C <abs-repo>` for worktree/branch commands
  (Bash CWD persists across calls).
- A git worktree per PR is optional but recommended if other sessions are
  active during the sweep.

## Risks and load-bearing details

- **#30** — because #29 decoupled `FullSnapshot` from `undo_available`, the
  explicit `take_occupied` + `UndoAvailable { false }` burst on LoadSession is
  load-bearing: the existing `FullSnapshot` publish will **not** clear it
  implicitly. The test must assert both the engine slots are cleared and the
  events fire. `RequestFullSnapshot` (`engine.rs:868`) is NOT a reload — do
  not reset undo there (UI just asking for current state).
- **#41** — the clone skip must be gated strictly on the `CopyPattern`
  variant. Cut/Paste/Clear still require the full clone (they mutate `&mut s`
  and publish). The restructure must satisfy the borrow checker: `copy_pattern`
  takes `&Session` + `&mut self.clipboard`; reading the single pattern from the
  `Arc`-loaded snapshot without the clone must not hold a borrow across the
  clipboard lock.
- **#42** — re-seed every frame, but skip the field currently holding edit
  focus (the slider for `after_loops`, the ComboBox for `action`, the target
  picker for `specific_target`) so an in-progress edit is not clobbered. The
  focus-skip is the one implementation detail to get right.
- **#39** — dropping close-on-emit diverges from the iOS
  `PatternOptionsSheet`, which dismisses after a clipboard action. This is a
  deliberate UX call (Copy-then-Paste without reopening; empty paste is a
  visible no-op rather than a silent dismiss), accepted in design review. If
  iOS parity is later required, a clipboard-presence event would be needed —
  out of scope for this no-new-Event sweep.
- **#38** — the neutral fallback must keep the cell visible (a neutral alpha
  of `0.5` mid-pulse, not `0.0` which would black it out).

## Cross-cutting constraints (apply to every fix)

- `nih_plug` pin stays at shared rev `f36931f` for any editor-side change.
- The V5 iOS guard must stay green.
- No new `Command` / `EngineEvent` ⇒ symmetric cross-layer `/add-feature` rule
  does not engage; all fixes are internal to existing arms/widgets.
- `#![forbid(unsafe_code)]` in `sequencer_engine` is preserved; no `unsafe`
  touched.
