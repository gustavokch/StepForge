# T12 handoff — PerformanceView + PatternOptionsSheet (CLAP Phase 3)

Handoff prompt to pick up T12 in a fresh session. Self-contained.

## Prerequisite

**T11 + the Undo fix must be on `main`.** Both merged 2026-08-02: PR #28
(`417c4c0` — T11 ActionDrawer + NotePickerSheet + editor Undo fix, SPEC
`T11 .→x`) and PR #29 (`bb8e5ee` — iOS `SessionMirror` Undo fix + core
`cut_then_paste_keeps_target_midi_note` test). After fetching, `origin/main` =
`bb8e5ee`; in `SPEC.md §T`, `T11` is `x` and **`T12` is the next `.`**. Branch
off `main`.

## You are picking up

StepForge's **CLAP plugin egui editor, Phase 3.** Implement **T12 —
PerformanceView + PatternOptionsSheet**: the second editor mode (iOS has Editing
+ Performance via `AppMode`). Port iOS `PerformanceView` (large PLAY/STOP, 3×3
pattern grid with EMPTY/FILLED/PLAYING/QUEUED + glow, track activity LEDs +
mute toggles, quantize selector) and `PatternOptionsSheet` (follow-action →
`SetFollowAction`) to pure-egui, wired to **existing** pattern `Command`s. No
engine change, no new `Command`/`EngineEvent` (verify), no orphans.

This is a **pure-UI port + a mode switch.** Every pattern algorithm
(queue/switch/retrigger/follow-action) already exists in `core` and is echoed
back via `PatternQueued`/`PatternSwitched`/`PatternLoopCountChanged`/
`FollowActionChanged`. You add: an AppMode toggle, a `performance.rs` view, a
`pattern_options.rs` overlay, then headless-test + smoke-test them.

## Context — read first

- **Surface:** pure-Rust `nih_plug` + `egui` editor in
  `engine/crates/editor_egui` (lib `stepforge_editor_egui`), wrapped by
  `engine/crates/clap_plugin`. Consumes `engine/crates/core` in-process. **No
  Swift, no FFI, no `engine_*` entry points.**
- **Spec of record:** `SPEC.md §T` (T12 row) + `§V` invariants — **V4** (editor
  = pure UI, reads `UiState`, emits `Command`, never mutates engine;
  `apply`-driven, no optimistic mutation), **V1** (RT-safety — untouched;
  GUI-thread `format!`/heap is fine), **V5** (iOS build stays green), **V7**
  (no `unsafe` outside `ffi`).
- **Design doc:** `docs/superpowers/specs/2026-07-27-clap-egui-editor-design.md`
  §Widgets (~line 359): *PerformanceView: large PLAY/STOP, 3×3 pattern grid
  (EMPTY/FILLED/PLAYING/QUEUED with time-driven glow), track activity LEDs +
  mute toggles, quantize selector. PatternOptionsSheet: follow-action →
  SetFollowAction.* §Touch→mouse/keyboard (~line 365).

### iOS sources to port (read in full — they are the reference)

- **`app/StepForge/Features/AppMode.swift`** — the Editing↔Performance mode
  concept. T12 must bring mode-switching to the editor.
- **`app/StepForge/Features/Performance/PerformanceView.swift`** — the
  reference. Large play/stop, 3×3 pattern grid, track LEDs/mutes, quantize.
- **`app/StepForge/Features/Performance/PatternOptionsSheet.swift`** — the
  reference. Follow-action editor → `SetFollowAction`.

### Engine contracts (already implemented — you only emit/read; verify exact fields)

`engine/crates/core/src/command.rs`: `QueuePattern { index, quantize }`,
`CancelQueuedPattern`, `RetriggerPattern { quantize }`, `SetQuantizeGrain {
grain }`, `SetFollowAction { pattern_idx, action }`. (Confirm whether pattern
*switch* is `QueuePattern` or a separate variant.)

`engine/crates/core/src/event.rs`: `PatternQueued { index, quantize }`,
`PatternSwitched { index }`, `PatternCleared { index }`,
`PatternLoopCountChanged { count }`, `FollowActionChanged { … }`.

`engine/crates/core/src/models.rs`: `QuantizeGrain`
(`NextStep/NextBeat/NextBar/EndOfPattern`); follow-action — note
`FollowActionType` (`None/PlayNext/PlaySpecific(Uuid)/PlayPrevious/Stop/
PlayRandom`) vs the `FollowAction` the command takes — **check the exact shape**
(is `FollowAction` a struct wrapping the type + target? does `PlaySpecific`
reference a pattern by `Uuid`?).

`PATTERN_SLOTS` — verify the count (3×3 implies 9; confirm in `models.rs`).

### Editor wiring points (where T12 plugs in)

- **`engine/crates/editor_egui/src/lib.rs::render()`** — currently renders
  transport → feel → track_management → error → grid → T11 overlays. T12 adds
  an **AppMode toggle** + conditionally renders
  `performance::render_performance_view` instead of the grid in Performance
  mode. Decide the egui trigger for mode-switch (transport toggle? a key? a
  dedicated mode bar?) — read `AppMode.swift` for the iOS analog.
- **New modules:** `engine/crates/editor_egui/src/performance.rs` +
  `pattern_options.rs`; add `pub mod` lines in `lib.rs` (alphabetical).
- **Reuse the T11/T10 idioms:** `CommandSink` + the shared `test_support`
  harness (merged in `1674ccc`); widget-local state in `ctx.data` temp (like
  `Zoom`, the overlay `opened_at`); the open-frame guard (`lib.rs`
  `tick_frame`/`frame_nr`).
- **Don't duplicate the pattern command:** the FeelBar (T10d) already has a
  pattern switcher (`QueuePattern`). T12's 3×3 grid is the richer trigger
  surface — same command, new UI.
- **PatternOptionsSheet is an overlay** → use the `Area` + `Frame::popup` idiom
  (see traps), with the open-frame self-dismiss guard.

### `UiState` fields you read (`ui_state.rs`)

`queued_pattern: Option<usize>`, `queued_pattern_quantize: Option<QuantizeGrain>`,
`pattern_loop_count: u32`, plus the session's `patterns: [Option<Pattern>;
PATTERN_SLOTS]` + `active_pattern_index` (apply arms at lines ~89-119:
`PatternQueued`/`PatternSwitched`/`PatternCleared`/`PatternLoopCountChanged`).
Check what `PatternOptionsSheet` needs (per-pattern follow-action — is it stored
on `Pattern`? read `models.rs`).

## Conventions (CLAUDE.md hard rules + working agreement)

- **No orphans.** T12 adds zero `Command`/`EngineEvent` variants (verify; if
  `PatternOptionsSheet` needs something new, that's a cross-layer
  `/add-feature` — unlikely). No C-ABI round-trip needed.
- **V4 / Hard Rule 2.** Widgets read `UiState`, emit `Command`s, never mutate
  the engine. No optimistic mutation — wait for the echo.
- **RT path untouched.** GUI-thread only.
- **iOS guard (V5)** must stay green regardless.

## Known traps (do NOT chase these as your bugs)

- **egui `Window` is WRONG for click-opened overlays** — its title-bar absorbs
  the first click to acquire focus (breaks single-click). Use
  `egui::Area::new(id).order(Foreground).current_pos(p)` + `Frame::popup` (the
  ratchet-popover / T11 idiom). `Area::show` returns `InnerResponse` (use
  `.response.rect`).
- **A floating `Area`'s widgets don't click until ~4 idle `settle()` frames
  after a cold open** (headless only; ~50ms at 60fps, invisible in a host). The
  `test_support` harness has this.
- **Open-frame self-dismiss guard:** `pointer.primary_clicked()` is global +
  NOT consumption-aware — the click that opens an overlay is still
  "primary_clicked" when the overlay renders one statement later the same
  frame; for any trigger outside the overlay rect it self-dismisses. The
  `lib.rs` `tick_frame`/`frame_nr` + `opened_at` guard (T11) handles this —
  reuse it for `PatternOptionsSheet`.
- **`Label` defaults to `TextWrapMode::Extend`** (grows the parent Ui →
  overflows layouts). Use `.truncate()` for any bounded label.
- **Mirror-glue bugs are invisible to headless.** Editor unit tests seed mirror
  fields directly (`fixture_with_*()`), bypassing the `Engine → event →
  UiState::apply` pipeline. T12 adds pattern-driven UI — **add at least one
  test that replays the real event ORDER** (`PatternQueued` → `PatternSwitched`
  → `PatternLoopCountChanged`), not just `write(field)` then assert. The T11
  Undo bug (`FullSnapshot` wiped `undo_available`; both surfaces fixed in
  #28/#29) is the canonical example.
- **DAW smoke is the ONLY catch** for audible/timing behavior (headless is
  blind to RT dispatch timing). Budget a real Bitwig smoke. `clap-validator`
  crashes upstream; headless `cargo test` is source of truth. `nih-plug` pinned
  `f36931f` — don't bump.
- **`install_clap.sh` builds from the script's directory, not CWD** — run it
  from the **T12 worktree**, not the main checkout (T11 smoked a wrong-branch
  bundle this way). Then **`Cmd+Q` the host** (CLAP plugins are cached).

## Out of scope (small follow-up, not T12)

**Speed/length pickers in TrackHeader** (`SetTrackLength` /
`SetTrackSpeedRatio`) — deferred from T11. A trivial separate task; optional
warmup or skip. Also latent (flagged `TODO(T11)` in `engine.rs`): non-1.0
`speed_ratio` mis-spaces ratchet/swing/micro_timing (no editor sets it yet) —
surfaces if/when those pickers ship.

## Task — T12 (in order; each has its own verification)

1. **AppMode toggle** — Editing↔Performance, stored in `ctx.data` (like `Zoom`).
   `render()` branches on it.
2. **PerformanceView (`performance.rs`)** — large PLAY/STOP (`transport_action`
   → `Play`/`Stop`), 3×3 pattern grid (cell state EMPTY/FILLED/PLAYING/QUEUED
   from `patterns`/`active_pattern_index`/`queued_pattern`; PLAYING/QUEUED glow
   via `pattern_loop_count` or `ctx.time()`), track activity LEDs (from
   `playheads`) + mute toggles (`SetTrackMuted`), quantize selector
   (`SetQuantizeGrain`). Gestures → `QueuePattern`/`CancelQueuedPattern`/
   `RetriggerPattern`.
3. **PatternOptionsSheet (`pattern_options.rs`)** — `Area` + `Frame::popup` +
   open-frame guard; per-pattern follow-action → `SetFollowAction`.
4. **Headless tests** (`test_support` harness): pattern-cell click →
   `QueuePattern`; current/queued/playing highlight; quantize →
   `SetQuantizeGrain`; mute toggle; follow-action → `SetFollowAction`; mode
   toggle; no-panic (no session). **Replay real event ORDER for ≥1 test**
   (mirror-glue lesson).
5. **Full verify:** `cargo test` (workspace) · `cargo clippy --all-targets -- -D
   warnings` · `cargo fmt` · `cargo check -p sequencer_engine --target
   aarch64-apple-ios` (rustup PATH). All green.
6. **Bundle + DAW smoke (Bitwig):** `bash engine/scripts/install_clap.sh`
   **from the T12 worktree**; `Cmd+Q` + relaunch. Assert: pattern switch
   audible; queued→switched timing; follow-action triggers; mutes; quantize.
   Record host + build sha in the PR body. If smoke finds a bug headless missed
   → TDD a failing test first, fix, **before** flipping the SPEC.
7. **Flip SPEC** `T12|.|` → `T12|x|` when smoke passes.

## Verification — must pass before PR

```bash
export PATH="$HOME/.cargo/bin:$PATH"   # rustup, not Homebrew
cd engine
cargo test                                 # workspace green
cargo clippy --all-targets -- -D warnings  # clean
cargo fmt
cargo check -p sequencer_engine --target aarch64-apple-ios   # V5
test -d target/bundled/stepforge_clap.clap && echo "bundle present"
# + manual DAW smoke (step 6) — host + sha in PR body
```

And `grep -nE '^T12\|' SPEC.md` → ends in `|x`.

## Done-criteria (SPEC T12)

- [ ] AppMode toggle (Editing↔Performance); `render()` branches.
- [ ] PerformanceView: large play/stop, 3×3 pattern grid (EMPTY/FILLED/PLAYING/
      QUEUED + glow), track LEDs + mutes, quantize selector →
      `QueuePattern`/`SetQuantizeGrain`/etc.
- [ ] PatternOptionsSheet: follow-action → `SetFollowAction`; Area+Frame::popup;
      open-frame guard; Esc/outside dismiss.
- [ ] Headless tests for every interaction + ≥1 real-event-ORDER replay +
      no-panic.
- [ ] `cargo test` (workspace) green; `clippy -D warnings` clean; iOS guard
      green.
- [ ] In-DAW smoke passes (pattern switch audible; queued timing; follow-action;
      mutes; quantize). Host + build in PR body.
- [ ] `SPEC.md §T`: `T12` → `x`.

## Branch / PR

Branch off **`main`** (`bb8e5ee`). Suggested name `feat/clap-phase3-performanceview`.
Editor-only — no engine change, no new `Command`/`EngineEvent`. The only spec
mutation is `T12 .→x`. Work in a worktree (the user runs concurrent sessions in
the shared tree — a prior 2-session tangle doubled the T11 Undo fix; isolate +
re-verify HEAD ancestry between dispatches). Follow `/add-feature`.

## Open questions to resolve or ask

- **AppMode toggle placement** — transport toggle, dedicated mode bar, or a key?
  (Read `AppMode.swift`.)
- **3×3 grid cell-state mapping** — confirm `PATTERN_SLOTS` count + which fields
  drive EMPTY/FILLED/PLAYING/QUEUED.
- **PLAYING/QUEUED glow** — time-driven glow needs wall-clock (`ctx.time()`);
  or drive it off `pattern_loop_count`?
- **`PatternOptionsSheet` container** — design doc says `Window`; **use `Area` +
  `Frame::popup`** (T11 lesson — Window absorbs the first click).
- **`FollowAction` model shape** — vs `FollowActionType`; `PlaySpecific(Uuid)` —
  what's the Uuid, and is follow-action stored per-`Pattern`?
- **Performance mode vs the grid** — does PerformanceView replace the grid
  entirely, or coexist with it? (iOS `AppMode` switches the whole view.)
- **Which DAW for the smoke?** Bitwig (reference, used T10f/T11) or Reaper —
  confirm access.
