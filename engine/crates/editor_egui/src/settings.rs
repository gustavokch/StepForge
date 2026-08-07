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

#[cfg(test)]
use egui::Rect;
use egui::{ComboBox, Context, Id, Pos2, RichText};

use crate::theme::{SURFACE_HIGHEST, TEXT_MUTED, TEXT_PRIMARY};
use crate::{CommandSink, UiState};
use sequencer_engine::command::Command;

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
    ctx.data(|d| {
        d.get_temp::<SettingsState>(settings_id())
            .unwrap_or_default()
    })
}
fn write(ctx: &Context, f: impl FnOnce(&mut SettingsState)) {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(settings_id())));
}

/// Open the sheet. Records the frame for the open-frame guard, then closes the
/// three sibling track/pattern overlays — only one floating sheet may be open at
/// a time (mutual exclusion, symmetric with each sibling's `open`).
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

pub(crate) fn close(ctx: &Context) {
    write(ctx, |s| s.open = false);
}

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

/// Render the sheet if open. No-op (no panic) when closed.
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
                            .fill(SURFACE_HIGHEST)
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
        assert!(
            window_rect(&h.ctx).is_some(),
            "settings area must render when open"
        );
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

    // ---- T13a Task 4: gear trigger + symmetric exclusion + mode-agnostic ----

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
        crate::note_picker::open(&h.ctx, 0);
        assert!(
            !read(&h.ctx).open,
            "opening note_picker must close settings"
        );
    }

    #[test]
    fn mode_switch_does_not_close_settings() {
        // T13a (resolved design, Option A): settings is mode-agnostic — a mode
        // switch via `write_mode` does NOT close it. The AppMode toggle's
        // explicit close-list (transport.rs) closes only the mode-bound overlays;
        // settings is deliberately excluded (see the comment there). An outside
        // POINTER click (e.g. on the toggle button) dismisses settings through
        // the shared `overlay::should_dismiss` outside-click guard, identical to
        // the other overlays — that path is covered by `outside_click_dismisses`.
        // This test guards the non-pointer mode-switch path.
        let h = Harness::new(UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        });
        crate::settings::open(&h.ctx);
        h.settle();
        assert!(read(&h.ctx).open);
        // Mode switch with NO outside pointer click → settings must persist.
        crate::write_mode(&h.ctx, crate::AppMode::Performance);
        h.idle();
        assert!(read(&h.ctx).open);
        crate::write_mode(&h.ctx, crate::AppMode::Editing);
        h.idle();
        assert!(read(&h.ctx).open, "a mode switch must not close settings");
    }
}
