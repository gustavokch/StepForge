# T13 Editor Slice — CLAP SettingsSheet + Theme/Typography Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a CLAP egui `SettingsSheet` (T13a) and port the iOS theme/typography token system into the editor (T13b).

**Architecture:** Editor-only work in `stepforge_editor_egui`. T13a clones the `pattern_options.rs` overlay template into a new `settings.rs` (Area + `Frame::popup`, shared `overlay::should_dismiss` guard, `ctx.data` temp state), triggered by a gear button in `TransportBar`; one writable control (Global MIDI Channel) + read-only status; sync stays read-only. T13b creates `theme.rs` (iOS `Theme.swift` tokens) + `typography.rs` (7-role type-scale), wires `apply_theme`, and migrates the 9 widget files off the inline `grid.rs` palette. No `core`/`ffi`/header/Command/EngineEvent change.

**Tech Stack:** Rust, egui 0.31.1, `sequencer_engine` core (`#![forbid(unsafe_code)]`), `proptest` (not needed here — no algorithm change). `nih_plug` pinned at git rev `f36931f` (untouched).

## Global Constraints

- **Editor-only.** No `engine/crates/core`, no `engine/crates/ffi`, no `engine/include/sequencer_engine.h` change. The header stays byte-identical: `git diff --exit-code -- engine/include/sequencer_engine.h`.
- **No new `Command` / `EngineEvent`.** `Command::SetGlobalMidiChannel { channel: u8 }` (`command.rs:100`) and `SetSyncSource { source: SyncSource }` (`command.rs:87`) already exist. The sheet emits the former; it only *displays* the latter (host owns transport — `clap_plugin/src/transport.rs:5-22`).
- **No `SESSION_FORMAT_VERSION` bump** (no serialized model change).
- **RT thread untouched.** Every change is editor UI off-RT. `#![forbid(unsafe_code)]` preserved (`editor_egui` already allows none).
- **Run cargo from `engine/`.** `cd engine && cargo ...`. iOS guard needs the rustup toolchain (Homebrew `rustc` cannot cross): `PATH="$HOME/.cargo/bin:$PATH" cargo check -p sequencer_engine --target aarch64-apple-ios`. Host clippy/test is blind to `#[cfg(target_os="ios")]`.
- **egui 0.31 quirks (adapt if an exact call differs):** `ComboBox::show_ui` returns `InnerResponse` (use `.response`); a `Window` title-bar eats the first click (use `Area` + `Frame::popup`); floating `Area`s need ~4 settle frames before cold-open widgets click; `CornerRadius` fields are `u8`; `ctx.style_mut(|s| …)` mutates `Style` in place; `TextStyle::Name(String::from("X"))` adds a named style.
- **Concurrent-session hazard:** work happens in the worktree `.claude/worktrees/t13-editor-slice` (branch `feat/t13-editor-slice`, off `origin/main` `ef9e0e0`). Re-verify HEAD ancestry against `origin/main` before each subagent dispatch.

## File Structure

**T13a (tasks 1-4):**
- CREATE `engine/crates/editor_egui/src/settings.rs` — the overlay (state, open/close, render, MIDI-channel pure helper).
- MODIFY `engine/crates/editor_egui/src/lib.rs` — `pub mod settings;` + `settings::render_settings(...)` call in the tick loop.
- MODIFY `engine/crates/editor_egui/src/ui_state.rs` — `global_midi_channel()` accessor.
- MODIFY `engine/crates/editor_egui/src/transport.rs` — gear button.
- MODIFY `engine/crates/editor_egui/src/{note_picker,action_drawer,pattern_options}.rs` — `settings::close(ctx)` in each `open()` (mutual exclusion).

**T13b (tasks 5-7):**
- CREATE `engine/crates/editor_egui/src/theme.rs` — palette + `Spacing` + `Radius` tokens (iOS port).
- CREATE `engine/crates/editor_egui/src/typography.rs` — 7-role type-scale + helper fns.
- MODIFY `engine/crates/editor_egui/src/lib.rs` — `apply_theme` body (spacing/radius/text_styles) + `pub mod theme;` `pub mod typography;`.
- MODIFY `engine/crates/editor_egui/src/grid.rs` — remove the 9 inline palette consts; `CORNER` uses `theme::Radius::SM`.
- MODIFY 9 widget files — import migration `crate::grid::` → `crate::theme::` (rename `SURFACE_HIGH` → `SURFACE_HIGHEST`), tokenize `Color32::BLACK`/`WHITE`/`LIGHT_RED` literals.

---

### Task 1: `settings.rs` overlay scaffold (state + lifecycle + dismiss)

Clone the `pattern_options.rs` overlay shell into `settings.rs`. State is target-free (`open: bool` + `opened_at`), mode-agnostic. No body content yet (Task 3 adds it); no sibling-closing yet (Task 4 adds mutual exclusion).

**Files:**
- Create: `engine/crates/editor_egui/src/settings.rs`
- Modify: `engine/crates/editor_egui/src/lib.rs:3-14` (mod decl), `lib.rs:141-147` (render call)
- Test: `engine/crates/editor_egui/src/settings.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub(crate) fn open(ctx: &Context)`, `pub(crate) fn close(ctx: &Context)`, `pub(crate) fn render_settings(ctx: &Context, ui_state: &UiState, sink: &impl CommandSink)`, `pub(crate) fn read(ctx: &Context) -> SettingsState`.

- [ ] **Step 1: Write the failing tests**

Create `engine/crates/editor_egui/src/settings.rs` with ONLY the test module (the functions it references do not exist yet → compile fail):

```rust
//! Phase 4 §T T13a — `SettingsSheet` overlay. Port of the iOS
//! `app/StepForge/Features/Settings/SettingsSheet.swift` (Session section only;
//! MIDI Routing is host-owned, sync is read-only — see the design spec).
//!
//! Like the T11/T12 overlays it is a floating `egui::Area` + `Frame::popup`
//! (NOT `egui::Window`: a Window's title-bar absorbs the first click), with the
//! shared [`crate::overlay::should_dismiss`] + open-frame guard. Target-free
//! and mode-agnostic (no per-track/per-pattern index): presence in `ctx.data`
//! temp = open. Mutual exclusion with the three track/pattern overlays is added
//! in Task 4.

use egui::{Context, Id, Pos2, RichText};
#[cfg(test)]
use egui::Rect;

use crate::grid::{SURFACE_HIGH, TEXT_PRIMARY};
use crate::{CommandSink, UiState};

fn settings_id() -> Id {
    Id::new("stepforge.settings")
}
#[cfg(test)]
fn done_rect_id() -> Id {
    Id::new("stepforge.settings.done")
}
#[cfg(test)]
fn window_rect_id() -> Id {
    Id::new("stepforge.settings.window")
}

/// Widget-local state. Presence (`open == true`) in `ctx.data` temp = open.
/// `opened_at` feeds the open-frame dismiss guard (same idiom as
/// `pattern_options::PatternOptionsState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SettingsState {
    pub(crate) open: bool,
    opened_at: u64,
}

pub(crate) fn read(ctx: &Context) -> SettingsState {
    ctx.data(|d| d.get_temp::<SettingsState>(settings_id()).unwrap_or_default())
}
fn write(ctx: &Context, f: impl FnOnce(&mut SettingsState)) {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(settings_id())));
}

/// Open the sheet. Records the frame for the open-frame guard. (Mutual
/// exclusion — closing the three sibling overlays — is added in Task 4.)
pub(crate) fn open(ctx: &Context) {
    let frame = crate::frame_nr(ctx);
    write(ctx, |s| {
        s.open = true;
        s.opened_at = frame;
    });
}

pub(crate) fn close(ctx: &Context) {
    write(ctx, |s| s.open = false);
}

/// Render the sheet if open. No-op (no panic) when closed. Body content
/// (status + MIDI channel) is added in Task 3.
pub(crate) fn render_settings(ctx: &Context, _ui_state: &UiState, _sink: &impl CommandSink) {
    let st = read(ctx);
    if !st.open {
        return;
    }
    #[cfg(test)]
    ctx.data_mut(|d| {
        *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = None;
        *d.get_temp_mut_or_default::<Option<Rect>>(done_rect_id()) = None;
    });

    let area = egui::Area::new(Id::new("stepforge.settings"))
        .order(egui::Order::Foreground)
        .current_pos(Pos2::new(40.0, 60.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(260.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Settings").strong().color(TEXT_PRIMARY));
                    ui.separator();
                    // Task 3 fills the Session status + MIDI channel here.
                    ui.separator();
                    let done = ui.add(
                        egui::Button::new(RichText::new("Done").color(TEXT_PRIMARY))
                            .fill(SURFACE_HIGH)
                            .min_size(egui::Vec2::new(80.0, 0.0)),
                    );
                    #[cfg(test)]
                    ctx.data_mut(|d| {
                        *d.get_temp_mut_or_default::<Option<Rect>>(done_rect_id()) = Some(done.rect)
                    });
                    if done.clicked() {
                        close(ctx);
                    }
                });
            });
        });

    let rect = area.response.rect;
    #[cfg(test)]
    ctx.data_mut(|d| *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = Some(rect));
    // `opened_at == frame_nr` only on the opening frame; suppress the
    // outside-click dismiss that frame (Esc still dismisses). Same guard as the
    // T11/T12 overlays.
    let is_open_frame = crate::frame_nr(ctx) == st.opened_at;
    if crate::overlay::should_dismiss(ctx, rect, is_open_frame) {
        close(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Harness;
    use sequencer_engine::models::Session;
    use std::sync::Arc;

    fn open_harness() -> Harness {
        let h = Harness::new(UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        });
        crate::settings::open(&h.ctx);
        h
    }

    fn done_center(ctx: &Context) -> egui::Pos2 {
        ctx.data(|d| d.get_temp::<Option<Rect>>(done_rect_id()))
            .unwrap_or_default()
            .map(|r| r.center())
            .expect("done rect recorded")
    }
    fn window_rect(ctx: &Context) -> Option<Rect> {
        ctx.data(|d| d.get_temp::<Option<Rect>>(window_rect_id()))
            .unwrap_or_default()
    }

    #[test]
    fn closed_renders_no_panic_no_emit() {
        let h = Harness::new(UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        });
        h.idle();
        assert!(!read(&h.ctx).open);
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn open_renders_area() {
        let h = open_harness();
        h.settle();
        assert!(read(&h.ctx).open);
        assert!(window_rect(&h.ctx).is_some(), "settings area must render when open");
    }

    #[test]
    fn done_dismisses() {
        let h = open_harness();
        h.settle();
        h.click_primary(done_center(&h.ctx));
        assert!(!read(&h.ctx).open);
        assert!(h.cmds().is_empty(), "Done must emit nothing");
    }

    #[test]
    fn esc_dismisses() {
        let h = open_harness();
        h.settle();
        h.press_key(egui::Key::Escape);
        assert!(!read(&h.ctx).open);
    }

    #[test]
    fn outside_click_dismisses() {
        let h = open_harness();
        h.settle();
        let outside = window_rect(&h.ctx)
            .map(|r| egui::Pos2::new(r.max.x + 25.0, r.center().y))
            .expect("window rect recorded");
        h.click_primary(outside);
        assert!(!read(&h.ctx).open);
    }
}
```

- [ ] **Step 2: Register the module + wire the render call**

In `engine/crates/editor_egui/src/lib.rs`, add `pub mod settings;` to the module list (alphabetically, after `pub mod pattern_options;` at line 8 — insert before `pub mod performance;`) and add the render call in `render` after the `pattern_options::render_pattern_options(...)` call (line 146):

```rust
        pattern_options::render_pattern_options(ui.ctx(), ui_state, sink);
        // Phase 4 §T T13a — SettingsSheet overlay (mode-agnostic; gear in the
        // TransportBar opens it). Floating `egui::Area`; no-op when closed.
        settings::render_settings(ui.ctx(), ui_state, sink);
```

- [ ] **Step 3: Run the tests — verify they pass**

Run (from `engine/`):
```bash
cargo test -p stepforge_editor_egui settings::
```
Expected: 5 tests PASS (`closed_renders_no_panic_no_emit`, `open_renders_area`, `done_dismisses`, `esc_dismisses`, `outside_click_dismisses`).

- [ ] **Step 4: Lint + format**

```bash
cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings
cargo fmt -p stepforge_editor_egui
```
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add engine/crates/editor_egui/src/settings.rs engine/crates/editor_egui/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(editor): SettingsSheet overlay scaffold (T13a)

Clone the pattern_options overlay shell into settings.rs: target-free
SettingsState (open + opened_at), open/close/read/write, Area + Frame::popup
render with Done + shared overlay::should_dismiss guard. Mode-agnostic;
wired into the render tick loop. No body content or sibling-exclusion yet.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `UiState::global_midi_channel()` accessor

One read accessor beside `bpm()` / `sync_source()`. The field already exists on `Session` (`models.rs:24`, default 10).

**Files:**
- Modify: `engine/crates/editor_egui/src/ui_state.rs` (add accessor after `sync_source()` ~line 266)
- Test: `engine/crates/editor_egui/src/ui_state.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub fn global_midi_channel(&self) -> u8` on `UiState`.
- Consumes: `Session.global_midi_channel: u8` (`models.rs:24`).

- [ ] **Step 1: Write the failing test**

In `ui_state.rs` test module, add (after `uistate_bpm_sync_accessors`-style tests — find the `state_with_session` helper at ~line 353):

```rust
    #[test]
    fn uistate_global_midi_channel_accessor() {
        // pre-snapshot default
        let mut st = UiState::default();
        assert_eq!(st.global_midi_channel(), 10);

        // with a session that overrides the channel
        let s = Session {
            global_midi_channel: 1,
            ..Default::default()
        };
        st.session = Some(Arc::new(s));
        assert_eq!(st.global_midi_channel(), 1);
    }
```

- [ ] **Step 2: Run the test — verify it fails**

```bash
cargo test -p stepforge_editor_egui uistate_global_midi_channel_accessor
```
Expected: FAIL — `no method named global_midi_channel found`.

- [ ] **Step 3: Implement the accessor**

In `ui_state.rs`, immediately after the `sync_source()` accessor (after line 266):

```rust
    /// Authoritative global MIDI output channel `[1, 16]` (snapshot). `10`
    /// before the first snapshot — matches `Session::default().global_midi_channel`
    /// (GM drums). Read by the SettingsSheet (T13a); edits become
    /// `Command::SetGlobalMidiChannel` and the engine echoes a `FullSnapshot`.
    pub fn global_midi_channel(&self) -> u8 {
        self.session
            .as_ref()
            .map(|s| s.global_midi_channel)
            .unwrap_or(10)
    }
```

- [ ] **Step 4: Run the test — verify it passes**

```bash
cargo test -p stepforge_editor_egui uistate_global_midi_channel_accessor
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add engine/crates/editor_egui/src/ui_state.rs
git commit -m "$(cat <<'EOF'
feat(editor): UiState.global_midi_channel accessor (T13a)

One read accessor beside bpm()/sync_source(); the field already exists on
Session (default 10, GM drums). The SettingsSheet reads it; no mirror/event
change.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: SettingsSheet body — read-only status + MIDI channel

Fill the sheet body: a read-only Session status block (BPM, Sync label, Link peers when linked) + a writable Global MIDI Channel ComboBox. The emit logic is a pure helper (headless oracle — the T11/T12 overlays test ComboBoxes via pure oracles, not by driving the dropdown). Commit-on-change (pick → emit), matching the iOS Picker.

**Files:**
- Modify: `engine/crates/editor_egui/src/settings.rs` (body + pure helper + tests)

**Interfaces:**
- Consumes: `UiState::bpm()`, `UiState::sync_source()`, `UiState::link_enabled`, `UiState::link_peers`, `UiState::global_midi_channel()` (Task 2), `crate::transport::sync_label(SyncSource) -> &'static str`.
- Produces: `pub(crate) fn midi_channel_command(current: u8, picked: u8) -> Option<Command>`.

- [ ] **Step 1: Write the failing tests**

Append to `settings.rs` test module:

```rust
    // ---- pure oracle: MIDI channel emit logic ----

    #[test]
    fn midi_channel_command_emits_only_on_change() {
        use sequencer_engine::command::Command;
        // unchanged → no emit
        assert!(midi_channel_command(10, 10).is_none());
        // changed → SetGlobalMidiChannel
        assert!(matches!(
            midi_channel_command(10, 12),
            Some(Command::SetGlobalMidiChannel { channel: 12 })
        ));
        assert!(matches!(
            midi_channel_command(1, 16),
            Some(Command::SetGlobalMidiChannel { channel: 16 })
        ));
    }

    // ---- e2e: status is read-only; sheet never emits SetSyncSource ----

    #[test]
    fn settings_status_block_emits_no_setsyncsource() {
        // Sync is host-owned → the sheet only labels it. Exercise the open
        // sheet across several frames + Done; nothing may emit SetSyncSource.
        let h = open_harness();
        h.settle();
        h.click_primary(done_center(&h.ctx));
        assert!(
            h.cmds()
                .iter()
                .all(|c| !matches!(c, sequencer_engine::command::Command::SetSyncSource { .. })),
            "sync source is read-only in the settings sheet"
        );
    }
```

- [ ] **Step 2: Run the tests — verify they fail**

```bash
cargo test -p stepforge_editor_egui settings::tests::midi_channel_command
cargo test -p stepforge_editor_egui settings::tests::settings_status_block_emits_no_setsyncsource
```
Expected: FAIL — `midi_channel_command` not defined; status block not yet present (test still passes vacuously if no SetSyncSource is emitted, but the body must render to be meaningful — implement next).

- [ ] **Step 3: Implement the body + pure helper**

In `settings.rs`:

(a) Add `ComboBox` to the egui import (line with `use egui::{...}`):
```rust
use egui::{ComboBox, Context, Id, Pos2, RichText};
```
Add `TEXT_MUTED` to the grid import:
```rust
use crate::grid::{SURFACE_HIGH, TEXT_MUTED, TEXT_PRIMARY};
```

(b) Add the pure helper above `render_settings`:
```rust
/// Pure: the command (if any) for a MIDI-channel pick. `None` when unchanged
/// (no spurious emit). Mirrors `transport::bpm_edit_command` as a headless
/// oracle. The widget reads `current` from [`UiState::global_midi_channel`]
/// each frame (commit-on-change, iOS Picker parity).
pub(crate) fn midi_channel_command(current: u8, picked: u8) -> Option<Command> {
    if picked == current {
        None
    } else {
        Some(Command::SetGlobalMidiChannel { channel: picked })
    }
}
```
Add `use sequencer_engine::command::Command;` to the imports.

(c) Replace the `// Task 3 fills ...` placeholder inside `render_settings` with the body, and change the signature to use `ui_state` + `sink`:
```rust
pub(crate) fn render_settings(ctx: &Context, ui_state: &UiState, sink: &impl CommandSink) {
    let st = read(ctx);
    if !st.open {
        return;
    }
    #[cfg(test)]
    ctx.data_mut(|d| {
        *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = None;
        *d.get_temp_mut_or_default::<Option<Rect>>(done_rect_id()) = None;
    });

    let area = egui::Area::new(Id::new("stepforge.settings"))
        .order(egui::Order::Foreground)
        .current_pos(Pos2::new(40.0, 60.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(260.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Settings").strong().color(TEXT_PRIMARY));
                    ui.separator();

                    // ---- Session (read-only status; host owns transport) ----
                    ui.label(RichText::new("Session").color(TEXT_MUTED).strong());
                    ui.label(format!("BPM: {:.0}", ui_state.bpm()));
                    ui.label(format!(
                        "Sync: {}",
                        crate::transport::sync_label(ui_state.sync_source())
                    ));
                    if ui_state.link_enabled {
                        ui.label(format!("Link: {} peers", ui_state.link_peers));
                    }

                    ui.separator();

                    // ---- Global MIDI Channel (writable; commit-on-change) ----
                    ui.label(RichText::new("MIDI Channel").color(TEXT_MUTED).strong());
                    let current = ui_state.global_midi_channel();
                    let mut sel = current;
                    ComboBox::from_id_salt("stepforge.settings.midi_channel")
                        .selected_text(format!("Ch {}", sel))
                        .show_ui(ui, |ui| {
                            for ch in 1u8..=16u8 {
                                ui.selectable_value(&mut sel, ch, format!("Ch {}", ch));
                            }
                        });
                    if let Some(cmd) = midi_channel_command(current, sel) {
                        sink.push(cmd);
                    }

                    ui.separator();
                    let done = ui.add(
                        egui::Button::new(RichText::new("Done").color(TEXT_PRIMARY))
                            .fill(SURFACE_HIGH)
                            .min_size(egui::Vec2::new(80.0, 0.0)),
                    );
                    #[cfg(test)]
                    ctx.data_mut(|d| {
                        *d.get_temp_mut_or_default::<Option<Rect>>(done_rect_id()) = Some(done.rect)
                    });
                    if done.clicked() {
                        close(ctx);
                    }
                });
            });
        });

    let rect = area.response.rect;
    #[cfg(test)]
    ctx.data_mut(|d| *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = Some(rect));
    let is_open_frame = crate::frame_nr(ctx) == st.opened_at;
    if crate::overlay::should_dismiss(ctx, rect, is_open_frame) {
        close(ctx);
    }
}
```

- [ ] **Step 4: Run the tests — verify they pass**

```bash
cargo test -p stepforge_editor_egui settings::
```
Expected: 7 tests PASS (5 from Task 1 + `midi_channel_command_emits_only_on_change` + `settings_status_block_emits_no_setsyncsource`).

- [ ] **Step 5: Lint + format + full editor suite**

```bash
cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings
cargo fmt -p stepforge_editor_egui
cargo test -p stepforge_editor_egui
```
Expected: clippy clean; full editor suite green (117+ tests).

- [ ] **Step 6: Commit**

```bash
git add engine/crates/editor_egui/src/settings.rs
git commit -m "$(cat <<'EOF'
feat(editor): SettingsSheet body — status + MIDI channel (T13a)

Read-only Session status (BPM, Sync label, Link peers when linked) + a
writable Global MIDI Channel ComboBox emitting SetGlobalMidiChannel
(commit-on-change, iOS Picker parity). midi_channel_command is a pure
oracle (same approach as the T11/T12 ComboBoxes). Sync stays read-only —
host owns transport.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Gear trigger + mutual exclusion + mode-agnostic

Add the gear button to `TransportBar`. Make exclusion symmetric: `settings::open` closes the three sibling overlays; each of their `open()` fns closes settings. The AppMode toggle must NOT close settings (mode-agnostic).

**Files:**
- Modify: `engine/crates/editor_egui/src/transport.rs` (gear button after the AppMode toggle ~line 196)
- Modify: `engine/crates/editor_egui/src/settings.rs` (`open()` closes siblings)
- Modify: `engine/crates/editor_egui/src/{note_picker,action_drawer,pattern_options}.rs` (`settings::close(ctx)` in each `open()`)
- Test: `engine/crates/editor_egui/src/settings.rs`, `engine/crates/editor_egui/src/transport.rs`

**Interfaces:**
- Consumes: `settings::open(ctx)`, `settings::close(ctx)`, `settings::read(ctx)` (Task 1).
- Produces: gear button in `render_transport_bar` (records its rect under `gear_rect_id()` for tests).

- [ ] **Step 1: Write the failing tests**

Append to `settings.rs` test module:

```rust
    #[test]
    fn gear_button_opens_settings() {
        // The gear lives in the TransportBar, rendered by the full Harness.
        let h = Harness::new(UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        });
        assert!(!read(&h.ctx).open);
        h.settle();
        let gear = crate::transport::gear_center(&h.ctx); // test-facing helper
        h.click_primary(gear);
        assert!(read(&h.ctx).open, "gear click must open the settings sheet");
    }

    #[test]
    fn opening_settings_closes_pattern_options() {
        let h = Harness::new(UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        });
        crate::write_mode(&h.ctx, crate::AppMode::Performance);
        crate::pattern_options::open(&h.ctx, 0, &h.state);
        h.settle();
        assert!(crate::pattern_options::read(&h.ctx).target.is_some());
        crate::settings::open(&h.ctx);
        assert!(
            crate::pattern_options::read(&h.ctx).target.is_none(),
            "opening settings must close pattern_options"
        );
    }

    #[test]
    fn opening_note_picker_closes_settings() {
        let h = Harness::new(UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        });
        crate::settings::open(&h.ctx);
        assert!(read(&h.ctx).open);
        crate::note_picker::open(&h.ctx, 0, &h.state);
        assert!(!read(&h.ctx).open, "opening note_picker must close settings");
    }

    #[test]
    fn settings_survives_mode_toggle() {
        // Settings is mode-agnostic: switching Editing<->Performance must NOT
        // close it (unlike the mode-bound overlays).
        let h = Harness::new(UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        });
        crate::settings::open(&h.ctx);
        h.settle();
        assert!(read(&h.ctx).open);
        // Toggle to Performance via the transport button.
        let mode_btn = crate::transport::mode_center(&h.ctx);
        h.click_primary(mode_btn);
        assert!(read(&h.ctx).open, "settings must stay open across mode toggle");
        // Toggle back.
        let mode_btn = crate::transport::mode_center(&h.ctx);
        h.click_primary(mode_btn);
        assert!(read(&h.ctx).open);
    }
```

- [ ] **Step 2: Run the tests — verify they fail**

```bash
cargo test -p stepforge_editor_egui settings::tests::gear_button_opens_settings
cargo test -p stepforge_editor_egui settings::tests::opening_settings_closes_pattern_options
cargo test -p stepforge_editor_egui settings::tests::opening_note_picker_closes_settings
cargo test -p stepforge_editor_egui settings::tests::settings_survives_mode_toggle
```
Expected: FAIL — `gear_center`/`mode_center` not found; siblings not closed; mode toggle closes settings.

- [ ] **Step 3: Add the gear button + test-facing rect helpers in `transport.rs`**

(a) Add a `gear_rect_id()` beside the other `#[cfg(test)]` rect id helpers (after `mode_rect_id()` ~line 97):
```rust
#[cfg(test)]
fn gear_rect_id() -> Id {
    Id::new("stepforge.transport.gear")
}
```

(b) Add test-facing helpers at the bottom of the `tests` module (or as `pub(crate) fn` outside it — these are called from the `settings` test module, so they must be `pub(crate)` free fns, not inside `mod tests`). Place them just above the `#[cfg(test)] mod tests` block:
```rust
/// Test-facing: center of the gear button rect (recorded each frame).
#[cfg(test)]
pub(crate) fn gear_center(ctx: &egui::Context) -> egui::Pos2 {
    ctx.data(|d| d.get_temp::<egui::Rect>(gear_rect_id()))
        .expect("gear rect recorded")
        .center()
}
/// Test-facing: center of the AppMode toggle rect.
#[cfg(test)]
pub(crate) fn mode_center(ctx: &egui::Context) -> egui::Pos2 {
    ctx.data(|d| d.get_temp::<egui::Rect>(mode_rect_id()))
        .expect("mode rect recorded")
        .center()
}
```

(c) Add the gear button at the end of the transport `ui.horizontal` block, after the AppMode-toggle `if m_resp.clicked() { … }` (after line 196, before the closing `});`):
```rust

        ui.separator();

        // Phase 4 §T T13a — Settings gear. Opens the mode-agnostic SettingsSheet
        // (a floating Area; closes the three track/pattern overlays on open).
        let s_resp = ui.button(RichText::new("⚙").color(TEXT_PRIMARY));
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(gear_rect_id(), s_resp.rect));
        if s_resp.clicked() {
            crate::settings::open(&ctx);
        }
```

- [ ] **Step 4: Make exclusion symmetric**

(a) In `settings.rs` `open()`, add the three sibling closes (after setting `open = true`):
```rust
pub(crate) fn open(ctx: &Context) {
    let frame = crate::frame_nr(ctx);
    write(ctx, |s| {
        s.open = true;
        s.opened_at = frame;
    });
    // Only one floating sheet at a time.
    crate::note_picker::close(ctx);
    crate::action_drawer::close(ctx);
    crate::pattern_options::close(ctx);
}
```

(b) In `note_picker.rs` `open()`, add `crate::settings::close(ctx);` (mirror `action_drawer::open` which already calls `crate::note_picker::close(ctx)`). Find `note_picker::open` and append the call before the closing brace.

(c) In `action_drawer.rs` `open()` (line 92-99), add `crate::settings::close(ctx);` after the existing `crate::note_picker::close(ctx);`.

(d) In `pattern_options.rs` `open()` (line 209-234), add `crate::settings::close(ctx);` at the end (after the `write(ctx, |s| { … });` block).

- [ ] **Step 5: Run the tests — verify they pass**

```bash
cargo test -p stepforge_editor_egui settings::tests
cargo test -p stepforge_editor_egui transport::tests
```
Expected: all PASS (4 new settings tests + existing transport tests green; the existing `transport_sync_badge_emits_no_setsyncsource` still passes — the gear adds no sync emit).

- [ ] **Step 6: Lint + format + full suite + iOS guard + header check**

```bash
cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings
cargo fmt -p stepforge_editor_egui
cargo test -p stepforge_editor_egui
PATH="$HOME/.cargo/bin:$PATH" cargo check -p sequencer_engine --target aarch64-apple-ios
git -C /Users/gus/Git/StepForge/.claude/worktrees/t13-editor-slice diff --exit-code -- engine/include/sequencer_engine.h
```
Expected: clippy clean; editor suite green; iOS check clean; header byte-identical (no output, exit 0).

- [ ] **Step 7: Commit**

```bash
git add engine/crates/editor_egui/src/transport.rs engine/crates/editor_egui/src/settings.rs engine/crates/editor_egui/src/note_picker.rs engine/crates/editor_egui/src/action_drawer.rs engine/crates/editor_egui/src/pattern_options.rs
git commit -m "$(cat <<'EOF'
feat(editor): Settings gear + symmetric overlay exclusion (T13a)

Gear button in the TransportBar opens the SettingsSheet. Exclusion is
symmetric: settings::open closes note_picker/action_drawer/pattern_options,
and each of their open() closes settings. Settings is mode-agnostic — the
AppMode toggle does not close it (unlike the mode-bound overlays).

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `theme.rs` — iOS token port (palette + Spacing + Radius)

Create the single source of truth for visuals, porting `app/StepForge/Theme/Theme.swift`.

**Files:**
- Create: `engine/crates/editor_egui/src/theme.rs`
- Modify: `engine/crates/editor_egui/src/lib.rs` (`pub mod theme;`)
- Test: `engine/crates/editor_egui/src/theme.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `theme::{SURFACE_*, BORDER_*, PRIMARY, PRIMARY_DIM, ON_PRIMARY, TEXT_*, ZONE_*, DANGER}` (all `Color32`), `theme::Spacing::{XS,SM,MD,LG,XL,GUTTER}` (`f32`), `theme::Radius::{SM,MD,LG}` (`u8`).

- [ ] **Step 1: Write the failing test**

Create `engine/crates/editor_egui/src/theme.rs` with the test module first:

```rust
//! Phase 4 §T T13b — design tokens. Faithful port of the iOS
//! `app/StepForge/Theme/Theme.swift` into egui `Color32` / `f32` / `u8`
//! constants. Dark-only (a light variant needs mode-aware tokens; out of
//! scope — see the spec's non-goals). Replaces the inline `grid.rs:39-50`
//! palette; imported by every widget as `crate::theme::{...}`.

use egui::Color32;

// ---- Surface (5 graphite tiers; higher elevation = lighter) ----
pub const SURFACE_LOWEST: Color32 = Color32::from_rgb(0x0E, 0x0E, 0x0E);
pub const SURFACE_LOW: Color32 = Color32::from_rgb(0x1B, 0x1B, 0x1C);
pub const SURFACE_DEFAULT: Color32 = Color32::from_rgb(0x20, 0x20, 0x20);
pub const SURFACE_HIGH: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2A);
pub const SURFACE_HIGHEST: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);

// ---- Border ----
pub const BORDER_WEAK: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x58, 0x42, 0x35);
pub const BORDER_ACCENT: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00);

// ---- Brand ----
pub const PRIMARY: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00);
pub const PRIMARY_DIM: Color32 = Color32::from_rgb(0xFF, 0xB6, 0x88);
pub const ON_PRIMARY: Color32 = Color32::from_rgb(0x23, 0x13, 0x00);

// ---- Text ----
pub const TEXT_PRIMARY: Color32 = Color32::WHITE;
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xA0, 0xA0, 0xA0);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x6E, 0x6E, 0x6E);

// ---- Velocity zones ----
pub const ZONE_ACCENT: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00);
pub const ZONE_MID: Color32 = Color32::from_rgb(0xFF, 0xB6, 0x88);
pub const ZONE_LOW: Color32 = Color32::from_rgb(0x98, 0xCB, 0xFF);

// ---- Semantic ----
/// Engine-error / danger text (replaces the stray `Color32::LIGHT_RED` in lib.rs).
pub const DANGER: Color32 = Color32::from_rgb(0xE5, 0x4B, 0x4B);

// ---- Spacing (4px grid; f32 for egui Vec2) — port of iOS `Theme.Spacing` ----
pub struct Spacing;
impl Spacing {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 16.0;
    pub const LG: f32 = 24.0;
    pub const XL: f32 = 48.0;
    pub const GUTTER: f32 = 12.0;
}

// ---- Radius (u8 for egui CornerRadius) — port of iOS `Theme.Radius` ----
pub struct Radius;
impl Radius {
    pub const SM: u8 = 4;
    pub const MD: u8 = 6;
    pub const LG: u8 = 8;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts every token equals the iOS `Theme.swift` source value (the
    /// port-fidelity guard). If iOS changes, update BOTH sides deliberately.
    #[test]
    fn palette_matches_ios_hex() {
        // Surface tiers
        assert_eq!(SURFACE_LOWEST, Color32::from_rgb(0x0E, 0x0E, 0x0E));
        assert_eq!(SURFACE_LOW, Color32::from_rgb(0x1B, 0x1B, 0x1C));
        assert_eq!(SURFACE_DEFAULT, Color32::from_rgb(0x20, 0x20, 0x20));
        assert_eq!(SURFACE_HIGH, Color32::from_rgb(0x2A, 0x2A, 0x2A));
        assert_eq!(SURFACE_HIGHEST, Color32::from_rgb(0x35, 0x35, 0x35));
        // Border
        assert_eq!(BORDER_WEAK, Color32::from_rgb(0x35, 0x35, 0x35));
        assert_eq!(BORDER_STRONG, Color32::from_rgb(0x58, 0x42, 0x35));
        assert_eq!(BORDER_ACCENT, Color32::from_rgb(0xFF, 0x7F, 0x00));
        // Brand
        assert_eq!(PRIMARY, Color32::from_rgb(0xFF, 0x7F, 0x00));
        assert_eq!(PRIMARY_DIM, Color32::from_rgb(0xFF, 0xB6, 0x88));
        assert_eq!(ON_PRIMARY, Color32::from_rgb(0x23, 0x13, 0x00));
        // Text
        assert_eq!(TEXT_PRIMARY, Color32::WHITE);
        assert_eq!(TEXT_SECONDARY, Color32::from_rgb(0xA0, 0xA0, 0xA0));
        assert_eq!(TEXT_MUTED, Color32::from_rgb(0x6E, 0x6E, 0x6E));
        // Velocity zones
        assert_eq!(ZONE_ACCENT, Color32::from_rgb(0xFF, 0x7F, 0x00));
        assert_eq!(ZONE_MID, Color32::from_rgb(0xFF, 0xB6, 0x88));
        assert_eq!(ZONE_LOW, Color32::from_rgb(0x98, 0xCB, 0xFF));
    }

    #[test]
    fn spacing_and_radius_match_ios() {
        assert_eq!(Spacing::XS, 4.0);
        assert_eq!(Spacing::SM, 8.0);
        assert_eq!(Spacing::MD, 16.0);
        assert_eq!(Spacing::LG, 24.0);
        assert_eq!(Spacing::XL, 48.0);
        assert_eq!(Spacing::GUTTER, 12.0);
        assert_eq!(Radius::SM, 4);
        assert_eq!(Radius::MD, 6);
        assert_eq!(Radius::LG, 8);
    }
}
```

- [ ] **Step 2: Register the module**

In `lib.rs`, add `pub mod theme;` (alphabetical — after `pub mod test_support` is cfg(test), so place after `pub mod track_management;` or wherever alphabetical order holds; the existing list is: action_drawer, feel, grid, note_picker, overlay, pattern_options, performance, track_management, transport, ui_state — insert `theme` between `test_support`-region and `track_management`, i.e. after `performance;`).

- [ ] **Step 3: Run the tests — verify they pass**

```bash
cargo test -p stepforge_editor_egui theme::
```
Expected: 2 tests PASS (`palette_matches_ios_hex`, `spacing_and_radius_match_ios`).

- [ ] **Step 4: Lint + format**

```bash
cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings
cargo fmt -p stepforge_editor_egui
```
Expected: clean. (Note: `pub struct Spacing;`/`Radius;` with only associated consts may trip a `dead_code` warning if unused — Task 7 uses them in `apply_theme`, so they are used by end of T13b. If clippy warns mid-task, it clears after Task 7.)

- [ ] **Step 5: Commit**

```bash
git add engine/crates/editor_egui/src/theme.rs engine/crates/editor_egui/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(editor): theme.rs — iOS Theme.swift token port (T13b)

Single source of truth for visuals: 5 graphite surface tiers, border
(weak/strong/accent), brand (primary/dim/onPrimary), text
(primary/secondary/muted), velocity zones, a DANGER semantic, a Spacing
4px scale (f32), and a Radius scale (u8). Faithful port of iOS Theme.swift;
palette_matches_ios_hex guards fidelity. Not yet wired into widgets (Task 7).

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: `typography.rs` — 7-role type-scale

Port the iOS `Typography.swift` 7 named styles into egui `FontId`s (size + family). egui default fonts have only normal/bold, so `medium → normal`, `semibold → bold` (applied per-call via `.strong()`). Provide helper fns returning `RichText`.

**Files:**
- Create: `engine/crates/editor_egui/src/typography.rs`
- Modify: `engine/crates/editor_egui/src/lib.rs` (`pub mod typography;`)
- Test: `engine/crates/editor_egui/src/typography.rs`

**Interfaces:**
- Produces: `typography::font_ids() -> impl Iterator<Item = (String, FontId)>`, `typography::{bpm_large, mono_value, step_index, track_name, control_label, section_tag, badge}(&str) -> RichText`.

- [ ] **Step 1: Write the failing test**

Create `engine/crates/editor_egui/src/typography.rs`:

```rust
//! Phase 4 §T T13b — typography type-scale. Port of iOS
//! `app/StepForge/Theme/Typography.swift`'s 7 named SF styles into egui
//! `FontId`s (size + family). egui default fonts expose only normal/bold, so
//! iOS `medium → normal` and `semibold → bold` (applied per-call via
//! `.strong()` in the helper fns). Registered as named `TextStyle`s by
//! [`crate::apply_theme`] (Task 7).

use egui::{FontFamily, FontId, RichText, TextStyle};

use crate::theme::TEXT_PRIMARY;

// ---- Role names (registered as TextStyle::Name) ----
pub const NAME_BPM_LARGE: &str = "BpmLarge";
pub const NAME_MONO_VALUE: &str = "MonoValue";
pub const NAME_STEP_INDEX: &str = "StepIndex";
pub const NAME_TRACK_NAME: &str = "TrackName";
pub const NAME_CONTROL_LABEL: &str = "ControlLabel";
pub const NAME_SECTION_TAG: &str = "SectionTag";
pub const NAME_BADGE: &str = "Badge";

// ---- Sizes (px; iOS Dynamic Type → fixed px in egui) ----
pub const BPM_LARGE_SIZE: f32 = 28.0; // iOS title2
pub const MONO_VALUE_SIZE: f32 = 13.0;
pub const STEP_INDEX_SIZE: f32 = 10.0;
pub const TRACK_NAME_SIZE: f32 = 14.0; // iOS subheadline
pub const CONTROL_LABEL_SIZE: f32 = 12.0; // iOS caption
pub const SECTION_TAG_SIZE: f32 = 11.0; // iOS caption2
pub const BADGE_SIZE: f32 = 10.0;

/// The 7 (name, FontId) pairs installed into `ctx.style().text_styles` by
/// [`crate::apply_theme`].
pub fn font_ids() -> impl Iterator<Item = (String, FontId)> {
    [
        (NAME_BPM_LARGE, FontId::new(BPM_LARGE_SIZE, FontFamily::Monospace)),
        (NAME_MONO_VALUE, FontId::new(MONO_VALUE_SIZE, FontFamily::Monospace)),
        (NAME_STEP_INDEX, FontId::new(STEP_INDEX_SIZE, FontFamily::Monospace)),
        (NAME_TRACK_NAME, FontId::new(TRACK_NAME_SIZE, FontFamily::Proportional)),
        (NAME_CONTROL_LABEL, FontId::new(CONTROL_LABEL_SIZE, FontFamily::Proportional)),
        (NAME_SECTION_TAG, FontId::new(SECTION_TAG_SIZE, FontFamily::Monospace)),
        (NAME_BADGE, FontId::new(BADGE_SIZE, FontFamily::Monospace)),
    ]
    .into_iter()
    .map(|(n, f)| (n.to_string(), f))
}

// ---- Helper fns (size + family + color; bold roles add .strong()) ----

/// Large numeric transport readout (BPM). iOS `bpmLarge` — mono, bold.
pub fn bpm_large(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_BPM_LARGE.into()))
        .strong()
        .color(TEXT_PRIMARY)
}
/// In-control numeric values. iOS `monoValue` — 13 semibold mono → bold.
pub fn mono_value(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_MONO_VALUE.into()))
        .strong()
        .color(TEXT_PRIMARY)
}
/// 1..16 column index. iOS `stepIndex` — 10 medium mono → normal.
pub fn step_index(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_STEP_INDEX.into()))
        .color(TEXT_PRIMARY)
}
/// Track / drum name. iOS `trackName` — subheadline semibold → bold.
pub fn track_name(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_TRACK_NAME.into()))
        .strong()
        .color(TEXT_PRIMARY)
}
/// Pill control labels + section headers. iOS `controlLabel` — caption medium.
pub fn control_label(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_CONTROL_LABEL.into()))
        .color(TEXT_PRIMARY)
}
/// Uppercase technical section tag. iOS `sectionTag` — caption2 semibold mono.
pub fn section_tag(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_SECTION_TAG.into()))
        .strong()
        .color(TEXT_PRIMARY)
}
/// Small chip/badge. iOS `badge` — 10 bold mono.
pub fn badge(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_BADGE.into()))
        .strong()
        .color(TEXT_PRIMARY)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 7 roles are emitted with their iOS-mapped size + family.
    #[test]
    fn font_ids_are_the_seven_ios_roles() {
        let map: std::collections::BTreeMap<String, FontId> = font_ids().collect();
        assert_eq!(map.len(), 7);
        assert_eq!(map[NAME_BPM_LARGE], FontId::new(28.0, FontFamily::Monospace));
        assert_eq!(map[NAME_MONO_VALUE], FontId::new(13.0, FontFamily::Monospace));
        assert_eq!(map[NAME_STEP_INDEX], FontId::new(10.0, FontFamily::Monospace));
        assert_eq!(map[NAME_TRACK_NAME], FontId::new(14.0, FontFamily::Proportional));
        assert_eq!(map[NAME_CONTROL_LABEL], FontId::new(12.0, FontFamily::Proportional));
        assert_eq!(map[NAME_SECTION_TAG], FontId::new(11.0, FontFamily::Monospace));
        assert_eq!(map[NAME_BADGE], FontId::new(10.0, FontFamily::Monospace));
    }
}
```

- [ ] **Step 2: Register the module**

In `lib.rs`, add `pub mod typography;` (alphabetical — after `track_management;`).

- [ ] **Step 3: Run the tests — verify they pass**

```bash
cargo test -p stepforge_editor_egui typography::
```
Expected: 1 test PASS (`font_ids_are_the_seven_ios_roles`).

- [ ] **Step 4: Lint + format**

```bash
cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings
cargo fmt -p stepforge_editor_egui
```
Expected: clean (any `dead_code` on the helper fns clears once Task 7 / follow-ups use them; if clippy blocks, add `#[allow(dead_code)]` temporarily on the unused helpers with a comment "used by T13b follow-up migration", removed in Task 7 if a call site is added there).

- [ ] **Step 5: Commit**

```bash
git add engine/crates/editor_egui/src/typography.rs engine/crates/editor_egui/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(editor): typography.rs — 7-role type-scale (T13b)

Port iOS Typography.swift into egui FontIds (size + family). egui default
fonts have only normal/bold, so medium->normal, semibold->bold (via
.strong() in the helper fns). 7 roles: BpmLarge/MonoValue/StepIndex/
TrackName/ControlLabel/SectionTag/Badge. font_ids() feeds apply_theme
(Task 7); helper fns return RichText per role.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Wire `apply_theme` + migrate widgets off the inline palette

Wire `apply_theme` to consume `theme` + `typography` (spacing/radius/text_styles). Remove the 9 inline palette consts from `grid.rs`. Migrate the 9 widget files (`crate::grid::` → `crate::theme::`, renaming `SURFACE_HIGH` → `SURFACE_HIGHEST`). Tokenize the stray `Color32::BLACK`/`WHITE`/`LIGHT_RED` literals. The compiler enforces migration completeness (`SURFACE_HIGH` no longer exists after removal → every stale reference is an error).

**Files:**
- Modify: `engine/crates/editor_egui/src/lib.rs` (`apply_theme` body)
- Modify: `engine/crates/editor_egui/src/grid.rs` (remove palette consts; `CORNER` → `theme::Radius::SM`; import palette from `theme`)
- Modify: `engine/crates/editor_egui/src/{action_drawer,feel,note_picker,pattern_options,performance,track_management,transport,settings}.rs` (import migration + `SURFACE_HIGH` → `SURFACE_HIGHEST` + literal tokenization)
- Test: `engine/crates/editor_egui/src/lib.rs` (`apply_theme` test)

**Interfaces:**
- Consumes: `theme::{Spacing, Radius, TEXT_PRIMARY, …}`, `typography::font_ids()` (Tasks 5-6).
- Produces: `apply_theme` now sets `ctx.style().spacing`, widget rounding, and 7 `text_styles`.

- [ ] **Step 1: Write the failing test**

In `lib.rs` test module, add:

```rust
    #[test]
    fn apply_theme_installs_spacing_radius_textstyles() {
        let ctx = Context::default();
        apply_theme(&ctx);
        let style = ctx.style();
        // Spacing from the shared tokens (not egui defaults).
        assert_eq!(style.spacing.item_spacing, egui::vec2(theme::Spacing::SM, theme::Spacing::XS));
        // Radius: widget rounding uses Radius::SM (fixes the old CORNER=3 drift).
        assert_eq!(style.visuals.widgets.inactive.rounding.nw, theme::Radius::SM);
        // All 7 typography roles registered as named TextStyles.
        for name in [
            typography::NAME_BPM_LARGE,
            typography::NAME_MONO_VALUE,
            typography::NAME_STEP_INDEX,
            typography::NAME_TRACK_NAME,
            typography::NAME_CONTROL_LABEL,
            typography::NAME_SECTION_TAG,
            typography::NAME_BADGE,
        ] {
            assert!(
                style.text_styles.contains_key(&egui::TextStyle::Name(name.into())),
                "TextStyle {name} must be registered"
            );
        }
    }
```

- [ ] **Step 2: Run the test — verify it fails**

```bash
cargo test -p stepforge_editor_egui apply_theme_installs_spacing_radius_textstyles
```
Expected: FAIL — `apply_theme` is still the Phase-0 stub (item_spacing / text_styles not set).

- [ ] **Step 3: Rewrite `apply_theme` in `lib.rs`**

Replace the stub `apply_theme` (lines 82-87) with:

```rust
/// Dark graphite theme — full Phase-4 palette/typography port of the iOS
/// `Theme.swift` + `Typography.swift`. Tokens live in [`theme`] +
/// [`typography`]. Widgets import `crate::theme::{...}`; `apply_theme` sets the
/// egui `Visuals` (widget rounding, override text color) + `Style` (item
/// spacing, button padding, the 7-name type-scale).
pub fn apply_theme(ctx: &Context) {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(theme::TEXT_PRIMARY);
    let r = egui::CornerRadius::same(theme::Radius::SM);
    v.widgets.noninteractive.rounding = r;
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.rounding = r;
    v.widgets.active.rounding = r;
    v.widgets.open.rounding = r;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(theme::Spacing::SM, theme::Spacing::XS);
        s.spacing.button_padding = egui::vec2(theme::Spacing::SM, theme::Spacing::XS);
        for (name, fid) in typography::font_ids() {
            s.text_styles.insert(egui::TextStyle::Name(name), fid);
        }
    });
}
```

- [ ] **Step 4: Remove the palette from `grid.rs`**

In `grid.rs`, delete the 9 `pub(crate) const` palette lines (39-50: `SURFACE_LOW`, `SURFACE_HIGH`, `PRIMARY`, `ZONE_ACCENT`, `ZONE_MID`, `ZONE_LOW`, `TEXT_PRIMARY`, `TEXT_MUTED`, `BORDER_WEAK`) and the comment above them (36-38). Replace `const CORNER: u8 = 3;` (line 65) with:

```rust
const CORNER: u8 = crate::theme::Radius::SM; // iOS Theme.Radius.sm (was 3 — drift fixed)
```

Add a `theme` import at the top of `grid.rs` for the palette names this file still uses internally (it references `SURFACE_LOW`, `SURFACE_HIGHEST`, `PRIMARY`, `ZONE_*`, `TEXT_*`, `BORDER_WEAK` in its body). After removing the local consts, add (replacing any existing palette import — `grid.rs` defined them locally before, so there is no import yet; add one):

```rust
use crate::theme::{
    BORDER_WEAK, PRIMARY, SURFACE_HIGHEST, SURFACE_LOW, TEXT_MUTED, TEXT_PRIMARY, ZONE_ACCENT,
    ZONE_LOW, ZONE_MID,
};
```

Then replace every `SURFACE_HIGH` token in the `grid.rs` body with `SURFACE_HIGHEST`. Run the compiler (Step 6) to find any missed sites.

- [ ] **Step 5: Migrate the other 8 widget files**

For each of `action_drawer.rs`, `feel.rs`, `note_picker.rs`, `pattern_options.rs`, `performance.rs`, `settings.rs`, `track_management.rs`, `transport.rs`:

1. Change the palette import line `use crate::grid::{...}` → `use crate::theme::{...}` (keep only the tokens actually used in that file).
2. Rename `SURFACE_HIGH` → `SURFACE_HIGHEST` everywhere in the file (import list + body). The other token names (`PRIMARY`, `TEXT_PRIMARY`, `TEXT_MUTED`, `BORDER_WEAK`, `ZONE_*`, `SURFACE_LOW`) are unchanged.
3. If the file imports `drum_name`, `read_grid`, `write_grid`, or `Zoom` from `crate::grid`, keep those on a SEPARATE `use crate::grid::{...}` line (those are still in `grid.rs`).

The exact current import lines (verified against `origin/main`):
- `action_drawer.rs:27` — `use crate::grid::{drum_name, TEXT_MUTED, TEXT_PRIMARY};`
- `feel.rs:26` — (read the file; split grid helpers from palette tokens)
- `note_picker.rs:23` — (read the file)
- `pattern_options.rs:25` — `use crate::grid::{PRIMARY, SURFACE_HIGH, TEXT_MUTED, TEXT_PRIMARY};`
- `performance.rs:35` — (read the file; 8 palette names per the scout)
- `settings.rs` (Task 1-4) — `use crate::grid::{SURFACE_HIGH, TEXT_MUTED, TEXT_PRIMARY};`
- `track_management.rs:23` — (read the file)
- `transport.rs:19` — `use crate::grid::{read_grid, write_grid, Zoom, PRIMARY, SURFACE_HIGH, TEXT_MUTED, TEXT_PRIMARY};` → split into `use crate::grid::{read_grid, write_grid, Zoom};` + `use crate::theme::{PRIMARY, SURFACE_HIGHEST, TEXT_MUTED, TEXT_PRIMARY};`

4. Tokenize stray literals:
   - `lib.rs:112` — `Color32::LIGHT_RED` → `crate::theme::DANGER`.
   - `note_picker.rs:268-275` — `Color32::BLACK` / `Color32::WHITE` → `crate::theme::ON_PRIMARY` (text on the orange accent fill) / `crate::theme::TEXT_PRIMARY` as appropriate to the call site (read each: BLACK fg on PRIMARY fill → `ON_PRIMARY`).
   - `grid.rs:569` — `Color32::BLACK` (the mute-button "M" text on PRIMARY fill) → `crate::theme::ON_PRIMARY`.

- [ ] **Step 6: Compile + fix every stale `SURFACE_HIGH` / palette reference**

```bash
cargo check -p stepforge_editor_egui
```
Expected: the compiler lists every remaining `SURFACE_HIGH` (or `crate::grid::TOKEN`) reference as an error. Fix each per Step 5. Repeat until `cargo check` is clean. (This is the migration-completion signal — there is no partial state.)

- [ ] **Step 7: Run the new test + the full suite**

```bash
cargo test -p stepforge_editor_egui apply_theme_installs_spacing_radius_textstyles
cargo test -p stepforge_editor_egui
```
Expected: new test PASS; full editor suite green (the import migration is mechanical; the 117+ existing tests are the regression guard — they assert structure/commands, not pixel colors, so the token revalues `TEXT_PRIMARY`→WHITE / `TEXT_MUTED`→0x6E6E6E do not break them).

- [ ] **Step 8: Lint + format + iOS guard + header check**

```bash
cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings
cargo fmt -p stepforge_editor_egui
PATH="$HOME/.cargo/bin:$PATH" cargo check -p sequencer_engine --target aarch64-apple-ios
git -C /Users/gus/Git/StepForge/.claude/worktrees/t13-editor-slice diff --exit-code -- engine/include/sequencer_engine.h
```
Expected: clippy clean; iOS check clean; header byte-identical (exit 0, no output).

- [ ] **Step 9: Commit**

```bash
git add engine/crates/editor_egui/
git commit -m "$(cat <<'EOF'
refactor(editor): wire apply_theme + migrate widgets to theme module (T13b)

apply_theme now sets ctx.style spacing (item_spacing/button_padding from
Spacing), widget rounding (Radius::SM — fixes the grid CORNER=3 drift),
and the 7-name typography type-scale. The 9 inline palette consts move
from grid.rs to theme.rs; 9 widget files import crate::theme::{...}
(SURFACE_HIGH renamed SURFACE_HIGHEST — the 0x353535 tier is iOS
'highest'; compiler-enforced). Stray BLACK/WHITE/LIGHT_RED literals
tokenized to ON_PRIMARY/TEXT_PRIMARY/DANGER.

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Final whole-branch verification (after Task 7)

- [ ] **Full workspace test + lint:**
```bash
cd engine
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```
- [ ] **iOS guard + header byte-identical + RT audit:**
```bash
PATH="$HOME/.cargo/bin:$PATH" cargo check -p sequencer_engine --target aarch64-apple-ios
git -C /Users/gus/Git/StepForge/.claude/worktrees/t13-editor-slice diff --exit-code -- engine/include/sequencer_engine.h
```
- [ ] **DAW smoke (Bitwig):** `cargo xtask bundle -p stepforge_clap --release`; load the `.clap`; open `SettingsSheet` from the gear; change MIDI channel → confirm the host sees MIDI on the new channel; confirm sync source displays read-only and emits nothing; confirm theme renders consistently across Editing/Performance; confirm the gear + sheet do not crash on rapid open/close or mode toggle.
- [ ] **SPEC §T update:** split T13 into T13a + T13b child rows (both `.` → `x`); narrow the parent T13 to the distribution remainder (VST3/codesign/CI/Live bus). Commit as `docs(spec): T13a/T13b complete`.
- [ ] **Append a T13 section to `.superpowers/sdd/progress.md`** (do NOT overwrite prior sections).

## Self-Review (run after writing — already done)

- **Spec coverage:** T13a (sheet, gear, accessor, mutual exclusion, mode-agnostic) → Tasks 1-4. T13b (theme, typography, apply_theme, migration) → Tasks 5-7. SPEC §T split → final step. All spec sections mapped.
- **Placeholder scan:** none — every step has concrete code or an exact compiler-guided instruction.
- **Type consistency:** `Command::SetGlobalMidiChannel { channel: u8 }`, `Session.global_midi_channel: u8`, `SettingsState { open: bool, opened_at: u64 }`, `midi_channel_command(u8, u8) -> Option<Command>` — consistent across tasks. `theme::Spacing`/`Radius` (assoc consts), `typography::font_ids() -> impl Iterator<Item=(String, FontId)>` — consistent.
- **One spec refinement vs. the written spec:** `settings::open(ctx)` takes no `&UiState` (commit-on-change reads live each frame — no seed needed). The spec showed `open(ctx, &UiState)`; the plan drops the unused param. Functionally identical.
