//! Phase 1 §T T10b — step-grid widget. Port of the iOS `EditingView` `TrackList`
//! (`app/StepForge/Features/Editing/{TrackList,StepRow,StepCell,TrackHeader}.swift`).
//!
//! Pure UI (V4): reads [`UiState`], emits [`Command`]s via a [`CommandSink`],
//! never touches the engine. Cells reflect the *actual* mirror state — gestures
//! only emit commands; the engine echoes the change back through `apply`
//! (⊥ optimistic mutation, mirroring the iOS Hard Rule 2 split).
//!
//! Desktop gesture adaptation (design doc §Touch → mouse/keyboard):
//! - left-click empty   → `SetStep Mid`
//! - left-click filled  → cycle Mid→Accent→Low→off (off = `DeleteStep`)
//! - right-click        → `DeleteStep`
//! - vertical drag      → `SetStep Accent` (up) / `Low` (down)
//! - Alt+click          → ratchet popover (Off/X2/X3/X4) → `SetRatchet`
//!
//! Zoom 8/16 via toolbar toggle, scroll-wheel, `1`/`2` keys; `zoom = 8` doubles
//! cell width (design: cols 0..8 doubled-width). Widget-local state (zoom,
//! active ratchet popover, in-progress drag accumulator) persists across frames
//! in egui `ctx.data` temp storage — it is NOT engine mirror state.

use egui::{
    Color32, Context, CornerRadius, Id, Key, Layout, PointerButton, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};

use sequencer_engine::command::Command;
use sequencer_engine::models::{Ratchet, Track, VelocityZone, STEP_COUNT};

use crate::{CommandSink, UiState};

// ---- Palette (design §Widgets) ---- dark graphite tiers, orange active, zones.
const SURFACE_LOW: Color32 = Color32::from_rgb(0x1B, 0x1B, 0x1B); // inactive cell fill
const SURFACE_HIGH: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);
const PRIMARY: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00); // accent zone / mute-on
const ZONE_ACCENT: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00);
const ZONE_MID: Color32 = Color32::from_rgb(0xFF, 0xB6, 0x88);
const ZONE_LOW: Color32 = Color32::from_rgb(0x98, 0xCB, 0xFF);
const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xF5, 0xF5, 0xF5);
const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x8A, 0x8A);
const BORDER_WEAK: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);

// ---- Layout (desktop; iOS `GridMetrics` port, fixed size-classes) ----
const HEADER_WIDTH: f32 = 120.0;
const CELL_W_16: f32 = 26.0;
const CELL_W_8: f32 = 52.0; // zoom = 8 doubles width
const CELL_H: f32 = 34.0;
const STEP_GAP: f32 = 3.0; // iOS stepGap
const ROW_SPACING: f32 = 4.0; // iOS rowSpacing
const CORNER: u8 = 3; // iOS Theme.Radius.sm (egui CornerRadius is u8 px)
const PLAYHEAD_BAR: f32 = 2.0;
const RATCHET_CAP_W: f32 = 2.0;
const RATCHET_CAP_H: f32 = 6.0;
const DRAG_THRESHOLD: f32 = 8.0; // iOS StepGestureModifier minimumDistance
const BEYOND_LENGTH_ALPHA: f32 = 0.22; // iOS StepCell opacity when !isWithinLength
const MUTED_ALPHA: f32 = 0.4;

// ---- Temp-data ids (ctx.data) — `Id::new` is non-const, so accessors. ----
fn grid_id() -> Id {
    Id::new("stepforge.grid")
}
#[cfg(test)]
fn cell_rects_id() -> Id {
    Id::new("stepforge.grid.cell_rects")
}
#[cfg(test)]
fn ratchet_btn_rects_id() -> Id {
    Id::new("stepforge.grid.ratchet_btns")
}

/// Visible-columns zoom. `Eight` = zoomed in (doubled cell width); `Sixteen` = default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Zoom {
    #[default]
    Sixteen,
    Eight,
}

impl Zoom {
    fn cell_w(self) -> f32 {
        match self {
            Zoom::Eight => CELL_W_8,
            Zoom::Sixteen => CELL_W_16,
        }
    }
    fn toggle(self) -> Self {
        match self {
            Zoom::Eight => Zoom::Sixteen,
            Zoom::Sixteen => Zoom::Eight,
        }
    }
}

/// Widget-local state persisted in `ctx.data` (egui `IdTypeMap` temp storage).
/// UI-only — NOT engine mirror state. `Copy` so it round-trips through temp
/// storage without borrow-threading through egui closures.
#[derive(Clone, Copy, Debug, Default)]
pub struct GridUiState {
    pub zoom: Zoom,
    /// Open ratchet popover target `(track, step)` — set by Alt+click.
    pub ratchet_target: Option<(usize, usize)>,
    /// Screen anchor for the open popover (the Alt-clicked cell center).
    ratchet_pos: Option<Pos2>,
    /// Accumulated vertical drag delta (y) for the current primary-button drag.
    drag_accum_y: f32,
}

fn read_grid(ctx: &Context) -> GridUiState {
    ctx.data(|d| d.get_temp::<GridUiState>(grid_id()).unwrap_or_default())
}

fn write_grid(ctx: &Context, f: impl FnOnce(&mut GridUiState)) {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(grid_id())));
}

/// Intent of a left-click given the cell's current step.
/// Cycle (desktop adaptation of the iOS tap model): empty→Mid,
/// Mid→Accent, Accent→Low, Low→off (`Delete`).
enum ClickIntent {
    Set(VelocityZone),
    Delete,
}

fn click_intent(step: VelocityZoneStep) -> ClickIntent {
    use VelocityZone as Z;
    if !step.active {
        return ClickIntent::Set(Z::Mid);
    }
    match step.velocity_zone {
        Z::Mid => ClickIntent::Set(Z::Accent),
        Z::Accent => ClickIntent::Set(Z::Low),
        Z::Low => ClickIntent::Delete,
    }
}

/// `(active, velocity_zone)` slice — lets the pure helpers avoid borrowing the
/// whole `Step`/`Track`.
#[derive(Clone, Copy)]
struct VelocityZoneStep {
    active: bool,
    velocity_zone: VelocityZone,
}

impl VelocityZoneStep {
    fn from(step: &sequencer_engine::models::Step) -> Self {
        Self {
            active: step.active,
            velocity_zone: step.velocity_zone,
        }
    }
}

/// Vertical-drag intent: accumulated y delta → `Accent` (up) / `Low` (down) /
/// `None`. Mirrors the iOS `DragGesture` (translation.height < -8 → Accent).
fn drag_intent(dy: f32) -> Option<VelocityZone> {
    if dy < -DRAG_THRESHOLD {
        Some(VelocityZone::Accent)
    } else if dy > DRAG_THRESHOLD {
        Some(VelocityZone::Low)
    } else {
        None
    }
}

fn zone_color(zone: VelocityZone) -> Color32 {
    match zone {
        VelocityZone::Accent => ZONE_ACCENT,
        VelocityZone::Mid => ZONE_MID,
        VelocityZone::Low => ZONE_LOW,
    }
}

fn ratchet_repeats(r: Ratchet) -> usize {
    match r {
        Ratchet::Off => 0,
        Ratchet::X2 => 2,
        Ratchet::X3 => 3,
        Ratchet::X4 => 4,
    }
}

fn ratchet_label(r: Ratchet) -> &'static str {
    match r {
        Ratchet::Off => "Off",
        Ratchet::X2 => "×2",
        Ratchet::X3 => "×3",
        Ratchet::X4 => "×4",
    }
}

/// GM drum name (minimal port of iOS `DrumNames`; full table is T11 NotePicker).
fn drum_name(note: u8) -> String {
    let name = match note {
        35 | 36 => "Kick",
        37 => "Side Stick",
        38 | 40 => "Snare",
        39 => "Clap",
        42 => "Closed Hat",
        44 => "Pedal Hat",
        46 => "Open Hat",
        49 => "Crash",
        51 => "Ride",
        _ => return format!("Note {}", note),
    };
    name.to_string()
}

fn apply_zoom_input(ctx: &Context) {
    let mut zoom = read_grid(ctx).zoom;
    if ctx.input(|i| i.key_pressed(Key::Num1)) {
        zoom = Zoom::Eight;
    }
    if ctx.input(|i| i.key_pressed(Key::Num2)) {
        zoom = Zoom::Sixteen;
    }
    // scroll-wheel: up (negative y) → zoom in.
    let sy = ctx.input(|i| i.smooth_scroll_delta.y);
    if sy < 0.0 {
        zoom = Zoom::Eight;
    } else if sy > 0.0 {
        zoom = Zoom::Sixteen;
    }
    write_grid(ctx, |g| g.zoom = zoom);
}

/// Render the step grid. Pinned track-header column (left, fixed width) beside a
/// horizontally-scrolling region of 16 step cells per track. Reads `UiState`;
/// gestures emit `Command`s via `sink`.
pub fn render_step_grid(ui: &mut Ui, state: &UiState, sink: &impl CommandSink) {
    let ctx = ui.ctx().clone();
    apply_zoom_input(&ctx);
    let zoom = read_grid(&ctx).zoom;

    #[cfg(test)]
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<((usize, usize), Rect)>>(cell_rects_id())
            .clear();
        d.get_temp_mut_or_default::<Vec<(Ratchet, Rect)>>(ratchet_btn_rects_id())
            .clear();
    });

    // Toolbar: zoom toggle (also reachable via `1`/`2` keys + scroll-wheel).
    ui.horizontal(|ui| {
        let mut zoomed = zoom == Zoom::Eight;
        let label = if zoomed { "zoom 8" } else { "zoom 16" };
        ui.toggle_value(&mut zoomed, label);
        if zoomed != (zoom == Zoom::Eight) {
            write_grid(&ctx, |g| g.zoom = g.zoom.toggle());
        }
    });
    ui.separator();

    let tracks: &[Track] = state.tracks();
    if tracks.is_empty() {
        ui.label(egui::RichText::new("No tracks").color(TEXT_MUTED));
    } else {
        ui.horizontal_top(|ui| {
            // Pinned header column — fixed width, outside the horizontal scroll.
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = ROW_SPACING;
                for (t, track) in tracks.iter().enumerate() {
                    header(ui, t, track, sink);
                }
            });
            // One shared horizontal scroller for every row's cells (columns align).
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = ROW_SPACING;
                    for (t, track) in tracks.iter().enumerate() {
                        let playhead = state.playheads.get(&t).copied();
                        row(&ctx, ui, t, track, playhead, zoom, sink);
                    }
                });
            });
        });
    }

    // Ratchet popover (Alt+click target) — floats above, rendered last.
    if let Some((t, s)) = read_grid(&ctx).ratchet_target {
        let anchor = read_grid(&ctx).ratchet_pos.unwrap_or(Pos2::new(40.0, 40.0));
        render_ratchet_popover(&ctx, anchor, t, s, sink);
    }
}

fn row(
    ctx: &Context,
    ui: &mut Ui,
    track_idx: usize,
    track: &Track,
    playhead: Option<usize>,
    zoom: Zoom,
    sink: &impl CommandSink,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = STEP_GAP;
        for s in 0..STEP_COUNT {
            let step = track.steps[s];
            let is_within_length = s < track.length;
            let is_playing = playhead == Some(s);
            step_cell(
                ctx,
                ui,
                track_idx,
                s,
                step,
                track.muted,
                is_within_length,
                is_playing,
                zoom,
                sink,
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn step_cell(
    ctx: &Context,
    ui: &mut Ui,
    track_idx: usize,
    step_idx: usize,
    step: sequencer_engine::models::Step,
    muted: bool,
    is_within_length: bool,
    is_playing: bool,
    zoom: Zoom,
    sink: &impl CommandSink,
) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(zoom.cell_w(), CELL_H), Sense::click_and_drag());

    paint_cell(
        ui.painter(),
        rect,
        step,
        muted,
        is_within_length,
        is_playing,
    );

    handle_cell_gestures(
        ctx,
        &response,
        rect,
        track_idx,
        step_idx,
        step,
        is_within_length,
        sink,
    );

    #[cfg(test)]
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<((usize, usize), Rect)>>(cell_rects_id())
            .push(((track_idx, step_idx), rect));
    });
}

fn paint_cell(
    painter: &egui::Painter,
    rect: Rect,
    step: sequencer_engine::models::Step,
    muted: bool,
    is_within_length: bool,
    is_playing: bool,
) {
    let mut fill = if step.active {
        zone_color(step.velocity_zone)
    } else {
        SURFACE_LOW
    };
    let alpha = if is_within_length {
        1.0
    } else {
        BEYOND_LENGTH_ALPHA
    };
    if alpha < 1.0 {
        fill = fill.gamma_multiply(alpha);
    }
    if muted {
        fill = fill.gamma_multiply(MUTED_ALPHA);
    }
    let stroke = if step.active && step.velocity_zone == VelocityZone::Accent {
        Stroke::new(1.5_f32, PRIMARY)
    } else {
        Stroke::new(1.0_f32, BORDER_WEAK)
    };
    painter.rect(
        rect,
        CornerRadius::same(CORNER),
        fill,
        stroke,
        StrokeKind::Inside,
    );

    // 2 px playhead bar across the top when this column is the playhead.
    if is_playing {
        let bar = Rect::from_min_size(rect.left_top(), Vec2::new(rect.width(), PLAYHEAD_BAR));
        painter.rect_filled(bar, CornerRadius::same(1), TEXT_PRIMARY);
    }

    // Ratchet markers (bottom): N capsules.
    if step.ratchet != Ratchet::Off {
        let n = ratchet_repeats(step.ratchet);
        let stride = RATCHET_CAP_W + 1.0;
        let total = n as f32 * stride - 1.0;
        let start_x = rect.center().x - total * 0.5;
        let y = rect.bottom() - RATCHET_CAP_H - 2.0;
        for i in 0..n {
            let x = start_x + i as f32 * stride;
            let cap = Rect::from_min_size(Pos2::new(x, y), Vec2::new(RATCHET_CAP_W, RATCHET_CAP_H));
            painter.rect_filled(
                cap,
                CornerRadius::same(1),
                TEXT_PRIMARY.gamma_multiply(0.85),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_cell_gestures(
    ctx: &Context,
    response: &Response,
    rect: Rect,
    track_idx: usize,
    step_idx: usize,
    step: sequencer_engine::models::Step,
    is_within_length: bool,
    sink: &impl CommandSink,
) {
    let vs = VelocityZoneStep::from(&step);

    // right-click → delete (design: double-tap-delete → right-click on desktop)
    if response.secondary_clicked() {
        sink.push(Command::DeleteStep {
            track_idx,
            step_idx,
        });
    }

    // vertical drag (primary) → Accent/Low (iOS isActive guard: filled cells only)
    if response.drag_started_by(PointerButton::Primary) {
        write_grid(ctx, |g| g.drag_accum_y = 0.0);
    }
    if response.dragged_by(PointerButton::Primary) {
        // Hoist `drag_delta()` OUT of the `write_grid` closure: it re-enters the
        // egui Context lock (`ctx.input`), which deadlocks against the write
        // lock `write_grid`→`ctx.data_mut` already holds (parking_lot is
        // non-reentrant — observed as a 0%-CPU park mid-frame).
        let dy = response.drag_delta().y;
        write_grid(ctx, |g| g.drag_accum_y += dy);
    }
    if response.drag_stopped_by(PointerButton::Primary) && step.active && is_within_length {
        let dy = read_grid(ctx).drag_accum_y;
        if let Some(zone) = drag_intent(dy) {
            sink.push(Command::SetStep {
                track_idx,
                step_idx,
                zone,
            });
        }
        write_grid(ctx, |g| g.drag_accum_y = 0.0);
    }

    // left-click → cycle, OR Alt+click → open ratchet popover
    if response.clicked() {
        if ctx.input(|i| i.modifiers.alt) {
            write_grid(ctx, |g| {
                g.ratchet_target = Some((track_idx, step_idx));
                g.ratchet_pos = Some(rect.center());
            });
        } else {
            match click_intent(vs) {
                ClickIntent::Set(zone) => sink.push(Command::SetStep {
                    track_idx,
                    step_idx,
                    zone,
                }),
                ClickIntent::Delete => sink.push(Command::DeleteStep {
                    track_idx,
                    step_idx,
                }),
            }
        }
    }
}

/// Pinned track header: mute toggle (→ `SetTrackMuted`) + drum name + NOTE n.
/// Name/note/speed/length pickers are read-only here (land in T10e/T11).
fn header(ui: &mut Ui, track_idx: usize, track: &Track, sink: &impl CommandSink) {
    ui.allocate_ui_with_layout(
        Vec2::new(HEADER_WIDTH, CELL_H),
        Layout::left_to_right(egui::Align::Center),
        |ui| {
            let mute_btn = egui::Button::new(egui::RichText::new("M").color(if track.muted {
                Color32::BLACK
            } else {
                TEXT_MUTED
            }))
            .fill(if track.muted { PRIMARY } else { SURFACE_HIGH });
            if ui.add(mute_btn).clicked() {
                sink.push(Command::SetTrackMuted {
                    track_idx,
                    muted: !track.muted,
                });
            }
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(drum_name(track.midi_note))
                        .color(if track.muted {
                            TEXT_MUTED
                        } else {
                            TEXT_PRIMARY
                        })
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("NOTE {}", track.midi_note))
                        .color(TEXT_MUTED)
                        .text_style(egui::TextStyle::Small),
                );
            });
        },
    );
}

fn render_ratchet_popover(
    ctx: &Context,
    anchor: Pos2,
    track_idx: usize,
    step_idx: usize,
    sink: &impl CommandSink,
) {
    egui::Area::new(Id::new("stepforge.ratchet_popover"))
        .order(egui::Order::Foreground)
        .current_pos(anchor)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(88.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Ratchet · step {}", step_idx + 1))
                            .color(TEXT_PRIMARY),
                    );
                    ui.separator();
                    for r in [Ratchet::Off, Ratchet::X2, Ratchet::X3, Ratchet::X4] {
                        let resp = ui.add(
                            egui::Button::new(ratchet_label(r)).min_size(Vec2::new(72.0, 0.0)),
                        );
                        #[cfg(test)]
                        if let Some(rrect) = resp.interact_pointer_pos() {
                            let _ = rrect;
                        }
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(Ratchet, Rect)>>(
                                ratchet_btn_rects_id(),
                            )
                            .push((r, resp.rect));
                        });
                        if resp.clicked() {
                            sink.push(Command::SetRatchet {
                                track_idx,
                                step_idx,
                                ratchet: r,
                            });
                            write_grid(ctx, |g| {
                                g.ratchet_target = None;
                                g.ratchet_pos = None;
                            });
                        }
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // ---- Pure oracle tests (no egui layout) ----

    #[test]
    fn click_intent_cycles_zone() {
        // empty → Mid
        assert!(matches!(
            click_intent(VelocityZoneStep {
                active: false,
                velocity_zone: VelocityZone::Mid
            }),
            ClickIntent::Set(VelocityZone::Mid)
        ));
        // Mid → Accent
        assert!(matches!(
            click_intent(VelocityZoneStep {
                active: true,
                velocity_zone: VelocityZone::Mid
            }),
            ClickIntent::Set(VelocityZone::Accent)
        ));
        // Accent → Low
        assert!(matches!(
            click_intent(VelocityZoneStep {
                active: true,
                velocity_zone: VelocityZone::Accent
            }),
            ClickIntent::Set(VelocityZone::Low)
        ));
        // Low → off (delete)
        assert!(matches!(
            click_intent(VelocityZoneStep {
                active: true,
                velocity_zone: VelocityZone::Low
            }),
            ClickIntent::Delete
        ));
    }

    #[test]
    fn drag_intent_vertical_zones() {
        assert_eq!(drag_intent(-20.0), Some(VelocityZone::Accent)); // up
        assert_eq!(drag_intent(20.0), Some(VelocityZone::Low)); // down
        assert_eq!(drag_intent(0.0), None); // no movement
        assert_eq!(drag_intent(7.9), None); // below threshold
        assert_eq!(drag_intent(-8.0), None); // strict `<`: exactly at threshold → no trigger
        assert_eq!(drag_intent(-8.1), Some(VelocityZone::Accent)); // just past threshold
    }

    #[test]
    fn ratchet_repeats_map() {
        assert_eq!(ratchet_repeats(Ratchet::Off), 0);
        assert_eq!(ratchet_repeats(Ratchet::X2), 2);
        assert_eq!(ratchet_repeats(Ratchet::X3), 3);
        assert_eq!(ratchet_repeats(Ratchet::X4), 4);
    }

    #[test]
    fn drum_name_falls_back_to_note() {
        assert_eq!(drum_name(36), "Kick");
        assert_eq!(drum_name(38), "Snare");
        assert_eq!(drum_name(42), "Closed Hat");
        assert_eq!(drum_name(12), "Note 12"); // unknown → fallback
    }

    // ---- Headless render harness (e2e wiring via real cell rects) ----

    #[derive(Default, Clone)]
    struct Rec(Arc<Mutex<Vec<Command>>>);
    impl CommandSink for Rec {
        fn push(&self, c: Command) {
            self.0.lock().unwrap().push(c);
        }
    }

    fn fixture() -> UiState {
        // 4 default tracks; track 0 step 0 = Mid (active), step 1 = Accent (active).
        use sequencer_engine::models::{Session, Step};
        let mut st = UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            let p = s.patterns[0].as_mut().unwrap();
            p.tracks[0].steps[0] = Step {
                active: true,
                velocity_zone: VelocityZone::Mid,
                ..Default::default()
            };
            p.tracks[0].steps[1] = Step {
                active: true,
                velocity_zone: VelocityZone::Accent,
                ..Default::default()
            };
        }
        st
    }

    fn raw_input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::new(0.0, 0.0),
                Vec2::new(1400.0, 800.0),
            )),
            ..Default::default()
        }
    }

    struct Harness {
        ctx: Context,
        state: UiState,
        sink: Rec,
    }
    impl Harness {
        fn new(state: UiState) -> Self {
            Self {
                ctx: Context::default(),
                state,
                sink: Rec::default(),
            }
        }
        fn frame(&self, raw: egui::RawInput) {
            let _ = self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    render_step_grid(ui, &self.state, &self.sink);
                });
            });
        }
        fn idle(&self) {
            self.frame(raw_input());
        }
        /// Cell center from the widget's own recorded rect (robust to layout
        /// margins/toolbar offset — the widget reports where it actually drew).
        fn cell_center(&self, t: usize, s: usize) -> Pos2 {
            self.idle();
            self.ctx
                .data(|d| d.get_temp::<Vec<((usize, usize), Rect)>>(cell_rects_id()))
                .unwrap_or_default()
                .into_iter()
                .find(|((tt, ss), _)| *tt == t && *ss == s)
                .map(|(_, r)| r.center())
                .expect("cell rect recorded")
        }
        fn ratchet_btn_center(&self, want: Ratchet) -> Pos2 {
            self.ctx
                .data(|d| d.get_temp::<Vec<(Ratchet, Rect)>>(ratchet_btn_rects_id()))
                .unwrap_or_default()
                .into_iter()
                .find(|(r, _)| *r == want)
                .map(|(_, r)| r.center())
                .expect("ratchet button rect recorded")
        }
        fn click_primary(&self, pos: Pos2, mods: egui::Modifiers) {
            let mut a = raw_input();
            a.modifiers = mods;
            a.events.push(egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: mods,
            });
            self.frame(a);
            let mut b = raw_input();
            b.modifiers = mods;
            b.events.push(egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: mods,
            });
            self.frame(b);
        }
        fn click_secondary(&self, pos: Pos2) {
            let mods = egui::Modifiers::default();
            for pressed in [true, false] {
                let mut r = raw_input();
                r.events.push(egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Secondary,
                    pressed,
                    modifiers: mods,
                });
                self.frame(r);
            }
        }
        fn drag(&self, from: Pos2, to: Pos2) {
            let mods = egui::Modifiers::default();
            let mut a = raw_input();
            a.events.push(egui::Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: mods,
            });
            self.frame(a);
            let mut m = raw_input();
            m.events.push(egui::Event::PointerMoved(to));
            self.frame(m);
            let mut rel = raw_input();
            rel.events.push(egui::Event::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: mods,
            });
            self.frame(rel);
        }
        fn press_key(&self, key: Key) {
            let mut r = raw_input();
            r.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            });
            self.frame(r);
        }
        fn scroll(&self, y: f32) {
            let mut r = raw_input();
            r.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, y),
                modifiers: Default::default(),
            });
            self.frame(r);
        }
        fn cmds(&self) -> Vec<Command> {
            self.sink.0.lock().unwrap().clone()
        }
    }

    #[test]
    fn grid_left_click_cycles_zone() {
        let h = Harness::new(fixture());
        let pos = h.cell_center(0, 0); // step 0 = active Mid
        h.click_primary(pos, egui::Modifiers::default());
        // Mid → Accent
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Command::SetStep {
                track_idx: 0,
                step_idx: 0,
                zone: VelocityZone::Accent
            }
        ));
    }

    #[test]
    fn grid_left_click_empty_places_mid() {
        let h = Harness::new(fixture());
        let pos = h.cell_center(0, 2); // empty
        h.click_primary(pos, egui::Modifiers::default());
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Command::SetStep {
                track_idx: 0,
                step_idx: 2,
                zone: VelocityZone::Mid
            }
        ));
    }

    #[test]
    fn grid_right_click_deletes() {
        let h = Harness::new(fixture());
        let pos = h.cell_center(0, 1); // active Accent
        h.click_secondary(pos);
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Command::DeleteStep {
                track_idx: 0,
                step_idx: 1
            }
        ));
    }

    #[test]
    fn grid_drag_up_emits_accent() {
        let h = Harness::new(fixture());
        let from = h.cell_center(0, 0); // active Mid
        h.drag(from, Pos2::new(from.x, from.y - 40.0));
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Command::SetStep {
                track_idx: 0,
                step_idx: 0,
                zone: VelocityZone::Accent
            }
        ));
    }

    #[test]
    fn grid_drag_down_emits_low() {
        let h = Harness::new(fixture());
        let from = h.cell_center(0, 1); // active Accent
        h.drag(from, Pos2::new(from.x, from.y + 40.0));
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Command::SetStep {
                track_idx: 0,
                step_idx: 1,
                zone: VelocityZone::Low
            }
        ));
    }

    #[test]
    fn grid_zoom_key_doubles_cell_width() {
        let h = Harness::new(fixture());
        let w16 = h.cell_center(0, 0); // prime default zoom = 16
        let _ = w16;
        // width at zoom 16
        let rect16 = h
            .ctx
            .data(|d| d.get_temp::<Vec<((usize, usize), Rect)>>(cell_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|((t, s), _)| *t == 0 && *s == 0)
            .map(|(_, r)| r)
            .unwrap();
        h.press_key(Key::Num1); // zoom in
        h.idle();
        let rect8 = h
            .ctx
            .data(|d| d.get_temp::<Vec<((usize, usize), Rect)>>(cell_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|((t, s), _)| *t == 0 && *s == 0)
            .map(|(_, r)| r)
            .unwrap();
        assert!(rect8.width() > rect16.width(), "zoom 8 should widen cells");
        assert!(
            (rect8.width() - CELL_W_8).abs() < 0.5,
            "zoom 8 width {}",
            rect8.width()
        );
    }

    #[test]
    fn grid_scroll_wheel_zooms_in() {
        let h = Harness::new(fixture());
        h.idle();
        let z_before = h
            .ctx
            .data(|d| d.get_temp::<GridUiState>(grid_id()).unwrap_or_default())
            .zoom;
        assert_eq!(z_before, Zoom::Sixteen);
        h.scroll(-50.0); // wheel up → zoom in
        let z_after = h
            .ctx
            .data(|d| d.get_temp::<GridUiState>(grid_id()).unwrap_or_default())
            .zoom;
        assert_eq!(z_after, Zoom::Eight);
    }

    #[test]
    fn grid_modifier_click_opens_ratchet_and_sets() {
        let h = Harness::new(fixture());
        let pos = h.cell_center(0, 0);
        // Alt+click → open popover (no command yet)
        h.click_primary(pos, egui::Modifiers::ALT);
        assert_eq!(h.cmds().len(), 0, "Alt+click must not place a step");
        let target = h
            .ctx
            .data(|d| d.get_temp::<GridUiState>(grid_id()).unwrap_or_default())
            .ratchet_target;
        assert_eq!(target, Some((0, 0)), "popover should be open");

        // render once so the popover's button rects are recorded
        h.idle();
        // click the ×2 button
        let btn = h.ratchet_btn_center(Ratchet::X2);
        h.click_primary(btn, egui::Modifiers::default());
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Command::SetRatchet {
                track_idx: 0,
                step_idx: 0,
                ratchet: Ratchet::X2
            }
        ));
    }

    #[test]
    fn grid_render_no_session_no_panic() {
        let h = Harness::new(UiState::default()); // no session → "No tracks"
        h.idle();
        h.press_key(Key::Num1); // zoom input on empty grid must not panic
        h.idle();
        // no commands possible with no tracks
        assert!(h.cmds().is_empty());
    }
}
