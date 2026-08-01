# Handoff prompt — implement T11 (Phase 2 ActionDrawer + NotePickerSheet), CLAP egui editor

> Paste this into a fresh Claude Code session to pick up T11. It is
> self-contained: read the files it cites, then implement. Do not assume prior
> conversation context.

---

## Prerequisite

**T10a–T10f must be merged to `main` first** (Phase 1 close). The last Phase 1
task is **PR #24** (`feat/clap-phase1-close` — install helper + a ratchet
timing fix + `SPEC.md §T T10 .→x`). After #24 merges, on `origin/main`
`SPEC.md §T` rows `T10a..T10f` are all `x`; **`T11` is the next `.`**. Branch
off `main` after fetching. (If `T10f` is still `.` on your `main`, stop and
merge #24 first.)

## You are picking up

StepForge's **CLAP plugin egui editor**, Phase 2. Implement **T11 —
ActionDrawer + NotePickerSheet**: the two track-level surfaces the iOS app has
and the editor doesn't yet. This ports the iOS `ActionDrawer` (Roll/Vary/Cut/
Copy/Paste/Trash/Undo + per-action strength) and `NotePickerSheet` (GM-drums
grid + piano roll → track `midi_note`) to pure-egui, wired to **existing**
`Command` variants. No engine change, no new `Command`/`EngineEvent`, no orphans.

This is a **pure-UI port** — every algorithm (`roll`, `vary`, clipboard, undo)
already exists in `core` and is already echoed back via `FullSnapshot` /
`UndoAvailable`. You are adding two widgets + two trigger points in the track
header, then headless-testing + smoke-testing them.

## Context — read first

- **Surface:** pure-Rust `nih_plug` + `egui` editor in `engine/crates/editor_egui`
  (lib crate `stepforge_editor_egui`), wrapped by `engine/crates/clap_plugin`
  (`stepforge_clap`, pinned nih-plug rev `f36931f`). Consumes
  `engine/crates/core` (`sequencer_engine`) in-process. **No Swift, no C ABI, no
  `engine_*` entry points on this path.**
- **Spec of record:** `SPEC.md §T` (T11 row, line ~74) + `§V` invariants —
  especially **V4** (editor = pure UI, reads `UiState`, emits `Command`, never
  mutates engine; `apply`-driven, no optimistic mutation), **V1** (RT-safety —
  untouched by this task; the GUI-thread `format!`/heap is fine), **V5** (iOS
  build stays green), **V7** (no `unsafe` outside `ffi`).
- **Design doc:** `docs/superpowers/specs/2026-07-27-clap-egui-editor-design.md`
  §Widgets (line ~357): "**ActionDrawer**: Roll/Vary/Cut/Copy/Paste/Trash/Undo
  + strength slider. **NotePickerSheet** (`egui::Window`): GM-drums grid + piano
  roll → `SetTrackNote`." §Touch→mouse/keyboard (line ~365) for gesture mapping.

### iOS sources to port (read these in full — they are the reference)

- **`app/StepForge/Features/Editing/ActionDrawer.swift:1-101`** — the reference.
  `.sheet` at `.height(180)` detent, presented from the track header `…` button.
  Header row (track name + keep/dismiss + revert/undo). Two `Slider`s: **VARY
  STRENGTH** (default `0.5`), **ROLL STRENGTH** (default `0.6`), each with a live
  `%` readout. 6-button `HStack`: Vary / Roll / Copy / Cut / Paste / Clear(Trash).
  Emits (lines 33, 75-80):
  - revert → `Command::Undo { track_idx }`
  - Vary → `Command::Vary { track_idx, strength: vary_strength }`
  - Roll → `Command::Roll { track_idx, strength: roll_strength }`
  - Copy/Cut/Paste/Trash → `Command::{Copy,Cut,Paste,Trash} { track_idx }`
- **`app/StepForge/Features/Editing/NotePickerSheet.swift:1-108`** — the reference.
  `NavigationStack` + segmented `Picker` (**GM Drums** / **Piano Roll**). GM mode:
  `LazyVGrid` (adaptive ≥140pt) of the 16 GM drum names covering notes **35–50**,
  each cell shows name + MIDI number, current note highlighted. Piano-roll mode:
  horizontal `ScrollView` of keys **36–60**, black/white styling, current note
  highlighted. Tap selects + dismisses. Emits (via `TrackHeader.swift:59`):
  - `Command::SetTrackNote { track_idx, midi_note }`
- **`app/StepForge/Features/Editing/TrackHeader.swift:54-61`** — presentation
  wiring: drum-name tap → `NotePickerSheet`; `…` button → `ActionDrawer`. Also
  has speed/length menus (`SetTrackSpeedRatio` / `SetTrackLength`) — see Open
  Questions for whether those are in-scope here.

### Engine contracts (already implemented — you only emit/read)

All variants live in `engine/crates/core/src/command.rs` (lines 32–66) and are
already wired to algorithms + echoed back. **You add zero variants.**

| Command | Fields | Algorithm / effect | Invariants preserved |
|---|---|---|---|
| `SetTrackNote` | `track_idx, midi_note: u8` | sets `Track.midi_note` | `length`/`speed_ratio`/`steps` untouched |
| `Roll` | `track_idx, strength: f32` | `algorithms/roll.rs:7` — toggles `active` + `micro_timing_offset` + `velocity_zone` over 16 steps | `length`/`midi_note`/`speed_ratio` unchanged |
| `Vary` | `track_idx, strength: f32` | `algorithms/vary.rs:8` — perturbs **non-accent** steps only; accents locked; falls back to `roll` when no accents | `length`/`midi_note`/`speed_ratio` unchanged; **accents locked** |
| `Cut` | `track_idx` | `clipboard.rs:19` — copy then clear `steps` to defaults | `length`/`midi_note`/`speed_ratio` untouched |
| `Copy` | `track_idx` | `clipboard.rs:27` — snapshot `steps`/`length`/`speed_ratio` into `TrackClipboard` (**no `midi_note`**); non-mutating, **no undo push** | — |
| `Paste` | `track_idx` | `clipboard.rs:39` — write `steps`/`length`/`speed_ratio`; **`midi_note` deliberately NOT overwritten** | `midi_note` preserved |
| `Trash` | `track_idx` | `engine.rs:971` inline — `steps = [Step::default(); 16]` (no clipboard) | `length`/`midi_note`/`speed_ratio` untouched |
| `Undo` | `track_idx` | `undo.rs:50` — restore `steps`/`length`/`speed_ratio` from one-deep `TrackSnapshot` (**no `midi_note`**); OOB → `false`, no panic | `midi_note` excluded from snapshot |

**Clipboard** (`clipboard.rs`): single-track `Clipboard { track: Option<TrackClipboard> }` — no session/pattern clipboard. **Undo** (`undo.rs:30`): `Undo { slots: [Option<TrackSnapshot>; MAX_TRACKS] }` — per-track, **one-deep**, fixed array. `Copy` and `Undo` do **not** push an undo snapshot; Roll/Vary/Cut/Paste/Trash do (engine.rs:937–940). `SetTrackLength` does **not** push undo.

**Echo events** (`event.rs`): after any of the 7 ops the dispatch arm emits
`EngineEvent::FullSnapshot { session }` (large channel) + `EngineEvent::UndoAvailable { track_idx, available }` (hot channel, event.rs:65). **No per-op event** — the mirror gets the whole session via `FullSnapshot`, same as T10a–e. So: **no optimistic mutation**; buttons emit, the UI refreshes when `FullSnapshot` lands (Hard Rule 2 split, already the T10 pattern).

**Strength is NOT a command.** There is no `SetRollStrength`/`SetVaryStrength`.
`strength` is passed inline per `Roll`/`Vary`. The default lives in the UI
(iOS `@State`: vary `0.5`, roll `0.6`) → store it as **widget-local state in
`egui` `ctx.data` temp** (exactly like the grid's zoom / drag-accumulator /
ratchet-target, see `grid.rs` `read_grid`/`write_grid`), NOT in `UiState`.

### Editor wiring points (where T11 plugs in)

- **`engine/crates/editor_egui/src/lib.rs:39` `render()`** — the panel stack:
  `transport` → `feel` → `track_management` → error → `grid`. The ActionDrawer +
  NotePickerSheet render as **overlays after the grid** (like the ratchet
  `Area` at `grid.rs:302`), gated on a target stored in `ctx.data`.
- **`engine/crates/editor_egui/src/grid.rs:545` `header()`** — the pinned track
  header. Its doc (line 546) literally says *"Name/note/speed/length pickers are
  read-only here (land in T10e/T11)."* Today: mute toggle (→ `SetTrackMuted`) +
  `drum_name(track.midi_note)` shown read-only with a `NOTE {n}` hover tooltip
  (line 584). **T11 makes the drum name clickable → opens `NotePickerSheet`**,
  and **adds a `…` button → opens `ActionDrawer`**. `drum_name()` (line 205) is
  reusable for the GM grid highlight.
- **`engine/crates/editor_egui/src/ui_state.rs`** — what you read:
  - `undo_available: HashSet<usize>` (line 45) — drives the ActionDrawer
    **Undo button's enabled state** (`undo_available.contains(&track_idx)`).
    Populated by `apply` on `UndoAvailable` (lines 159–166).
  - `tracks() -> &[Track]` (line 221) + `Track.midi_note` — drives NotePicker
    current-note highlight.
- **New modules:** create `engine/crates/editor_egui/src/action_drawer.rs` and
  `note_picker.rs`; add `pub mod` lines in `lib.rs:3-7` (mirrors `feel`/`grid`/
  `transport`/`track_management`).
- **`CommandSink`** (`lib.rs:14`) + the `RecordingSink` test harness
  (`lib.rs:71-77`) — emit `Command`s through `sink.push(...)`, assert via
  `RecordingSink` in tests. This is the established T10 pattern.

### Conventions (CLAUDE.md hard rules + working agreement)

- **No orphans.** This task adds zero `Command`/`EngineEvent` variants and zero
  engine change — the `/add-feature` symmetric-agreement is satisfied trivially.
  No C-ABI round-trip test needed (no new variant to round-trip).
- **V4 / Hard Rule 2.** Widgets read `UiState`, emit `Command`s, never mutate
  engine state. No optimistic mutation — wait for `FullSnapshot`.
- **RT path untouched.** All work is GUI-thread. `format!`/`Vec`/heap are fine
  here (V1 only constrains the RT path). No `unsafe` (V7). iOS build unaffected
  (V5) — run the guard regardless.
- **Headless gesture tests** use the `#[cfg(test)]` cell-rect / response-rect
  exposure pattern from `grid.rs`/`transport.rs` (record the `Response::rect`
  into `ctx.data`, then the click harness presses+releases at the rect center).
  See `grid.rs` ratchet-popover button rects + `transport.rs` zoom-radio rects.

### Known traps (do NOT chase these as your bugs)

- **`clap-validator` crashes on state/param tests** — upstream nih-plug
  unbounded alloc + validator div-by-zero on zero-param. Headless `cargo test`
  is the source of truth, not clap-validator.
- **nih-plug pinned at rev `f36931f`** — do not bump in this task.
- **⚠ E3 — `Step.micro_timing_offset` is set by `Roll` but NEVER read by
  dispatch** (`docs/specs/amendments.md` E3). So `Roll`'s *timing* perturbation
  is currently inaudible — only the `active` toggles + `velocity_zone` changes
  are heard. This is the **exact analog of the T10f ratchet bug** (a dispatch
  field that's written but ignored). Expect the in-DAW smoke to surface "Roll's
  timing doesn't change." Decision in Open Questions: fix it TDD-style during
  T11 (read it in `process()` like the ratchet spread fix) or document + defer.
  Do **not** treat it as a regression you introduced.
- **DAW-smoke-only bugs.** Per the T10f ratchet lesson: headless tests assert
  dispatch *count/structure*, not audio *timing*. A green headless suite ≠
  audibly correct. Budget a real-host smoke for T11 (Roll/Vary/Paste/Undo must
  *sound* right, not just pass tests).

## Task — T11

Do in order; each has its own verification.

### 1. TrackHeader trigger points (`grid.rs:545 header()`)

- Make the drum-name label **clickable** → opens `NotePickerSheet` for `track_idx`
  (store target in `ctx.data`, same id pattern as `ratchet_target`).
- Add a **`…` button** in the header → opens `ActionDrawer` for `track_idx`.
- Keep mute toggle + existing read-only speed/length as-is (see Open Questions
  for speed/length scope).

### 2. NotePickerSheet (`note_picker.rs`)

- `egui::Window` (design doc mandates it), titled e.g. "Track N — note".
- Segmented toggle: **GM Drums** / **Piano Roll** (widget-local mode in
  `ctx.data`, like grid `Zoom`).
- GM Drums: grid of the 16 names covering notes **35–50** (copy names from
  `NotePickerSheet.swift`). Highlight `tracks()[track_idx].midi_note`. Tap →
  `Command::SetTrackNote { track_idx, midi_note }` + close.
- Piano Roll: horizontal keys **36–60**, black/white styling, current-note
  highlight. Tap → same command + close.
- Close on select, on `Esc`, or on outside-click (reuse the ratchet-popover
  dismiss logic, `grid.rs:638-649`).

### 3. ActionDrawer (`action_drawer.rs`)

- `egui::Window` or `egui::Area` (see Open Questions), titled with the track's
  drum name.
- Two `Slider`s: VARY strength (default `0.5`), ROLL strength (default `0.6`),
  `0.0..=1.0`, each with a live `%` readout. State in `ctx.data` temp
  (**per-track** key, e.g. `(track_idx, "roll_str")`), NOT `UiState`.
- 6 buttons (match iOS order/labels): **Vary / Roll / Copy / Cut / Paste / Clear**
  → emit the corresponding `Command` (Vary/Roll carry the slider's current
  strength). Keep the drawer open after Copy/Cut/Paste (clipboard ops); Close on
  Clear/Trash + Undo, or keep iOS parity (iOS dismisses on Clear/Undo only —
  match it).
- **Undo (revert)** button: enabled iff `ui_state.undo_available.contains(&track_idx)`
  → `Command::Undo { track_idx }`.
- **Keep (dismiss)**: closes without emitting.

### 4. Wire overlays into `render()` (`lib.rs:39`)

After `grid::render_step_grid`, render whichever overlay target is set in
`ctx.data` (drawer target, note-picker target) — mirror how `grid.rs:302`
renders the ratchet `Area` last. Two targets can't be open at once (opening one
closes the other).

### 5. Headless tests (the established T10 pattern)

Add to the relevant module test blocks (or `lib.rs`):
- TrackHeader: drum-name click opens NotePicker; `…` click opens ActionDrawer.
- NotePicker: GM cell click → exactly `Command::SetTrackNote { .. }` with the
  right `midi_note`; current-note cell highlighted; Esc / outside-click dismisses.
- ActionDrawer: each of the 6 buttons emits the right `Command`; **Vary/Roll
  carry the slider's current strength** (drag slider → click → assert strength
  field); **Undo button disabled when `!undo_available.contains(track_idx)`**
  (seed `UiState.undo_available` in the test) and enabled when it is; dismiss
  emits nothing.
- No-panic with empty/no-session `UiState` (overlay closed).
- `render_does_not_panic` in `lib.rs` stays green with the new overlays.

### 6. Full test + lint suite

```bash
export PATH="$HOME/.cargo/bin:$PATH"   # rustup toolchain, not Homebrew rust
cd engine
cargo test -p stepforge_editor_egui        # editor UI tests (headless)
cargo test                                 # whole workspace (core + ffi + editor + clap + xtask)
cargo clippy --all-targets -- -D warnings  # zero warnings
cargo fmt
```
All green. Workspace `cargo test` includes the C-ABI round-trip safety tests
in `ffi` — they stay green though this task touches no FFI (regression guard).
The Roll/Vary **proptest** invariants (`algorithms/{roll,vary}.rs`) must stay
green — you are not changing the algorithms, only wiring UI.

### 7. iOS guard (V5)

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo check -p sequencer_engine --target aarch64-apple-ios
```
Must stay green (desktop crates must not contaminate iOS).

### 8. Bundle + in-DAW smoke (Bitwig or Reaper)

```bash
export PATH="$HOME/.cargo/bin:$PATH"
bash engine/scripts/install_clap.sh   # bundles --release + ditto-installs + xattr strip
```
Then load in a host and assert (manual — nih_plug `Buffer`/`Transport` are
unconstructable downstream, same wall as T6m/T10f):
- [ ] **NotePicker** changes the track's audible drum (pick a different GM note
      → the step hits play the new instrument).
- [ ] **Roll** audibly reshuffles the pattern (you'll hear active/velocity
      changes; the `micro_timing_offset` timing is the E3 caveat — see Open
      Questions).
- [ ] **Vary** perturbs the pattern but **accented hits stay locked**.
- [ ] **Cut → Paste** round-trips the pattern (Cut clears the source; Paste on
      another track restores `steps`/`length`/`speed_ratio` but **keeps that
      track's own `midi_note`**).
- [ ] **Undo** reverts the last Roll/Vary/Cut/Paste/Trash on that track.
- [ ] **Trash** clears the track; Undo restores.
- [ ] ActionDrawer + NotePickerSheet open/close cleanly; no frozen controls.

Record host + build in the PR body (e.g. "Bitwig 5.x, release bundle from
`<sha>`"). If the smoke surfaces a real bug headless missed → debug it
(`systematic-debugging`, TDD a failing test first) **before** flipping the SPEC.

### 9. Flip the SPEC

When all of the above pass, in `SPEC.md §T`: `T11|.|` → `T11|x`. Commit with
the port work (or as the final commit on the branch).

## Verification — must pass before PR

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd engine
cargo test                                 # workspace green
cargo clippy --all-targets -- -D warnings  # clean
cargo check -p sequencer_engine --target aarch64-apple-ios   # V5 green
test -d target/bundled/stepforge_clap.clap && echo "bundle present"
# + manual DAW smoke (step 8) — record host + build in PR body
```
And `grep -nE '^T11\|' SPEC.md` → ends in `|x`.

## Done-criteria (SPEC T11)

- [ ] TrackHeader drum-name tap → `NotePickerSheet`; `…` → `ActionDrawer`.
- [ ] NotePickerSheet: GM-drums grid (35–50) + piano roll (36–60) →
      `SetTrackNote`; current-note highlight; Esc/outside dismiss.
- [ ] ActionDrawer: Vary/Roll/Copy/Cut/Paste/Clear + Undo; Vary/Roll carry
      slider strength; Undo gated on `undo_available`.
- [ ] Strength sliders widget-local (`ctx.data`), defaults vary `0.5` / roll `0.6`.
- [ ] Headless gesture tests for every button + the disabled-Undo floor + dismiss.
- [ ] `cargo test` (workspace) green; `clippy -D warnings` clean; iOS guard green.
- [ ] In-DAW smoke passes (NotePicker audible; Roll reshuffles; Vary locks
      accents; Cut/Paste round-trip keeps `midi_note`; Undo reverts). Host +
      build in PR body.
- [ ] `SPEC.md §T`: `T11` → `x`.

## Branch / PR

Branch off **`main`** (after fetching the #24 merge), PR → `main`. Suggested
name `feat/clap-phase2-actiondrawer` (matches `feat/clap-{feelbar,
trackmanagement,phase1-close}`). Editor-only — no engine change, no
`Command`/`EngineEvent` variants, no orphans. The only spec mutation is the
`T11 .→x` flip. Follow the `/add-feature` discipline from `CLAUDE.md`.

## Open questions to resolve or ask

- **ActionDrawer container: `egui::Window` vs `egui::Area`?** Design doc says
  `NotePickerSheet` is a `Window`; it doesn't pin `ActionDrawer`. The ratchet
  popover uses `Area` (Order::Foreground). A `Window` (title bar, movable=false,
  `collapsible=false`) is the closer analog to the iOS `.sheet` detent and is
  probably right for both. Decide + stay consistent.
- **⚠ E3 — Roll's `micro_timing_offset` is dead in dispatch.** Two options:
  (a) **Fix during T11** (TDD: write a failing dispatch test asserting
  `micro_timing_offset` shifts `send_at_offset_micros`, then read it in
  `process()` — same shape as the T10f ratchet-spread fix). This makes Roll
  audibly timing-rich. (b) **Document + defer** (note it in the PR body + leave
  E3 in `amendments.md`, fix in a follow-up). Option (a) is more satisfying and
  fits the just-learned ratchet lesson; option (b) keeps T11 editor-scoped. Pick
  one with your reviewer before smoke — don't silently flip `T11|x` over a
  known-inaudible Roll timing.
- **Speed / length pickers in TrackHeader?** iOS `TrackHeader` has
  `SetTrackSpeedRatio` / `SetTrackLength` menus. The SPEC T11 row names only
  ActionDrawer + NotePickerSheet. Recommendation: **defer speed/length to a
  small follow-up task** (or T12) to keep T11 scoped — but confirm with
  reviewer. If you do include them, they're trivial (`SetTrackLength` clamps to
  `1..=STEP_COUNT`; `speed_ratio` to the engine's range).
- **ActionDrawer dismiss parity.** iOS dismisses on Clear(Trash) + Undo but not
  on Copy/Cut/Paste. Match that exactly, or close on every action? Match iOS
  unless your reviewer prefers always-close.
- **Which DAW for the smoke?** Bitwig (reference, used T6m/T10f) or Reaper.
  Confirm access; reuse `engine/scripts/install_clap.sh` to stage the bundle.
