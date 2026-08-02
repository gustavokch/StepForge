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
//! Zoom 8/16 via the TransportBar radios + `1`/`2` keys; `zoom = 8` doubles
//! cell width (cols 0..8 doubled-width). No scroll-wheel zoom: DAWs claim
//! Cmd+scroll before the plugin sees it, and egui's `zoom_delta` is the same
//! signal it applies to global `pixels_per_point`, so a wheel-driven grid zoom
//! is either a no-op (host ate the gesture) or a whole-editor scale — neither
//! is the grid zoom we want. Plain wheel just pans the row scroller.
//! Widget-local state (zoom, active ratchet popover, in-progress drag
//! accumulator) persists across frames in egui `ctx.data` temp storage — it is
//! NOT engine mirror state.

use egui::{
    Color32, Context, CornerRadius, Id, Key, Layout, PointerButton, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};

use sequencer_engine::command::Command;
use sequencer_engine::models::{Ratchet, Step, Track, VelocityZone, STEP_COUNT};

use crate::{CommandSink, UiState};

// ---- Palette (design §Widgets) ---- dark graphite tiers, orange active, zones.
// `pub(crate)` so the TransportBar (T10c) reuses the same tokens — one palette,
// no drift between widgets. Full palette/typography module lands in Phase 4.
pub(crate) const SURFACE_LOW: Color32 = Color32::from_rgb(0x1B, 0x1B, 0x1B); // inactive cell fill
pub(crate) const SURFACE_HIGH: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);
// PRIMARY (UI accent: accent-zone stroke, mute-on fill) and ZONE_ACCENT (step
// velocity-zone fill) share the same orange deliberately — kept as two names so
// each call site reads by role, not by coincidental value.
pub(crate) const PRIMARY: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00); // accent stroke / mute-on fill
pub(crate) const ZONE_ACCENT: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00); // Accent-zone step fill
pub(crate) const ZONE_MID: Color32 = Color32::from_rgb(0xFF, 0xB6, 0x88);
pub(crate) const ZONE_LOW: Color32 = Color32::from_rgb(0x98, 0xCB, 0xFF);
pub(crate) const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xF5, 0xF5, 0xF5);
pub(crate) const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x8A, 0x8A);
pub(crate) const BORDER_WEAK: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);

// ---- Layout (desktop; iOS `GridMetrics` port, fixed size-classes) ----
// Fits the longest drum name ("Closed Hat"/"Side Stick") + mute + `…` buttons.
// The name label is `.truncate()`d into a bounded slot (see `header`), so even
// a longer name clips inside the column instead of overflowing the grid — but
// 160 keeps the current names un-truncated. (egui `Label` defaults to
// `TextWrapMode::Extend`, which GROWS the parent Ui to fit the text — that was
// the overflow: "Closed Hat" expanded the header past its column.)
const HEADER_WIDTH: f32 = 160.0;
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
fn cell_rects_id() -> Id {
    Id::new("stepforge.grid.cell_rects")
}
#[cfg(test)]
fn ratchet_btn_rects_id() -> Id {
    Id::new("stepforge.grid.ratchet_btns")
}
#[cfg(test)]
fn mute_btn_rects_id() -> Id {
    Id::new("stepforge.grid.mute_btns")
}
#[cfg(test)]
fn popover_rect_id() -> Id {
    Id::new("stepforge.grid.popover_rect")
}
#[cfg(test)]
pub(crate) fn note_btn_rects_id() -> Id {
    Id::new("stepforge.grid.note_btns")
}
#[cfg(test)]
pub(crate) fn more_btn_rects_id() -> Id {
    Id::new("stepforge.grid.more_btns")
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
}

/// Widget-local state persisted in `ctx.data` (egui `IdTypeMap` temp storage).
/// UI-only — NOT engine mirror state. `Copy` so it round-trips through temp
/// storage without borrow-threading through egui closures.
#[derive(Clone, Copy, Debug, Default)]
pub struct GridUiState {
    pub zoom: Zoom,
    /// Open ratchet popover target `(track, step)` — set by Alt+click.
    pub ratchet_target: Option<(usize, usize)>,
    /// Accumulated vertical drag delta (y) for the current primary-button drag.
    drag_accum_y: f32,
}

/// `pub(crate)`: the TransportBar zoom toggle (T10c) reads/writes the SAME
/// `grid_id()` temp slot — one zoom state shared by both widgets, no duplicate.
pub(crate) fn read_grid(ctx: &Context) -> GridUiState {
    ctx.data(|d| d.get_temp::<GridUiState>(grid_id()).unwrap_or_default())
}

pub(crate) fn write_grid(ctx: &Context, f: impl FnOnce(&mut GridUiState)) {
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
    fn from(step: &Step) -> Self {
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
/// Returns a `&'static str` so the mapped path allocates nothing per frame;
/// unmapped notes return `"Note"`; the raw number is shown as a hover
/// tooltip on the header name. `pub(crate)`: the T11 ActionDrawer title and the
/// NotePickerSheet current-note label both reuse this short map (the long GM
/// table lives in `note_picker::GM_DRUMS`).
pub(crate) fn drum_name(note: u8) -> &'static str {
    match note {
        35 | 36 => "Kick",
        37 => "Side Stick",
        38 | 40 => "Snare",
        39 => "Clap",
        42 => "Closed Hat",
        44 => "Pedal Hat",
        46 => "Open Hat",
        49 => "Crash",
        51 => "Ride",
        _ => "Note",
    }
}

/// Zoom follows only the TransportBar radios and the `1`/`2` keys. Scroll-wheel
/// zoom was removed: DAWs claim Cmd+scroll before the plugin sees it, and egui's
/// `zoom_delta` is the same signal it applies to global `pixels_per_point`, so a
/// wheel-driven grid zoom is either a no-op (host ate the gesture) or a
/// whole-editor scale — neither is the grid zoom we want. Plain wheel pans.
fn apply_zoom_input(ctx: &Context) {
    let mut zoom = read_grid(ctx).zoom;
    if ctx.input(|i| i.key_pressed(Key::Num1)) {
        zoom = Zoom::Eight;
    }
    if ctx.input(|i| i.key_pressed(Key::Num2)) {
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
    let grid = read_grid(&ctx);
    let zoom = grid.zoom;
    // `popover_open` (pre-loop snapshot) drives per-cell rect recording below.
    // It is false on the Alt+click frame itself; `just_opened` (returned by
    // `handle_cell_gestures`) records the just-clicked cell that same frame.
    let popover_open = grid.ratchet_target.is_some();

    // The popover anchor reads the live target-cell rect each frame, so the
    // cell-rect buffer must clear in prod (not just tests).
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<((usize, usize), Rect)>>(cell_rects_id())
            .clear();
    });
    #[cfg(test)]
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(Ratchet, Rect)>>(ratchet_btn_rects_id())
            .clear();
        d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(mute_btn_rects_id())
            .clear();
        // Reset the popover-rect probe so a closed popover doesn't keep a stale
        // rect from the last frame it was open.
        *d.get_temp_mut_or_default::<Option<Rect>>(popover_rect_id()) = None;
    });

    // The visible zoom toggle lives in the TransportBar (T10c) — it shares this
    // widget's `grid_id()` temp slot, so the grid just reads `zoom` here. The
    // `1`/`2` keys (see `apply_zoom_input`) still mutate the same slot from
    // within the grid region.

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
                        row(&ctx, ui, t, track, playhead, zoom, popover_open, sink);
                    }
                });
            });
        });
    }

    // Ratchet popover (Alt+click target) — floats above, rendered last. The
    // anchor is the target cell's LIVE rect (recorded this frame by `step_cell`
    // into `cell_rects_id`), so the popover tracks the cell when the grid is
    // scrolled horizontally. The old code snapshotted a screen `Pos2` at
    // Alt-click that went stale on the first scroll.
    let g = read_grid(&ctx);
    if let Some((t, s)) = g.ratchet_target {
        let anchor = ctx
            .data(|d| d.get_temp::<Vec<((usize, usize), Rect)>>(cell_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|((tt, ss), _)| (*tt, *ss) == (t, s))
            .map(|(_, r)| r.center())
            // Load-bearing assumption: egui's `ScrollArea` is NON-virtualizing
            // — it lays out all 16 cells every frame, so the target cell's rect
            // is always present in `cell_rects` and this lookup never misses.
            // If scrolling is ever virtualized (off-screen cells skipped), this
            // falls silently to the `(40, 40)` fallback and the popover detaches
            // from the cell under scroll.
            .unwrap_or(Pos2::new(40.0, 40.0));
        render_ratchet_popover(&ctx, anchor, t, s, sink);
    }
}

#[allow(clippy::too_many_arguments)]
fn row(
    ctx: &Context,
    ui: &mut Ui,
    track_idx: usize,
    track: &Track,
    playhead: Option<usize>,
    zoom: Zoom,
    popover_open: bool,
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
                popover_open,
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
    step: Step,
    muted: bool,
    is_within_length: bool,
    is_playing: bool,
    zoom: Zoom,
    popover_open: bool,
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

    // `handle_cell_gestures` may open the ratchet popover this very frame
    // (Alt+click); `just_opened` lets us record that cell's rect on the same
    // frame so the popover anchors correctly instead of flashing to the
    // fallback pos for one frame.
    let just_opened = handle_cell_gestures(
        ctx,
        &response,
        track_idx,
        step_idx,
        step,
        is_within_length,
        sink,
    );

    // Record this cell's rect so the open popover can anchor to its live
    // position (and follow it under horizontal scroll). `popover_open` covers
    // the steady state (records every cell while open); `just_opened` covers
    // the frame the popover opens; `cfg!(test)` lets the headless harness click
    // any cell with no popover open. No recording work on the common path.
    if cfg!(test) || popover_open || just_opened {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<((usize, usize), Rect)>>(cell_rects_id())
                .push(((track_idx, step_idx), rect));
        });
    }
}

fn paint_cell(
    painter: &egui::Painter,
    rect: Rect,
    step: Step,
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
    track_idx: usize,
    step_idx: usize,
    step: Step,
    is_within_length: bool,
    sink: &impl CommandSink,
) -> bool {
    let vs = VelocityZoneStep::from(&step);
    let mut just_opened = false;

    // right-click → delete (design: double-tap-delete → right-click on desktop).
    // Beyond `track.length` the step is inert — iOS disables double-tap-delete
    // past the length window — so block the delete there.
    if response.secondary_clicked() && is_within_length {
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
            });
            just_opened = true;
        } else {
            match click_intent(vs) {
                ClickIntent::Set(zone) => sink.push(Command::SetStep {
                    track_idx,
                    step_idx,
                    zone,
                }),
                // iOS: double-tap-delete is disabled beyond the length window.
                // Placing (Set) stays allowed past `track.length`, but the cycle's
                // Delete state is a no-op there.
                ClickIntent::Delete if is_within_length => sink.push(Command::DeleteStep {
                    track_idx,
                    step_idx,
                }),
                ClickIntent::Delete => {}
            }
        }
    }

    just_opened
}

/// Pinned track header: mute toggle (→ `SetTrackMuted`) + drum name (note number on hover).
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
            let mute_resp = ui.add(mute_btn);
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(mute_btn_rects_id())
                    .push((track_idx, mute_resp.rect));
            });
            if mute_resp.clicked() {
                sink.push(Command::SetTrackMuted {
                    track_idx,
                    muted: !track.muted,
                });
            }
            // T11 — the drum name is the NotePickerSheet trigger (iOS
            // TrackHeader drum-name tap). Bounded to a fixed slot + `.truncate()`
            // so a long name clips INSIDE the header column instead of expanding
            // it into the step grid (egui `Label` defaults to `TextWrapMode::Extend`,
            // which grows the parent Ui — that was the "Closed Hat" overflow).
            // `…` space is reserved out of `available_width` first so the name
            // can't steal it. `Sense::click` keeps the lazy NOTE hover tooltip.
            let name_w = (ui.available_width() - 36.0 - ui.spacing().item_spacing.x).max(40.0);
            let name_resp = ui
                .add_sized(
                    Vec2::new(name_w, CELL_H),
                    egui::Label::new(
                        egui::RichText::new(drum_name(track.midi_note))
                            .color(if track.muted {
                                TEXT_MUTED
                            } else {
                                TEXT_PRIMARY
                            })
                            .strong(),
                    )
                    .truncate()
                    .sense(Sense::click()),
                )
                .on_hover_text(format!("NOTE {}", track.midi_note));
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(note_btn_rects_id())
                    .push((track_idx, name_resp.rect));
            });
            if name_resp.clicked() {
                crate::note_picker::open(ui.ctx(), track_idx);
            }
            // T11 — "…" opens the ActionDrawer for this track (iOS ellipsis).
            let more_resp = ui.add(egui::Button::new("…").fill(SURFACE_HIGH));
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(more_btn_rects_id())
                    .push((track_idx, more_resp.rect));
            });
            if more_resp.clicked() {
                crate::action_drawer::open(ui.ctx(), track_idx);
            }
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
    let area_resp = egui::Area::new(Id::new("stepforge.ratchet_popover"))
        .order(egui::Order::Foreground)
        .current_pos(anchor)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(88.0);
                ui.vertical_centered(|ui| {
                    // GUI-thread `format!`, only while this popover is open (a
                    // rare, user-driven state) — V4 requires the RT path to be
                    // alloc-free, not the GUI thread.
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
                            });
                        }
                    }
                });
            });
        });

    // Dismiss the popover without committing: Esc, or a primary click anywhere
    // outside it. `!alt` keeps the same-frame opening Alt+click from
    // self-dismissing; Alt+clicking another cell re-targets via that cell's
    // gesture (which runs before this popover render) instead.
    let rect = area_resp.response.rect;
    // Test-only probe: expose the popover's screen rect so the headless harness
    // can assert the anchor tracks the target cell (N3 regression guard).
    #[cfg(test)]
    ctx.data_mut(|d| {
        *d.get_temp_mut_or_default::<Option<Rect>>(popover_rect_id()) = Some(rect);
    });
    let dismiss = ctx.input(|i| i.key_pressed(Key::Escape))
        || ctx.input(|i| {
            i.pointer.primary_clicked()
                && !i.modifiers.alt
                && i.pointer.latest_pos().is_none_or(|p| !rect.contains(p))
        });
    if dismiss {
        write_grid(ctx, |g| {
            g.ratchet_target = None;
        });
    }
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
        assert_eq!(drum_name(12), "Note"); // unknown → static fallback (number shown on hover)
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

    fn fixture_short() -> UiState {
        // Track 0 length = 8: cols 0..7 within length, 8..15 beyond (inert).
        // step 12 beyond-length and pre-set to active Low — exercises the
        // click-cycle's Delete-blocked branch past the length window.
        use sequencer_engine::models::Session;
        let mut st = UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            let p = s.patterns[0].as_mut().unwrap();
            p.tracks[0].length = 8;
            p.tracks[0].steps[12] = Step {
                active: true,
                velocity_zone: VelocityZone::Low,
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
        /// Cell center from the LAST rendered frame's recorded rect — no re-idle,
        /// so a test can read the popover rect and the cell rect from the same
        /// frame for a same-frame delta comparison.
        fn cell_center_now(&self, t: usize, s: usize) -> Option<Pos2> {
            self.ctx
                .data(|d| d.get_temp::<Vec<((usize, usize), Rect)>>(cell_rects_id()))
                .unwrap_or_default()
                .into_iter()
                .find(|((tt, ss), _)| *tt == t && *ss == s)
                .map(|(_, r)| r.center())
        }
        /// Ratchet popover `Area` rect from the last rendered frame (test-only
        /// probe stashed in `render_ratchet_popover`).
        fn popover_rect(&self) -> Option<Rect> {
            self.ctx
                .data(|d| d.get_temp::<Option<Rect>>(popover_rect_id()))
                .unwrap_or_default()
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
        fn ratchet_target(&self) -> Option<(usize, usize)> {
            self.ctx
                .data(|d| d.get_temp::<GridUiState>(grid_id()).unwrap_or_default())
                .ratchet_target
        }
        fn mute_btn_center(&self, t: usize) -> Pos2 {
            self.idle();
            self.ctx
                .data(|d| d.get_temp::<Vec<(usize, Rect)>>(mute_btn_rects_id()))
                .unwrap_or_default()
                .into_iter()
                .find(|(tt, _)| *tt == t)
                .map(|(_, r)| r.center())
                .expect("mute button rect recorded")
        }
        fn note_btn_center(&self, t: usize) -> Pos2 {
            self.idle();
            self.ctx
                .data(|d| d.get_temp::<Vec<(usize, Rect)>>(note_btn_rects_id()))
                .unwrap_or_default()
                .into_iter()
                .find(|(tt, _)| *tt == t)
                .map(|(_, r)| r.center())
                .expect("note button rect recorded")
        }
        fn more_btn_center(&self, t: usize) -> Pos2 {
            self.idle();
            self.ctx
                .data(|d| d.get_temp::<Vec<(usize, Rect)>>(more_btn_rects_id()))
                .unwrap_or_default()
                .into_iter()
                .find(|(tt, _)| *tt == t)
                .map(|(_, r)| r.center())
                .expect("more (…) button rect recorded")
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
        let _ = h.cell_center(0, 0); // prime default zoom = 16
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
    fn grid_zoom_key_num2_restores_sixteen() {
        // Keys are the primary zoom control now (scroll-wheel zoom dropped —
        // DAWs claim Cmd+scroll; see N5). Cover the reverse direction: Num1
        // widens to 8, Num2 narrows back to the 16-cell width.
        let h = Harness::new(fixture());
        let cell_w = |h: &Harness| {
            h.idle();
            h.ctx
                .data(|d| d.get_temp::<Vec<((usize, usize), Rect)>>(cell_rects_id()))
                .unwrap_or_default()
                .into_iter()
                .find(|((t, s), _)| *t == 0 && *s == 0)
                .map(|(_, r)| r.width())
                .unwrap()
        };
        assert!((cell_w(&h) - CELL_W_16).abs() < 0.5, "default = 16");
        h.press_key(Key::Num1); // → Eight
        assert!((cell_w(&h) - CELL_W_8).abs() < 0.5, "Num1 → 8 (wide)");
        h.press_key(Key::Num2); // → Sixteen
        assert!((cell_w(&h) - CELL_W_16).abs() < 0.5, "Num2 → 16 (narrow)");
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

    // ---- PR #16 review: beyond-length editability is iOS-faithful ----

    #[test]
    fn grid_right_click_beyond_length_blocked() {
        let h = Harness::new(fixture_short()); // track 0 length = 8
        let pos = h.cell_center(0, 12); // beyond length → inert
        h.click_secondary(pos);
        assert!(
            h.cmds().is_empty(),
            "right-click beyond length must not delete"
        );
    }

    #[test]
    fn grid_left_click_beyond_length_places_not_deletes() {
        // empty beyond-length cell → placing (SetStep Mid) is allowed
        let h = Harness::new(fixture_short());
        let pos = h.cell_center(0, 11); // beyond length(8), empty
        h.click_primary(pos, egui::Modifiers::default());
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Command::SetStep {
                track_idx: 0,
                step_idx: 11,
                zone: VelocityZone::Mid
            }
        ));

        // active-Low beyond-length cell → cycle's Delete state is blocked
        let h2 = Harness::new(fixture_short());
        let pos2 = h2.cell_center(0, 12); // beyond length(8), active Low
        h2.click_primary(pos2, egui::Modifiers::default());
        assert!(
            h2.cmds().is_empty(),
            "click-cycle to Delete beyond length must be a no-op"
        );
    }

    // ---- PR #16 review: ratchet popover dismiss (Esc + outside click) ----

    #[test]
    fn grid_ratchet_popover_dismiss_esc() {
        let h = Harness::new(fixture());
        let pos = h.cell_center(0, 0);
        h.click_primary(pos, egui::Modifiers::ALT); // open popover
        assert_eq!(h.ratchet_target(), Some((0, 0)));
        h.press_key(Key::Escape);
        assert_eq!(h.ratchet_target(), None, "Esc should dismiss the popover");
        assert!(
            !h.cmds()
                .iter()
                .any(|c| matches!(c, Command::SetRatchet { .. })),
            "dismiss must not emit SetRatchet"
        );
    }

    #[test]
    fn grid_ratchet_popover_dismiss_outside_click() {
        let h = Harness::new(fixture());
        let pos = h.cell_center(0, 0);
        h.click_primary(pos, egui::Modifiers::ALT); // open popover
        assert_eq!(h.ratchet_target(), Some((0, 0)));
        // click far outside the popover and outside any cell
        h.click_primary(Pos2::new(1300.0, 700.0), egui::Modifiers::default());
        assert_eq!(
            h.ratchet_target(),
            None,
            "outside click should dismiss the popover"
        );
    }

    // ---- PR #18 review: popover anchor tracks the live cell rect (N3) ----

    #[test]
    fn grid_ratchet_popover_anchor_tracks_cell() {
        // N3 regression: the popover anchor must derive from the target cell's
        // LIVE rect each frame (recorded into `cell_rects`), so it follows the
        // cell when layout shifts. A zoom change (Num1) widens every cell and
        // pushes cell 8 to the right; the popover must move with it. The pre-N3
        // code snapshotted a screen `Pos2` at Alt-click, which would NOT track —
        // popover delta would diverge from cell delta.
        //
        // Zoom stands in for horizontal scroll here: both shift the cell's
        // screen rect and both rely on the same live-rect re-derivation. The
        // headless harness viewport (1400px) fits all 16 cells, so plain
        // horizontal scroll has no range; zoom exercises the invariant without
        // viewport surgery.
        let h = Harness::new(fixture());
        let pos = h.cell_center(0, 8); // open the popover on cell (0, 8)
        h.click_primary(pos, egui::Modifiers::ALT);

        // The release frame recorded the popover rect and the cell rects together.
        let pop_before = h
            .popover_rect()
            .expect("popover rendered on open")
            .center()
            .x;
        let cell_before = h.cell_center_now(0, 8).expect("cell rect recorded").x;

        h.press_key(Key::Num1); // zoom in -> cells widen -> cell 8 shifts right

        let pop_after = h.popover_rect().expect("popover still rendered").center().x;
        let cell_after = h.cell_center_now(0, 8).expect("cell rect recorded").x;

        assert!(
            pop_after > pop_before + 50.0,
            "popover should shift right with the wider cells: {} -> {}",
            pop_before,
            pop_after
        );
        // The popover is positioned at the cell's center each frame, so its
        // horizontal delta must equal the cell's — a stale snapshot would not.
        let pop_delta = pop_after - pop_before;
        let cell_delta = cell_after - cell_before;
        assert!(
            (pop_delta - cell_delta).abs() < 1.0,
            "popover must track the cell: pop_delta={}, cell_delta={}",
            pop_delta,
            cell_delta
        );
    }

    // ---- PR #16 review: header mute toggle coverage ----

    #[test]
    fn grid_header_mute_toggles() {
        let h = Harness::new(fixture());
        let pos = h.mute_btn_center(0); // track 0 default muted = false
        h.click_primary(pos, egui::Modifiers::default());
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Command::SetTrackMuted {
                track_idx: 0,
                muted: true
            }
        ));
    }

    // ---- T11: header trigger points (NotePicker + ActionDrawer open) ----

    #[test]
    fn grid_header_drum_name_click_opens_note_picker() {
        let h = Harness::new(fixture());
        let pos = h.note_btn_center(0);
        h.click_primary(pos, egui::Modifiers::default());
        // Drum-name tap opens the NotePickerSheet for track 0…
        assert_eq!(
            crate::note_picker::read(&h.ctx).target,
            Some(0),
            "drum-name click should open the NotePickerSheet"
        );
        // …and closes the ActionDrawer (mutual exclusion — one overlay at a time).
        assert_eq!(crate::action_drawer::read(&h.ctx).target, None);
        // Opening is UI-only state — no command emitted.
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn grid_header_dots_click_opens_action_drawer() {
        let h = Harness::new(fixture());
        let pos = h.more_btn_center(0);
        h.click_primary(pos, egui::Modifiers::default());
        assert_eq!(
            crate::action_drawer::read(&h.ctx).target,
            Some(0),
            "… click should open the ActionDrawer"
        );
        // Mutual exclusion: NotePicker closed.
        assert_eq!(crate::note_picker::read(&h.ctx).target, None);
        assert!(h.cmds().is_empty());
    }
}
