//! Phase 1 §T T10d — FeelBar widget. Port of the iOS `EditingView` "Row 2"
//! (`app/StepForge/Features/Editing/FeelBar.swift`): global swing, humanize
//! popover, quantize-grain cycle, and pattern switcher popover.
//!
//! Pure UI (V4): reads [`UiState`], emits [`Command`]s via a [`CommandSink`],
//! and never mutates the engine. Each control reflects the *actual* mirror
//! state (⊥ optimistic) — a gesture only emits a command; the engine echoes the
//! change back through `UiState::apply` (Hard Rule 2 split, ported from iOS).
//!
//! The single exception is the **quantize grain**: the engine stores it on the
//! scheduler (T16) and does not echo it to the mirror yet, so the FeelBar holds
//! the current grain as widget-local [`FeelUiState`] here, cycled on click —
//! matching the iOS `@State quantizeGrain`. The pattern switcher hardcodes
//! `NextBar` (faithful to `FeelBar.swift:136`), so the grain stays display-only
//! for now; the mirror echo + a live-grain `QueuePattern` land with T16.

use egui::{
    popup::popup_above_or_below_widget, AboveOrBelow, Button, Context, CornerRadius, Id, Layout,
    PopupCloseBehavior, RichText, Slider, Stroke, StrokeKind, Ui, Vec2,
};
use sequencer_engine::command::Command;
use sequencer_engine::models::{QuantizeGrain, PATTERN_SLOTS};

use crate::grid::{PRIMARY, SURFACE_HIGH, TEXT_MUTED, TEXT_PRIMARY};
use crate::{CommandSink, UiState};

// ---- Pure helpers (headless oracle; ⊥ egui state) ----

/// Swing upper bound — the iOS slider is `0...0.5` (`FeelBar.swift:71`); past
/// 0.5 the swing ratio exceeds 2:1 and stops making musical sense. The engine
/// field `global_swing_pct` is a plain `f32`, so the UI owns this clamp.
pub(crate) const SWING_MAX: f32 = 0.5;

/// Clamp a raw swing to `[0, SWING_MAX]`, mirroring the iOS slider range.
pub(crate) fn clamp_swing(raw: f32) -> f32 {
    raw.clamp(0.0, SWING_MAX)
}

/// Command for a swing edit. Pure → testable without driving the slider.
pub(crate) fn swing_edit_command(raw: f32) -> Command {
    Command::SetGlobalSwing {
        pct: clamp_swing(raw),
    }
}

/// Clamp a humanize axis to `[0, 1]` (iOS `HumanizeEditor` slider range,
/// `FeelBar.swift:186`).
pub(crate) fn clamp_axis(raw: f32) -> f32 {
    raw.clamp(0.0, 1.0)
}

/// Command for a humanize Apply. Pure → testable without driving the popover.
pub(crate) fn humanize_command(timing: f32, velocity: f32) -> Command {
    Command::SetHumanize {
        timing: clamp_axis(timing),
        velocity: clamp_axis(velocity),
    }
}

/// Pattern-queue command. iOS hardcodes `.nextBar` at the `PatternPickerPopover`
/// call site (`FeelBar.swift:136`) — the picker does NOT use the live quantize
/// grain, so neither do we (faithful port; revisit if a later spec ties them).
pub(crate) fn queue_pattern_command(index: usize) -> Command {
    Command::QueuePattern {
        index,
        quantize: QuantizeGrain::NextBar,
    }
}

/// Short label for a grain — port of iOS `QuantizeGrain.shortLabel`
/// (`Engine/Models.swift:53`).
pub(crate) fn grain_label(g: QuantizeGrain) -> &'static str {
    match g {
        QuantizeGrain::NextStep => "Step",
        QuantizeGrain::NextBeat => "Beat",
        QuantizeGrain::NextBar => "Bar",
        QuantizeGrain::EndOfPattern => "Pat",
    }
}

/// Next grain in the cycle (`FeelBar.swift:116` order):
/// Step → Beat → Bar → Pat → Step.
pub(crate) fn next_grain(g: QuantizeGrain) -> QuantizeGrain {
    use QuantizeGrain::*;
    match g {
        NextStep => NextBeat,
        NextBeat => NextBar,
        NextBar => EndOfPattern,
        EndOfPattern => NextStep,
    }
}

/// Swing shown this frame: the in-flight (user-edited, echo-pending) value wins
/// over the mirror so an active drag/click does not snap back to the stale
/// mirror each frame; once the engine echo (a throttled `FullSnapshot` on the
/// large channel) lands, [`clear_swing_inflight`] drops it. Same pattern as the
/// transport BPM inflight
/// (`transport.rs`).
pub(crate) fn seed_swing(mirror: f32, inflight: Option<f32>) -> f32 {
    inflight.unwrap_or(mirror)
}

/// Whether the mirror has caught up to the pending in-flight swing (echo
/// arrived) → safe to drop the override. Exact f32 eq is sound here: the value
/// round-trips `SetGlobalSwing { pct }` to the snapshot's `global_swing_pct`
/// unchanged (plain `f32` assign in `FullSnapshot`, no rounding/clamp).
pub(crate) fn clear_swing_inflight(mirror: f32, inflight: Option<f32>) -> bool {
    match inflight {
        Some(pending) => (mirror - pending).abs() < 1e-6,
        None => false,
    }
}

// ---- Widget-local temp ids (`Id::new` is non-const ∴ accessors) ----

fn feel_id() -> Id {
    Id::new("stepforge.feel")
}
fn humanize_popup_id() -> Id {
    Id::new("stepforge.feel.humanize_popup")
}
fn patterns_popup_id() -> Id {
    Id::new("stepforge.feel.patterns_popup")
}
#[cfg(test)]
fn humanize_btn_rect_id() -> Id {
    Id::new("stepforge.feel.humanize_btn_rect")
}
#[cfg(test)]
fn humanize_apply_rect_id() -> Id {
    Id::new("stepforge.feel.humanize_apply_rect")
}
#[cfg(test)]
fn patterns_btn_rect_id() -> Id {
    Id::new("stepforge.feel.patterns_btn_rect")
}
#[cfg(test)]
fn pattern_slot_rects_id() -> Id {
    Id::new("stepforge.feel.pattern_slot_rects")
}
#[cfg(test)]
fn quantize_btn_rect_id() -> Id {
    Id::new("stepforge.feel.quantize_btn_rect")
}
#[cfg(test)]
fn swing_rect_id() -> Id {
    Id::new("stepforge.feel.swing_rect")
}

/// Widget-local state persisted in `ctx.data` (egui `IdTypeMap` temp storage).
/// UI-only — NOT engine mirror state. `Copy` so it round-trips through temp
/// storage without borrow-threading through egui closures (same shape as
/// `grid.rs::GridUiState`). The quantize grain lives here because the engine
/// does not echo it to the mirror yet (T16); the humanize axes are the
/// popover's editing values, seeded from the mirror when the popover opens
/// (`FeelBar.swift:seedFromMirror`); the swing inflight holds an echo-pending
/// drag value (⊥ snap-back).
#[derive(Clone, Copy, Debug)]
struct FeelUiState {
    /// Current quantize grain (cycled by the GRID button). Default `NextBeat`,
    /// matching iOS `@State quantizeGrain = .nextBeat`.
    grain: QuantizeGrain,
    /// Echo-pending swing value (⊥ snap-back while a `FullSnapshot` echo is pending).
    swing_inflight: Option<f32>,
    /// Humanize popover editing values, seeded from the mirror on open.
    humanize_timing: f32,
    humanize_velocity: f32,
}

impl Default for FeelUiState {
    fn default() -> Self {
        Self {
            grain: QuantizeGrain::NextBeat,
            swing_inflight: None,
            humanize_timing: 0.0,
            humanize_velocity: 0.0,
        }
    }
}

fn read_feel(ctx: &Context) -> FeelUiState {
    ctx.data(|d| d.get_temp::<FeelUiState>(feel_id()).unwrap_or_default())
}

fn write_feel(ctx: &Context, f: impl FnOnce(&mut FeelUiState)) {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(feel_id())));
}

// ---- The bar ----

/// Render the FeelBar (Row 2). `state` is the live mirror; gestures emit via
/// `sink`. Read-only over session ground truth except for the explicit emits:
/// `SetGlobalSwing` (swing slider), `SetHumanize` (humanize Apply),
/// `SetQuantizeGrain` (GRID cycle), `QueuePattern` (pattern slot).
pub fn render_feel_bar(ui: &mut Ui, state: &UiState, sink: &impl CommandSink) {
    let ctx = ui.ctx().clone();

    ui.horizontal(|ui| {
        // ---- patterns switcher (popover): P{active+1}; click → open bank ----
        let active = state.active_pattern_index();
        let pat_resp = ui.add(patterns_button(active));
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(patterns_btn_rect_id(), pat_resp.rect));
        if pat_resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(patterns_popup_id()));
        }
        render_patterns_popover(ui, &pat_resp, state, sink);

        ui.separator();

        // ---- swing slider: GROOVE [────] NN%  (continuous → SetGlobalSwing) ----
        ui.label(RichText::new("GROOVE").color(TEXT_MUTED));
        let mirror_swing = state.swing_pct();
        // Hoist the temp read OUT of any later data_mut closure (⊥ re-entrant
        // Context lock — parking_lot is non-reentrant; see grid.rs drag notes).
        let inflight = read_feel(&ctx).swing_inflight;
        let mut v = seed_swing(mirror_swing, inflight);
        let sl = ui.add(Slider::new(&mut v, 0.0..=SWING_MAX).show_value(false));
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(swing_rect_id(), sl.rect));
        if sl.changed() {
            let pct = clamp_swing(v);
            sink.push(swing_edit_command(pct));
            write_feel(&ctx, |f| f.swing_inflight = Some(pct));
        }
        if clear_swing_inflight(mirror_swing, inflight) {
            write_feel(&ctx, |f| f.swing_inflight = None);
        }
        ui.label(RichText::new(format!("{}%", (v * 100.0).round() as i32)).color(TEXT_MUTED));

        ui.separator();

        // ---- humanize (popover): NUANCE, accent while timing|velocity > 0 ----
        let humanize_active = state.humanize_timing() > 0.0 || state.humanize_velocity() > 0.0;
        let was_open = ui.memory(|m| m.is_popup_open(humanize_popup_id()));
        let hum_resp = ui.add(humanize_button(humanize_active));
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(humanize_btn_rect_id(), hum_resp.rect));
        if hum_resp.clicked() {
            // Seed the popover editing values from the mirror on OPEN
            // (⊥ stale leftovers on re-open; mirrors `FeelBar.swift:seedFromMirror`).
            if !was_open {
                let (t, vel) = (state.humanize_timing(), state.humanize_velocity());
                write_feel(&ctx, |f| {
                    f.humanize_timing = t;
                    f.humanize_velocity = vel;
                });
            }
            ui.memory_mut(|m| m.toggle_popup(humanize_popup_id()));
        }
        render_humanize_popover(ui, &hum_resp, sink);

        // ---- quantize cycle (right-aligned): GRID {label} → SetQuantizeGrain ----
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            let g = read_feel(ui.ctx()).grain;
            let q_resp = ui.add(quantize_button(g));
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(quantize_btn_rect_id(), q_resp.rect));
            if q_resp.clicked() {
                let ng = next_grain(g);
                sink.push(Command::SetQuantizeGrain { grain: ng });
                write_feel(ui.ctx(), |f| f.grain = ng);
            }
        });
    });
}

// ---- Popovers ----

/// Pattern bank popover (3×3, slots P1..P9). Active slot filled PRIMARY, queued
/// slot outlined PRIMARY. Click → `QueuePattern{index, NextBar}` + close.
fn render_patterns_popover(
    ui: &mut Ui,
    anchor: &egui::Response,
    state: &UiState,
    sink: &impl CommandSink,
) {
    popup_above_or_below_widget(
        ui,
        patterns_popup_id(),
        anchor,
        AboveOrBelow::Below,
        PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(180.0);
            #[cfg(test)]
            let mut slot_rects: Vec<(usize, egui::Rect)> = Vec::new();
            ui.vertical(|ui| {
                ui.label(RichText::new("Pattern Bank").color(TEXT_PRIMARY).strong());
                ui.separator();
                let active = state.active_pattern_index();
                let queued = state.queued_pattern;
                egui::Grid::new("stepforge.feel.pattern_bank")
                    .num_columns(3)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        for idx in 0..PATTERN_SLOTS {
                            let is_active = active == idx;
                            let is_queued = queued == Some(idx);
                            let mut btn = Button::new(
                                RichText::new(format!("P{}", idx + 1))
                                    .color(TEXT_PRIMARY)
                                    .strong(),
                            )
                            .min_size(Vec2::new(44.0, 30.0));
                            btn = if is_active {
                                btn.fill(PRIMARY)
                            } else {
                                btn.fill(SURFACE_HIGH)
                            };
                            let resp = ui.add(btn);
                            #[cfg(test)]
                            slot_rects.push((idx, resp.rect));
                            if is_queued {
                                ui.painter().rect_stroke(
                                    resp.rect,
                                    CornerRadius::same(4),
                                    Stroke::new(2.0_f32, PRIMARY),
                                    StrokeKind::Inside,
                                );
                            }
                            if resp.clicked() {
                                sink.push(queue_pattern_command(idx));
                                ui.memory_mut(|m| m.close_popup());
                            }
                            if (idx + 1) % 3 == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
            #[cfg(test)]
            ctx_insert_slot_rects(ui, slot_rects);
        },
    );
}

#[cfg(test)]
fn ctx_insert_slot_rects(ui: &Ui, rects: Vec<(usize, egui::Rect)>) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(pattern_slot_rects_id(), rects));
}

/// Humanize popover: Timing + Velocity sliders (0..=1) + Apply. The sliders edit
/// widget-local values seeded from the mirror on open; Apply commits them as a
/// single `SetHumanize` and closes (iOS `HumanizeEditor` + Apply button).
fn render_humanize_popover(ui: &mut Ui, anchor: &egui::Response, sink: &impl CommandSink) {
    popup_above_or_below_widget(
        ui,
        humanize_popup_id(),
        anchor,
        AboveOrBelow::Below,
        PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(220.0);
            let f = read_feel(ui.ctx());
            let mut timing = f.humanize_timing;
            let mut velocity = f.humanize_velocity;
            ui.vertical(|ui| {
                ui.label(RichText::new("Humanize").color(TEXT_PRIMARY).strong());
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Timing").color(TEXT_MUTED));
                    ui.add(Slider::new(&mut timing, 0.0..=1.0).show_value(false));
                    ui.label(
                        RichText::new(format!("{}%", (timing * 100.0).round() as i32))
                            .color(TEXT_MUTED),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Velocity").color(TEXT_MUTED));
                    ui.add(Slider::new(&mut velocity, 0.0..=1.0).show_value(false));
                    ui.label(
                        RichText::new(format!("{}%", (velocity * 100.0).round() as i32))
                            .color(TEXT_MUTED),
                    );
                });
                ui.separator();
                let apply = ui.add(
                    Button::new(RichText::new("Apply").color(TEXT_PRIMARY))
                        .fill(PRIMARY)
                        .min_size(Vec2::new(96.0, 0.0)),
                );
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|d| d.insert_temp(humanize_apply_rect_id(), apply.rect));
                if apply.clicked() {
                    sink.push(humanize_command(timing, velocity));
                    ui.memory_mut(|m| m.close_popup());
                }
            });
            // Persist the popover editing values each open frame (Apply commits
            // them; closing drops further writes, re-open reseeds from mirror).
            write_feel(ui.ctx(), |f| {
                f.humanize_timing = timing;
                f.humanize_velocity = velocity;
            });
        },
    );
}

// ---- Buttons ----

fn patterns_button(active: usize) -> Button<'static> {
    Button::new(RichText::new(format!("▦ PATTERNS  P{}", active + 1)).color(TEXT_MUTED))
        .fill(SURFACE_HIGH)
}

fn humanize_button(active: bool) -> Button<'static> {
    let color = if active { PRIMARY } else { TEXT_MUTED };
    Button::new(RichText::new("≈ NUANCE").color(color)).fill(SURFACE_HIGH)
}

fn quantize_button(g: QuantizeGrain) -> Button<'static> {
    Button::new(RichText::new(format!("GRID  {}", grain_label(g))).color(TEXT_PRIMARY))
        .fill(SURFACE_HIGH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use sequencer_engine::models::Session;

    #[derive(Default, Clone)]
    struct Rec(Arc<Mutex<Vec<Command>>>);
    impl CommandSink for Rec {
        fn push(&self, c: Command) {
            self.0.lock().unwrap().push(c);
        }
    }

    fn raw_input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::new(0.0, 0.0),
                egui::Vec2::new(1400.0, 800.0),
            )),
            ..Default::default()
        }
    }

    struct Harness {
        ctx: egui::Context,
        state: UiState,
        sink: Rec,
    }
    impl Harness {
        fn new(state: UiState) -> Self {
            Self {
                ctx: egui::Context::default(),
                state,
                sink: Rec::default(),
            }
        }
        fn frame(&self, raw: egui::RawInput) {
            let _ = self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    render_feel_bar(ui, &self.state, &self.sink);
                });
            });
        }
        fn idle(&self) {
            self.frame(raw_input());
        }
        fn click(&self, pos: egui::Pos2) {
            for pressed in [true, false] {
                let mut r = raw_input();
                r.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
                self.frame(r);
            }
        }
        fn rect(&self, id: Id) -> egui::Rect {
            self.idle();
            self.ctx
                .data(|d| d.get_temp::<egui::Rect>(id))
                .expect("rect recorded")
        }
        fn center(&self, id: Id) -> egui::Pos2 {
            self.rect(id).center()
        }
        fn slot_center(&self, idx: usize) -> egui::Pos2 {
            self.idle();
            let rects = self
                .ctx
                .data(|d| d.get_temp::<Vec<(usize, egui::Rect)>>(pattern_slot_rects_id()))
                .expect("slot rects recorded");
            rects
                .into_iter()
                .find(|(i, _)| *i == idx)
                .map(|(_, r)| r.center())
                .unwrap_or_else(|| panic!("slot {idx} rect not recorded"))
        }
        fn cmds(&self) -> Vec<Command> {
            self.sink.0.lock().unwrap().clone()
        }
    }

    // ---- pure oracle tests ----

    #[test]
    fn swing_helpers() {
        assert_eq!(clamp_swing(0.25), 0.25);
        assert_eq!(clamp_swing(-0.1), 0.0);
        assert_eq!(clamp_swing(0.9), SWING_MAX);
        assert!(matches!(
            swing_edit_command(0.9),
            Command::SetGlobalSwing { pct } if pct == SWING_MAX
        ));
        assert!(matches!(
            swing_edit_command(-1.0),
            Command::SetGlobalSwing { pct: 0.0 }
        ));
    }

    #[test]
    fn humanize_clamps_axes() {
        assert!(matches!(
            humanize_command(2.0, -1.0),
            Command::SetHumanize {
                timing: 1.0,
                velocity: 0.0
            }
        ));
        assert!(matches!(
            humanize_command(0.3, 0.7),
            Command::SetHumanize {
                timing: 0.3,
                velocity: 0.7
            }
        ));
    }

    #[test]
    fn queue_pattern_hardcodes_next_bar() {
        // iOS call site hardcodes .nextBar (FeelBar.swift:136) — faithful port.
        for idx in 0..PATTERN_SLOTS {
            assert!(matches!(
                queue_pattern_command(idx),
                Command::QueuePattern { index, quantize: QuantizeGrain::NextBar } if index == idx
            ));
        }
    }

    #[test]
    fn grain_label_and_cycle() {
        use QuantizeGrain::*;
        assert_eq!(grain_label(NextStep), "Step");
        assert_eq!(grain_label(NextBeat), "Beat");
        assert_eq!(grain_label(NextBar), "Bar");
        assert_eq!(grain_label(EndOfPattern), "Pat");
        // cycle: Step → Beat → Bar → Pat → Step
        assert_eq!(next_grain(NextStep), NextBeat);
        assert_eq!(next_grain(NextBeat), NextBar);
        assert_eq!(next_grain(NextBar), EndOfPattern);
        assert_eq!(next_grain(EndOfPattern), NextStep);
    }

    #[test]
    fn swing_inflight_seed_and_clear() {
        // in-flight wins over mirror while echo is pending
        assert_eq!(seed_swing(0.0, None), 0.0);
        assert_eq!(seed_swing(0.1, Some(0.3)), 0.3);
        // drop the override once the mirror catches up
        assert!(clear_swing_inflight(0.3, Some(0.3)));
        assert!(!clear_swing_inflight(0.1, Some(0.3)));
        assert!(!clear_swing_inflight(0.3, None));
    }

    #[test]
    fn uistate_feel_accessors() {
        let mut st = UiState::default();
        // pre-snapshot defaults
        assert_eq!(st.swing_pct(), 0.0);
        assert_eq!(st.humanize_timing(), 0.0);
        assert_eq!(st.humanize_velocity(), 0.0);
        assert_eq!(st.active_pattern_index(), 0);

        let s = Session {
            global_swing_pct: 0.42,
            humanize_timing: 0.5,
            humanize_velocity: 0.25,
            active_pattern_index: 3,
            ..Default::default()
        };
        st.session = Some(Arc::new(s));
        assert_eq!(st.swing_pct(), 0.42);
        assert_eq!(st.humanize_timing(), 0.5);
        assert_eq!(st.humanize_velocity(), 0.25);
        assert_eq!(st.active_pattern_index(), 3);
    }

    // ---- headless harness tests (e2e wiring) ----

    #[test]
    fn feel_render_no_session_no_panic() {
        let h = Harness::new(UiState::default()); // no session
        h.idle();
        // quantize cycle works without a session (widget-local grain)
        h.click(h.center(quantize_btn_rect_id()));
        assert!(matches!(
            h.cmds().as_slice(),
            [Command::SetQuantizeGrain { .. }]
        ));
    }

    #[test]
    fn swing_click_emits_setglobalswing() {
        // stopped at mirror 0.0; clicking the slider trough emits a clamped
        // SetGlobalSwing with a value strictly inside (0, SWING_MAX].
        let h = Harness::new(UiState::default());
        h.click(h.center(swing_rect_id()));
        match h.cmds().as_slice() {
            [Command::SetGlobalSwing { pct }] => {
                assert!(
                    *pct > 0.0 && *pct <= SWING_MAX,
                    "pct in (0, {SWING_MAX}], got {pct}"
                );
            }
            other => panic!("expected one SetGlobalSwing, got {other:?}"),
        }
    }

    #[test]
    fn quantize_cycle_advances_grain() {
        // default grain = NextBeat; each click emits the NEXT grain + advances.
        let h = Harness::new(UiState::default());
        h.click(h.center(quantize_btn_rect_id())); // NextBeat → NextBar
        h.click(h.center(quantize_btn_rect_id())); // NextBar  → EndOfPattern
        let cmds = h.cmds();
        assert!(matches!(
            cmds.as_slice(),
            [
                Command::SetQuantizeGrain {
                    grain: QuantizeGrain::NextBar
                },
                Command::SetQuantizeGrain {
                    grain: QuantizeGrain::EndOfPattern
                }
            ]
        ));
    }

    #[test]
    fn humanize_apply_emits_sethumanize() {
        // open popover (seeds editing values from mirror = 0,0) → Apply commits.
        let h = Harness::new(UiState::default());
        h.click(h.center(humanize_btn_rect_id())); // open
        h.click(h.center(humanize_apply_rect_id())); // Apply
        match h.cmds().as_slice() {
            [Command::SetHumanize { timing, velocity }] => {
                assert_eq!(*timing, 0.0);
                assert_eq!(*velocity, 0.0);
            }
            other => panic!("expected one SetHumanize, got {other:?}"),
        }
    }

    #[test]
    fn patterns_slot_emits_queuepattern_next_bar() {
        let h = Harness::new(UiState::default());
        h.click(h.center(patterns_btn_rect_id())); // open bank
        h.click(h.slot_center(2)); // queue slot P3 (idx 2)
        match h.cmds().as_slice() {
            [Command::QueuePattern { index, quantize }] => {
                assert_eq!(*index, 2);
                assert_eq!(*quantize, QuantizeGrain::NextBar);
            }
            other => panic!("expected one QueuePattern, got {other:?}"),
        }
    }
}
