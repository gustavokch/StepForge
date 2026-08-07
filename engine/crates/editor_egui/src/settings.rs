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
use egui::{Context, Id, Pos2, RichText};

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
    ctx.data(|d| {
        d.get_temp::<SettingsState>(settings_id())
            .unwrap_or_default()
    })
}
fn write(ctx: &Context, f: impl FnOnce(&mut SettingsState)) {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(settings_id())));
}

/// Open the sheet. Records the frame for the open-frame guard. (Mutual
/// exclusion — closing the three sibling overlays — is added in Task 4.)
#[allow(dead_code)] // Task 2 (T13b) wires the TransportBar gear that calls this.
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
}
