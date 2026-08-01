# CLAP Grid Review Polish — PR #16 re-review nits (N1–N4) + N5 scroll-zoom drop

- **Date:** 2026-08-01
- **Branch:** `fix/clap-grid-review-polish` (off `main`, after #16 + #15 merged)
- **Status:** **EXECUTED** — all four nits + N5 (scroll-zoom drop) addressed; verified independently.

## Context

The re-review of PR #16 (T10b step-grid widget) returned **LGTM**. Four non-blocking nits were
surfaced (N1 coverage, N2 per-frame alloc, N3 popover-anchor staleness, N4 header clip). None
blocked merge. #16 and #15 (T10c TransportBar) were both merged to `main` on 2026-07-31, so these
nits are addressed where the code *persists* in the final tree, not on the about-to-merge #16.

Key constraint driving the landing decision: #15 already (a) removed #16's in-grid zoom toolbar,
(b) relocated the toggle to `TransportBar` with its own test, (c) edited `grid.rs`
(palette → `pub(crate)`, dropped `Zoom::toggle`). The header and popover regions touched by N3/N4
are byte-identical across #16 → T10c, so the fixes apply cleanly.

## Landing

Dedicated follow-up branch off `main`, merged after #16 and #15 — keeps both in-flight PRs clean.
Alternative considered (fold into #15/T10c) rejected: would muddy #15's TransportBar scope and
re-trigger its review.

## Per-issue

### N1 — in-grid toolbar zoom toggle untested → **NO ACTION** (resolved by #15)

#16's `ui.toggle_value` toolbar (the primary visible zoom control) had no direct test. #15 removes
that toolbar and relocates the toggle to `TransportBar`, already covered by
`transport_zoom_toggle_writes_shared_grid_state` (both radios click-tested). The final tree has
zoom-toggle coverage; no code change.

### N2 — per-frame `format!` allocs → **documented + closed**

- `format!("NOTE {}", midi_note)` (header, ~4×/frame): closed by N4 — moved to `on_hover_ui`, so
  the `format!` runs only while the pointer hovers the drum-name, not every frame.
- `format!("Ratchet · step {}", step_idx+1)` (popover title): left in place, documented in code as
  GUI-thread + open-only — V4 permits GUI-thread allocs (only the RT path must be alloc-free).
  No `thread_local!` scratch buffer warranted.

### N3 — popover anchor goes stale on horizontal scroll → **FIXED** (commit `4513e3a`)

Root cause: `ratchet_pos` was an absolute screen `Pos2` captured at Alt-click, never refreshed —
scrolling the grid left the popover floating at the old spot.

Fix in `engine/crates/editor_egui/src/grid.rs`:
- Drop `GridUiState.ratchet_pos`; derive the anchor from a `cell_rects` lookup of the open
  `(track, step)` target each frame. Cells render before the popover, so the target's live rect is
  recorded by popover-render time.
- Ungate `cell_rects_id` + its clear so prod records rects; gate the per-cell push behind
  `cfg!(test) || popover_open || just_opened` — zero recording work (and no per-cell data lock) on
  the common path.
- `handle_cell_gestures` returns `just_opened` so the Alt-clicked cell records its rect the same
  frame it opens the popover (no 1-frame flash to the fallback anchor).

### N4 — header text may clip in `CELL_H = 34` → **FIXED** (commit `7f1b71a`)

`header()` stacked drum-name (`.strong()`) + `NOTE n` (`Small`) in a fixed 34px allocation. Row
alignment was always safe (both columns advance `CELL_H + ROW_SPACING`), but the two labels could
visually clip.

Fix: drop the always-on `NOTE n` label; attach the raw MIDI number as a hover tooltip
(`name_resp.on_hover_ui`) on the drum-name label. One line in the 34px allocation → no clip risk,
and the per-frame NOTE alloc is gone (closes N2). `drum_name` stays a zero-alloc `&'static str`.

Applied unconditionally (not verify-first): strictly an improvement, the note stays reachable on
hover. In-host eyeball still welcome.

### N5 — scroll-wheel zoom is non-functional in a real DAW → **FIXED** (drop scroll-zoom)

Follow-up to **F2** (the original PR #16 review's scroll-zoom finding), surfaced by in-DAW testing
rather than the re-review. F2 was addressed in `cd6f73e` by reading `i.zoom_delta()`; the headless
test `grid_scroll_zoom_requires_command_modifier` passed. A real-DAW test (2026-08-01) proved it
broken: **Cmd+scroll never flipped the grid 8↔16** — 16 cells in both states, just physically
scaled, then overflowing the canvas.

Root cause: `zoom_delta()` returns `zoom_factor_delta` (`input_state/mod.rs:345`, fed by
command-modified `MouseWheel`), which is the *same* signal egui applies to global
`pixels_per_point`. In-host the grid never flipped ⇒ egui received no zoom event
(`zoom_delta() == 1.0`) ⇒ the visible scaling was the **DAW scaling the plugin window** (hosts
claim Cmd+scroll for their own timeline zoom; the plugin sees nothing). The headless test was a
**false positive** — it injects a synthetic `Event::MouseWheel { modifiers: COMMAND }` the host
never delivers.

Wheel-zoom is unsalvageable for a plugin editor: no-op (host ate the gesture) or whole-editor
scale. Resolution = drop it.

Fix in `engine/crates/editor_egui/src/grid.rs` (+ `transport.rs` doc):
- `apply_zoom_input`: removed the `zoom_delta()` read. Grid zoom now follows only the
  TransportBar radios + the `1`/`2` keys (keyboard events hosts don't eat).
- Removed the two false-positive tests (`grid_scroll_zoom_requires_command_modifier`,
  `grid_scroll_plain_wheel_does_not_zoom`) + their now-dead helpers (`scroll`, `scroll_with`,
  `Harness::zoom`).
- Updated doc comments to state the new contract.

Memory corrected: the prior `stepforge-clap-egui-port` note taught "cmd+scroll zoom must read
`zoom_delta()`" — rewritten to the in-host truth.

## Verification (run 2026-08-01, this branch)

- `cargo test -p stepforge_editor_egui` → **42 passed; 0 failed**.
- `cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings` → **clean**.
- `cargo check -p stepforge_editor_egui --target aarch64-apple-ios` → **green** (V5, no iOS-guard regression).
- N3 manual (popover follows its cell under horizontal scroll): in-host only — no standalone runner
  exists for `editor_egui`; existing headless tests still cover open/dismiss/commit.
- N4 manual (header clip): blind-applied; in-host eyeball deferred.
- N5 (2026-08-01): scroll-zoom dropped → **41 passed; 0 failed** (−2 false-positive scroll tests
  removed), clippy `-D warnings` clean. In-host `1`/`2`-key zoom re-test pending.

## Commits

- `4513e3a` — fix(clap): ratchet popover tracks its cell under scroll (N3) [+ documents N2 popover-title alloc]
- `7f1b71a` — fix(clap): move header NOTE number to hover tooltip (N4) [+ fully closes N2]
- N5 — fix(clap): drop scroll-wheel grid zoom (this plan-doc commit's companion code change on
  `fix/clap-grid-review-polish`; lands via PR #18).

## Sequencing outcome

1. ✅ Merge #16 (T10b step-grid) — merged 2026-07-31 (`42d0e00`).
2. ✅ Merge #15 (T10c TransportBar) — merged 2026-07-31 (`8940535`).
3. ✅ Branch `fix/clap-grid-review-polish` off `main`; apply N3 + N4 (N2 folded in, N1 no-op); verify.
4. ✅ PR #18 open (`fix/clap-grid-review-polish` → `main`). Title predates the N4 commit (says "N3/N2"); branch + body carry N3 + N4 + N2-doc.
