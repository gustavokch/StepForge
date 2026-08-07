//! Phase 3 §T T12 — PerformanceView. Port of the iOS
//! `app/StepForge/Features/Performance/PerformanceView.swift`.
//!
//! Pure UI (V4): reads [`UiState`], emits [`Command`]s, never mutates the
//! engine. The second editor mode (iOS `AppMode.performance`): a large
//! PLAY/STOP, a 3×3 pattern grid (EMPTY/FILLED/PLAYING/QUEUED), per-track
//! activity LEDs + mute toggles, and a quantize-grain selector.
//!
//! Cell state is derived purely from the mirror — three fields, strict
//! precedence QUEUED > PLAYING > FILLED > EMPTY: `queued_pattern == Some(idx)`
//! (QUEUED), `active_pattern_index == idx` (PLAYING), `patterns[idx].is_some()`
//! (FILLED), else EMPTY. No wall-clock drives state (iOS parity — grep for
//! TimelineView/animation in the iOS Performance dir returns nothing).
//!
//! Two editor-only visual touches over strict iOS parity (requested for the
//! CLAP port):
//! - a subtle PLAYING-cell border **pulse** via `ctx.time()` + `request_repaint`
//!   (iOS differentiates PLAYING/QUEUED by static color only);
//! - the follow-action next-destination glow is a static PRIMARY ring derived
//!   from the active pattern's follow_action via [`next_pattern_index`] (the
//!   iOS shadow, ported). `PlaySpecific`/`PlayRandom`/`None`/`Stop` glow nil.
//!
//! Loop progress is playhead-derived (track 0), NOT a wall clock — it advances
//! step-by-step as `Playhead` events land in the mirror.

use egui::{
    Button, Color32, CornerRadius, Label, ProgressBar, Response, RichText, Stroke, Ui, Vec2,
};
use sequencer_engine::command::Command;
use sequencer_engine::models::{FollowActionType, Pattern, QuantizeGrain, PATTERN_SLOTS};

#[cfg(test)]
use egui::{Context, Id, Rect};

use crate::grid::drum_name;
use crate::theme::{BORDER_WEAK, PRIMARY, SURFACE_HIGHEST, SURFACE_LOW, TEXT_MUTED, TEXT_PRIMARY};
use crate::{transport_action, CommandSink, UiState};

const GRAINS: [QuantizeGrain; 4] = [
    QuantizeGrain::NextStep,
    QuantizeGrain::NextBeat,
    QuantizeGrain::NextBar,
    QuantizeGrain::EndOfPattern,
];
const CELL_W: f32 = 150.0;
const CELL_H: f32 = 58.0;

// ---- test rect / state probes (cleared at the top of each render) ----
#[cfg(test)]
fn play_rect_id() -> Id {
    Id::new("stepforge.perf.play")
}
#[cfg(test)]
fn grain_rects_id() -> Id {
    Id::new("stepforge.perf.grains")
}
#[cfg(test)]
fn cell_rects_id() -> Id {
    Id::new("stepforge.perf.cells")
}
#[cfg(test)]
fn mute_rects_id() -> Id {
    Id::new("stepforge.perf.mutes")
}
#[cfg(test)]
fn options_rects_id() -> Id {
    Id::new("stepforge.perf.options")
}
#[cfg(test)]
fn state_probe_id() -> Id {
    Id::new("stepforge.perf.cell_states")
}

// ---- Pure helpers (headless oracle; ⊥ egui state) ----

/// Resolved per-cell visual state (the iOS EMPTY/FILLED/PLAYING/QUEUED chain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellState {
    Empty,
    Filled,
    Playing,
    Queued,
}

/// Resolve a cell's visible state. Pure port of `PerformanceView.swift:179-195`
/// (precedence QUEUED > PLAYING > FILLED > EMPTY). `filled` is
/// `patterns[idx].is_some()`; `active` is `active_pattern_index`; `queued` is
/// `state.queued_pattern`. NOTE: PLAYING wins over filled (iOS parity) — an
/// active slot whose pattern was cleared still shows PLAYING.
pub(crate) fn cell_state(
    idx: usize,
    active: usize,
    queued: Option<usize>,
    filled: bool,
) -> CellState {
    if queued == Some(idx) {
        CellState::Queued
    } else if active == idx {
        CellState::Playing
    } else if filled {
        CellState::Filled
    } else {
        CellState::Empty
    }
}

/// The pattern index the active pattern's follow-action will transition to, for
/// the next-destination glow. Aligned to the engine's RT follow-action
/// (`engine.rs:~714-728`) — NOT the iOS `SessionMirror.nextPatternIndex(from:)`
/// port the previous version claimed (the iOS mirror filters to FILLED slots;
/// the engine does not). `None`/`Stop`/`PlayRandom` → `None`; `PlayNext` →
/// `(active + 1) % PATTERN_SLOTS`; `PlayPrevious` →
/// `(active + PATTERN_SLOTS - 1) % PATTERN_SLOTS` — both wrap over ALL slots, so
/// the glow may land on an empty cell (the engine's `PlayNext` can likewise
/// switch to a `None` slot — a separate latent engine-side concern, out of
/// scope here; the glow now honestly reflects that). `PlaySpecific(id)` → the
/// slot whose `Pattern.id` matches, else `None` (matches the engine's
/// `.position(...).unwrap_or(active)` resolution, but returns `None` so the UI
/// doesn't glow on the active cell itself when the target is missing).
pub(crate) fn next_pattern_index(
    patterns: &[Option<Pattern>; PATTERN_SLOTS],
    active: usize,
    action: &FollowActionType,
) -> Option<usize> {
    match action {
        FollowActionType::None | FollowActionType::Stop | FollowActionType::PlayRandom => None,
        FollowActionType::PlayNext => Some((active + 1) % PATTERN_SLOTS),
        FollowActionType::PlayPrevious => Some((active + PATTERN_SLOTS - 1) % PATTERN_SLOTS),
        FollowActionType::PlaySpecific(id) => {
            (0..PATTERN_SLOTS).find(|&i| patterns[i].as_ref().is_some_and(|p| p.id == *id))
        }
    }
}

/// PLAYING-cell pulse alpha in `[0,1]` for a wall-clock time (seconds). Smooth
/// sine ~0.75 Hz (an editor-only enhancement; iOS has no animation). Pure over
/// `f64` time → testable without driving egui. #38: a non-finite wall-clock
/// (suspend/resume, platform clock glitch) falls back to a neutral mid-pulse
/// `0.5` so the active cell's fill/stroke don't go NaN.
pub(crate) fn pulse_alpha(time: f64) -> f32 {
    let raw = (time * std::f64::consts::TAU * 0.75).sin() * 0.5 + 0.5;
    if raw.is_finite() {
        raw as f32
    } else {
        0.5 // neutral mid-pulse — keeps the cell visible (not black)
    }
}

/// Loop progress ratio for the active cell, anchored to track 0 (iOS parity —
/// `PerformanceView.swift:288-293`): `(playheads[0] + 1) / tracks[0].length`.
/// `[0,1]`. Pure over [`UiState`].
pub(crate) fn loop_progress(state: &UiState) -> f32 {
    let step = state.playheads.get(&0).copied().unwrap_or(0);
    let total = state.tracks().first().map(|t| t.length).unwrap_or(16) as f32;
    ((step as f32 + 1.0) / total.max(1.0)).clamp(0.0, 1.0)
}

fn state_label(st: CellState) -> &'static str {
    match st {
        CellState::Empty => "EMPTY",
        CellState::Filled => "FILLED",
        CellState::Playing => "PLAYING",
        CellState::Queued => "QUEUED",
    }
}

fn cell_text_color(st: CellState) -> Color32 {
    match st {
        CellState::Empty => TEXT_MUTED,
        CellState::Filled | CellState::Playing | CellState::Queued => TEXT_PRIMARY,
    }
}

/// Cell fill. PLAYING pulses a dark-orange tint via `gamma_multiply` (brightens
/// with `pulse`, 0..1) — no `Color32::lerp` to stay off uncertain 0.31 API.
fn cell_fill(st: CellState, pulse: f32) -> Color32 {
    match st {
        CellState::Empty => SURFACE_LOW,
        CellState::Filled => SURFACE_HIGHEST,
        CellState::Queued => SURFACE_HIGHEST,
        CellState::Playing => PRIMARY.gamma_multiply(0.20 + 0.25 * pulse),
    }
}

/// Cell stroke. PLAYING/QUEUED get a PRIMARY ring (PLAYING width pulses). A
/// next-destination glow overrides with a solid 3px PRIMARY ring — EXCEPT on a
/// Playing cell: if the active pattern's follow-action targets itself
/// (`PlaySpecific(own_id)`), the PLAYING pulse must win, otherwise the cell
/// collapses to a flat 3px ring and the "currently playing" signal is lost.
fn cell_stroke(st: CellState, is_next: bool, pulse: f32) -> Stroke {
    if is_next && st != CellState::Playing {
        return Stroke::new(3.0_f32, PRIMARY);
    }
    let (base_w, base_c) = match st {
        CellState::Empty | CellState::Filled => (1.0, BORDER_WEAK),
        CellState::Playing => (2.0, PRIMARY),
        CellState::Queued => (1.5, PRIMARY),
    };
    let w = base_w
        + if st == CellState::Playing {
            1.5 * pulse
        } else {
            0.0
        };
    Stroke::new(w, base_c)
}

fn patterns_slice(state: &UiState) -> Option<&[Option<Pattern>; PATTERN_SLOTS]> {
    state.session.as_deref().map(|s| &s.patterns)
}

// ---- The view ----

/// Render the PerformanceView. No-op (no panic) when there is no session.
pub(crate) fn render_performance_view(ui: &mut Ui, state: &UiState, sink: &impl CommandSink) {
    let ctx = ui.ctx().clone();

    let Some(patterns) = patterns_slice(state) else {
        ui.label(RichText::new("no session").color(TEXT_MUTED));
        return;
    };
    let active = state.active_pattern_index();
    let queued = state.queued_pattern;
    // Follow-action next-destination for the glow (active pattern's follow_action).
    let next_dest = patterns
        .get(active)
        .and_then(Option::as_ref)
        .and_then(|pat| next_pattern_index(patterns, active, &pat.follow_action.action));

    #[cfg(test)]
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(usize, CellState)>>(state_probe_id())
            .clear()
    });

    // ---- Row 1: large PLAY/STOP + quantize pills ----
    ui.horizontal(|ui| {
        let playing = state.playing;
        let r = ui.add_sized(Vec2::new(150.0, 50.0), play_button(playing));
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(play_rect_id(), r.rect));
        if r.clicked() {
            sink.push(transport_action(playing));
        }

        ui.separator();
        ui.label(RichText::new("GRID").color(TEXT_MUTED));
        let cur_grain = crate::feel::read_grain(&ctx);
        #[cfg(test)]
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<(QuantizeGrain, Rect)>>(grain_rects_id())
                .clear()
        });
        for g in GRAINS {
            let is_cur = cur_grain == g;
            let r = grain_pill(ui, g, is_cur);
            #[cfg(test)]
            ctx.data_mut(|d| {
                d.get_temp_mut_or_default::<Vec<(QuantizeGrain, Rect)>>(grain_rects_id())
                    .push((g, r.rect))
            });
            if r.clicked() {
                sink.push(Command::SetQuantizeGrain { grain: g });
                crate::feel::write_grain(&ctx, g);
            }
        }
    });

    ui.separator();

    // ---- 3×3 pattern grid ----
    #[cfg(test)]
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(cell_rects_id())
            .clear()
    });
    #[cfg(test)]
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(options_rects_id())
            .clear()
    });
    let grain = crate::feel::read_grain(&ctx);
    for row in 0..3 {
        ui.horizontal(|ui| {
            for col in 0..3 {
                let idx = row * 3 + col;
                let filled = patterns.get(idx).is_some_and(|p| p.is_some());
                let st = cell_state(idx, active, queued, filled);
                #[cfg(test)]
                ctx.data_mut(|d| {
                    d.get_temp_mut_or_default::<Vec<(usize, CellState)>>(state_probe_id())
                        .push((idx, st))
                });

                let is_active = st == CellState::Playing;
                let is_next = next_dest == Some(idx);
                // Pulse only while rolling on the active cell → request a repaint
                // so the animation advances (invisible cost off-playback).
                if is_active && state.playing {
                    ctx.request_repaint();
                }
                let pulse = if is_active && state.playing {
                    pulse_alpha(ctx.input(|i| i.time))
                } else {
                    0.0
                };

                ui.vertical(|ui| {
                    ui.set_min_width(CELL_W);
                    ui.set_max_width(CELL_W);
                    ui.add(
                        Label::new(
                            RichText::new(state_label(st))
                                .color(cell_text_color(st))
                                .small(),
                        )
                        .truncate(),
                    );
                    let main = ui.add_sized(
                        Vec2::new(CELL_W, CELL_H),
                        Button::new(
                            RichText::new(format!("P{}", idx + 1))
                                .strong()
                                .color(cell_text_color(st)),
                        )
                        .fill(cell_fill(st, pulse))
                        .stroke(cell_stroke(st, is_next, pulse))
                        .corner_radius(CornerRadius::same(6)),
                    );
                    #[cfg(test)]
                    ctx.data_mut(|d| {
                        d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(cell_rects_id())
                            .push((idx, main.rect))
                    });
                    if main.clicked() {
                        match st {
                            CellState::Playing => sink.push(Command::RetriggerPattern {
                                quantize: QuantizeGrain::NextBeat,
                            }),
                            CellState::Queued => sink.push(Command::CancelQueuedPattern),
                            CellState::Filled => sink.push(Command::QueuePattern {
                                index: idx,
                                quantize: grain,
                            }),
                            CellState::Empty => {}
                        }
                    }
                    // Right-click (mouse long-press analog) on the active cell
                    // retriggers with the finer NextStep grain (iOS parity).
                    if main.secondary_clicked() && st == CellState::Playing {
                        sink.push(Command::RetriggerPattern {
                            quantize: QuantizeGrain::NextStep,
                        });
                    }

                    // Loop progress bar (active + playing); playhead-derived.
                    if is_active && state.playing {
                        ui.add_sized(
                            Vec2::new(CELL_W, 4.0),
                            ProgressBar::new(loop_progress(state)),
                        );
                    }

                    // Gear → PatternOptionsSheet (filled cells only; iOS
                    // long-press + gearshape button, unified to one `…` button).
                    if filled {
                        let gear = ui.add_sized(
                            Vec2::new(CELL_W, 18.0),
                            Button::new(RichText::new("…").color(TEXT_MUTED)).fill(SURFACE_LOW),
                        );
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(options_rects_id())
                                .push((idx, gear.rect))
                        });
                        if gear.clicked() {
                            crate::pattern_options::open(&ctx, idx, state);
                        }
                    }
                });
            }
        });
    }

    ui.separator();

    // ---- Track activity LEDs + mute toggles (active pattern) ----
    #[cfg(test)]
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(mute_rects_id())
            .clear()
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("TRACKS").color(TEXT_MUTED));
        for (tidx, track) in state.tracks().iter().enumerate() {
            ui.vertical(|ui| {
                // LED: hit gate — lit only while rolling AND the per-track
                // playhead lands on an active step (iOS parity; no note-on event
                // exists, so playheads is the only per-track liveness signal).
                let playhead = state.playheads.get(&tidx).copied().unwrap_or(0);
                let is_hit =
                    state.playing && playhead < track.steps.len() && track.steps[playhead].active;
                let led_color = if is_hit { PRIMARY } else { SURFACE_LOW };
                ui.add(Label::new(RichText::new("●").color(led_color)));
                ui.add(
                    Label::new(
                        RichText::new(drum_name(track.midi_note))
                            .color(TEXT_MUTED)
                            .small(),
                    )
                    .truncate(),
                );
                let (mute_label, mute_color) = if track.muted {
                    ("MUTED", PRIMARY)
                } else {
                    ("mute", TEXT_PRIMARY)
                };
                let mb = ui.add_sized(
                    Vec2::new(54.0, 18.0),
                    Button::new(RichText::new(mute_label).small().color(mute_color))
                        .fill(SURFACE_HIGHEST),
                );
                #[cfg(test)]
                ctx.data_mut(|d| {
                    d.get_temp_mut_or_default::<Vec<(usize, Rect)>>(mute_rects_id())
                        .push((tidx, mb.rect))
                });
                if mb.clicked() {
                    sink.push(Command::SetTrackMuted {
                        track_idx: tidx,
                        muted: !track.muted,
                    });
                }
            });
        }
    });
}

fn play_button(playing: bool) -> Button<'static> {
    let (glyph, color) = if playing {
        ("■ STOP", PRIMARY)
    } else {
        ("▶ PLAY", TEXT_PRIMARY)
    };
    Button::new(RichText::new(glyph).strong().color(color))
        .fill(SURFACE_HIGHEST)
        .corner_radius(CornerRadius::same(6))
}

fn grain_pill(ui: &mut Ui, g: QuantizeGrain, is_cur: bool) -> Response {
    let txt = RichText::new(crate::feel::grain_label(g))
        .color(if is_cur { PRIMARY } else { TEXT_PRIMARY })
        .strong();
    ui.add(
        Button::new(txt)
            .fill(if is_cur { SURFACE_HIGHEST } else { SURFACE_LOW })
            .min_size(Vec2::new(40.0, 0.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Harness;
    use sequencer_engine::event::EngineEvent;
    use sequencer_engine::models::{Pattern, QuantizeGrain, Session, PATTERN_SLOTS};
    use std::sync::Arc;

    fn fixture() -> UiState {
        UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        }
    }

    fn perf_harness(state: UiState) -> Harness {
        let h = Harness::new(state);
        crate::write_mode(&h.ctx, crate::AppMode::Performance);
        h
    }

    fn cell_states(ctx: &Context) -> Vec<CellState> {
        let mut out = vec![CellState::Empty; PATTERN_SLOTS];
        for (idx, st) in ctx
            .data(|d| d.get_temp::<Vec<(usize, CellState)>>(state_probe_id()))
            .unwrap_or_default()
        {
            if idx < PATTERN_SLOTS {
                out[idx] = st;
            }
        }
        out
    }
    fn cell_center(ctx: &Context, idx: usize) -> Rect {
        ctx.data(|d| d.get_temp::<Vec<(usize, Rect)>>(cell_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(i, _)| *i == idx)
            .map(|(_, r)| r)
            .unwrap_or_else(|| panic!("cell {idx} rect recorded"))
    }
    fn grain_center(ctx: &Context, want: QuantizeGrain) -> Rect {
        ctx.data(|d| d.get_temp::<Vec<(QuantizeGrain, Rect)>>(grain_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(g, _)| *g == want)
            .map(|(_, r)| r)
            .unwrap_or_else(|| panic!("grain {want:?} rect recorded"))
    }
    fn mute_center(ctx: &Context, tidx: usize) -> Rect {
        ctx.data(|d| d.get_temp::<Vec<(usize, Rect)>>(mute_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(i, _)| *i == tidx)
            .map(|(_, r)| r)
            .unwrap_or_else(|| panic!("mute {tidx} rect recorded"))
    }
    fn gear_center(ctx: &Context, idx: usize) -> Rect {
        ctx.data(|d| d.get_temp::<Vec<(usize, Rect)>>(options_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(i, _)| *i == idx)
            .map(|(_, r)| r)
            .unwrap_or_else(|| panic!("gear {idx} rect recorded"))
    }

    fn make_patterns(filled: usize) -> [Option<Pattern>; PATTERN_SLOTS] {
        let mut p: [Option<Pattern>; PATTERN_SLOTS] = Default::default();
        for slot in p.iter_mut().take(filled.min(PATTERN_SLOTS)) {
            *slot = Some(Pattern::default());
        }
        p
    }

    // ---- pure oracle tests ----

    #[test]
    fn cell_state_precedence() {
        use CellState::*;
        // QUEUED beats PLAYING beats FILLED beats EMPTY.
        assert_eq!(cell_state(0, 0, Some(0), true), Queued); // queued+active+filled
        assert_eq!(cell_state(0, 0, None, true), Playing); // active+filled
        assert_eq!(cell_state(0, 0, None, false), Playing); // active wins over not-filled (iOS)
        assert_eq!(cell_state(1, 0, None, true), Filled);
        assert_eq!(cell_state(1, 0, Some(1), false), Queued); // queued even if empty
        assert_eq!(cell_state(1, 0, None, false), Empty);
    }

    #[test]
    fn next_pattern_index_all_variants() {
        let p = make_patterns(9); // all filled
        assert_eq!(next_pattern_index(&p, 0, &FollowActionType::None), None);
        assert_eq!(next_pattern_index(&p, 0, &FollowActionType::Stop), None);
        assert_eq!(
            next_pattern_index(&p, 0, &FollowActionType::PlayRandom),
            None
        );
        // PlayNext / PlayPrevious wrap over ALL 9 slots (engine-aligned: simple
        // `(active ± 1) % PATTERN_SLOTS` over the full array, no filled filter).
        assert_eq!(
            next_pattern_index(&p, 0, &FollowActionType::PlayNext),
            Some(1)
        );
        assert_eq!(
            next_pattern_index(&p, 5, &FollowActionType::PlayNext),
            Some(6)
        );
        assert_eq!(
            next_pattern_index(&p, 8, &FollowActionType::PlayNext),
            Some(0)
        ); // wrap
        assert_eq!(
            next_pattern_index(&p, 0, &FollowActionType::PlayPrevious),
            Some(8)
        ); // wrap back
        assert_eq!(
            next_pattern_index(&p, 3, &FollowActionType::PlayPrevious),
            Some(2)
        );

        // PlaySpecific resolves to the slot whose Pattern.id matches.
        let target_id = p[4].as_ref().unwrap().id;
        assert_eq!(
            next_pattern_index(&p, 0, &FollowActionType::PlaySpecific(target_id)),
            Some(4)
        );
        // PlaySpecific not found → None (a uuid not on any slot).
        let stranger = Pattern::default().id;
        assert_eq!(
            next_pattern_index(&p, 0, &FollowActionType::PlaySpecific(stranger)),
            None
        );
    }

    #[test]
    fn next_pattern_index_wraps_over_all_slots_engine_aligned() {
        // Engine parity with `engine.rs:~715-720`: PlayNext/PlayPrevious wrap
        // over ALL slots — no filled filter, no <2-filled gate. Empty slots
        // are valid destinations (the engine can switch to a None slot — a
        // latent engine-side concern, out of scope here; the glow now honestly
        // reflects that).
        let p = make_patterns(3); // slots 0,1,2 filled; 3..8 empty
                                  // Adjacent empty slot is a valid next destination (NOT skipped).
        assert_eq!(
            next_pattern_index(&p, 2, &FollowActionType::PlayNext),
            Some(3)
        );
        // Wrap forward across the empty tail to slot 0.
        assert_eq!(
            next_pattern_index(&p, 8, &FollowActionType::PlayNext),
            Some(0)
        );
        // PlayPrevious from 0 wraps to the last slot (8) even though it's empty.
        assert_eq!(
            next_pattern_index(&p, 0, &FollowActionType::PlayPrevious),
            Some(8)
        );
        // PlayPrevious with an empty neighbor steps into it (NOT skipped).
        assert_eq!(
            next_pattern_index(&p, 4, &FollowActionType::PlayPrevious),
            Some(3)
        );
        // A single filled slot still advances — the old <2-filled gate is gone.
        let one = make_patterns(1);
        assert_eq!(
            next_pattern_index(&one, 0, &FollowActionType::PlayNext),
            Some(1)
        );
        assert_eq!(
            next_pattern_index(&one, 0, &FollowActionType::PlayPrevious),
            Some(8)
        );
    }

    #[test]
    fn cell_stroke_playing_self_target_keeps_pulse() {
        // Self-targeting PlaySpecific: `is_next && Playing` — must keep the
        // pulsing PLAYING stroke (2.0 + 1.5*pulse), NOT collapse to a flat 3.0
        // next-destination ring. The "currently playing" signal must survive.
        for pulse in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            let s = cell_stroke(CellState::Playing, true, pulse);
            assert!(
                (s.width - (2.0 + 1.5 * pulse)).abs() < 1e-6,
                "Playing cell must pulse even when is_next: pulse={pulse}, width={}",
                s.width
            );
            assert_eq!(
                s.color, PRIMARY,
                "Playing cell keeps PRIMARY color even when is_next"
            );
        }
        // Sanity: at peak pulse the PLAYING width (3.5) is distinct from the
        // flat 3.0 destination ring.
        let s_peak = cell_stroke(CellState::Playing, true, 1.0);
        assert!((s_peak.width - 3.5).abs() < 1e-6);
    }

    #[test]
    fn cell_stroke_next_destination_non_playing_is_3px() {
        // Non-playing next-destination cells get the solid 3px PRIMARY glow
        // (unchanged behavior for the glow itself).
        for pulse in [0.0_f32, 0.5, 1.0] {
            assert_eq!(cell_stroke(CellState::Filled, true, pulse).width, 3.0);
            assert_eq!(cell_stroke(CellState::Queued, true, pulse).width, 3.0);
            assert_eq!(cell_stroke(CellState::Empty, true, pulse).width, 3.0);
            // All destination rings are PRIMARY.
            assert_eq!(cell_stroke(CellState::Filled, true, pulse).color, PRIMARY);
        }
        // Without is_next, no 3px glow on non-playing cells.
        assert_eq!(cell_stroke(CellState::Filled, false, 0.5).width, 1.0);
    }

    #[test]
    fn pulse_alpha_range_and_zero() {
        for i in 0..200 {
            let t = i as f64 * 0.07;
            let a = pulse_alpha(t);
            assert!((0.0..=1.0).contains(&a), "pulse out of range at {t}: {a}");
        }
        assert!((pulse_alpha(0.0) - 0.5).abs() < 1e-6); // sin(0)=0 → 0.5
    }

    #[test]
    fn pulse_alpha_is_finite_for_non_finite_time() {
        // #38: a suspend/resume or platform clock glitch can feed a non-finite
        // wall-clock into pulse_alpha. The result must stay finite (and in range)
        // so the active cell's gamma_multiply fill and stroke width don't go NaN.
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let a = pulse_alpha(bad);
            assert!(a.is_finite(), "pulse_alpha({bad:?}) = {a} must be finite");
            assert!(
                (0.0..=1.0).contains(&a),
                "pulse_alpha({bad:?}) = {a} must stay in [0,1]"
            );
        }
        // A normal time still produces a normal finite alpha.
        assert!(pulse_alpha(0.0).is_finite());
        assert!((0.0..=1.0).contains(&pulse_alpha(1.0)));
    }

    #[test]
    fn cell_fill_and_stroke_finite_under_guarded_pulse() {
        // #38: belt-and-suspenders — feeding the guarded (finite) pulse into the
        // PLAYING paint helpers must yield finite fill + stroke (no NaN to
        // gamma_multiply / tessellation). The guarded pulse must stay finite;
        // the stroke width (f32) must stay finite; and the fill must NOT
        // degenerate to the all-zero black that a NaN factor would produce via
        // `gamma_multiply`'s saturating `as u8` cast (PRIMARY is non-black, so a
        // finite positive factor keeps its tint non-black).
        let pulse = pulse_alpha(f64::NAN);
        assert!(pulse.is_finite(), "guarded pulse must stay finite: {pulse}");
        let fill = cell_fill(CellState::Playing, pulse);
        assert!(
            fill.r() > 0 || fill.g() > 0 || fill.b() > 0,
            "fill must not degenerate to black: {fill:?}"
        );
        let stroke = cell_stroke(CellState::Playing, false, pulse);
        assert!(stroke.width.is_finite());
    }

    #[test]
    fn loop_progress_track0_anchored() {
        let mut st = fixture();
        st.apply_playhead(0, 3);
        let total = st.tracks().first().unwrap().length as f32;
        assert!((loop_progress(&st) - (4.0 / total)).abs() < 1e-6);
        st.apply_playhead(0, (total as usize) - 1);
        assert!((loop_progress(&st) - 1.0).abs() < 1e-6); // last step → full
    }

    // ---- headless harness tests (e2e wiring) ----

    #[test]
    fn play_stop_reflects_actual_state() {
        // Stopped (session loaded, not playing) → click emits Play.
        let h = perf_harness(fixture());
        h.settle();
        let play = h
            .ctx
            .data(|d| d.get_temp::<Rect>(play_rect_id()))
            .expect("play rect recorded")
            .center();
        h.click_primary(play);
        assert!(matches!(h.cmds().as_slice(), [Command::Play]));

        // Playing → Stop.
        let h2 = perf_harness(UiState {
            playing: true,
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        });
        h2.settle();
        let play = h2
            .ctx
            .data(|d| d.get_temp::<Rect>(play_rect_id()))
            .expect("play rect recorded")
            .center();
        h2.click_primary(play);
        assert!(matches!(h2.cmds().as_slice(), [Command::Stop]));
    }

    #[test]
    fn grain_pill_emits_set_quantize_grain_and_shares_slot() {
        let h = perf_harness(fixture());
        h.settle();
        // Click the "Bar" pill.
        h.click_primary(grain_center(&h.ctx, QuantizeGrain::NextBar).center());
        assert!(matches!(
            h.cmds().as_slice(),
            [Command::SetQuantizeGrain {
                grain: QuantizeGrain::NextBar
            }]
        ));
        // Shared with the FeelBar GRID slot: a fresh FeelBar render now reads Bar.
        assert_eq!(crate::feel::read_grain(&h.ctx), QuantizeGrain::NextBar);
    }

    #[test]
    fn filled_cell_click_emits_queuepattern_with_live_grain() {
        let h = perf_harness(fixture());
        // Set the grain to EndOfPattern so the queue carries it (not the default).
        crate::feel::write_grain(&h.ctx, QuantizeGrain::EndOfPattern);
        h.settle();
        // P4 (idx 3) is filled, not active (active is P1/idx 0) → QueuePattern.
        h.click_primary(cell_center(&h.ctx, 3).center());
        assert!(matches!(
            h.cmds().as_slice(),
            [Command::QueuePattern {
                index: 3,
                quantize: QuantizeGrain::EndOfPattern
            }]
        ));
    }

    #[test]
    fn queued_cell_click_emits_cancel() {
        let mut st = fixture();
        st.queued_pattern = Some(3);
        let h = perf_harness(st);
        h.settle();
        assert_eq!(cell_states(&h.ctx)[3], CellState::Queued);
        h.click_primary(cell_center(&h.ctx, 3).center());
        assert!(matches!(
            h.cmds().as_slice(),
            [Command::CancelQueuedPattern]
        ));
    }

    #[test]
    fn active_cell_click_retriggers_next_beat() {
        // Default active = idx 0. Click it → RetriggerPattern{NextBeat} (iOS tap).
        let h = perf_harness(fixture());
        h.settle();
        assert_eq!(cell_states(&h.ctx)[0], CellState::Playing);
        h.click_primary(cell_center(&h.ctx, 0).center());
        assert!(matches!(
            h.cmds().as_slice(),
            [Command::RetriggerPattern {
                quantize: QuantizeGrain::NextBeat
            }]
        ));
    }

    #[test]
    fn active_cell_right_click_retriggers_next_step() {
        let h = perf_harness(fixture());
        h.settle();
        // Right-click (secondary) on the active cell → finer NextStep grain.
        let rect = cell_center(&h.ctx, 0);
        let pos = rect.center();
        // Press + release secondary across two frames (mirrors click_primary).
        for pressed in [true, false] {
            let mut r = crate::test_support::raw_input();
            r.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
            h.frame(r);
        }
        assert!(matches!(
            h.cmds().as_slice(),
            [Command::RetriggerPattern {
                quantize: QuantizeGrain::NextStep
            }]
        ));
    }

    #[test]
    fn mute_toggle_emits_set_track_muted() {
        let h = perf_harness(fixture());
        h.settle();
        // Track 0 unmuted by default → toggle to muted.
        h.click_primary(mute_center(&h.ctx, 0).center());
        assert!(matches!(
            h.cmds().as_slice(),
            [Command::SetTrackMuted {
                track_idx: 0,
                muted: true
            }]
        ));
    }

    #[test]
    fn gear_click_opens_pattern_options_stays_open() {
        // The gear `…` click opens the sheet; the opening click must NOT
        // self-dismiss on its opening frame (the open-frame guard). Target set.
        let h = perf_harness(fixture());
        h.settle();
        h.click_primary(gear_center(&h.ctx, 1).center());
        assert_eq!(
            crate::pattern_options::read(&h.ctx).target,
            Some(1),
            "gear click must open the sheet, not self-dismiss on the opening frame"
        );
    }

    #[test]
    fn render_no_session_no_panic() {
        let h = perf_harness(UiState::default()); // no session
        h.idle();
        h.settle();
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn cell_highlights_track_real_event_order_through_apply() {
        // Mirror-glue guard: replay the REAL event order (Queued →
        // LoopCountChanged → Switched) through `UiState::apply` — NOT direct
        // field seeding — then render and assert the PerformanceView's cell-state
        // probe tracks the mirror at each step. (The T11 Undo bug —
        // `FullSnapshot` wiping `undo_available` — was invisible to a
        // seeded-state test; this is the same class of guard for the pattern
        // path. The handoff's exact Queued→LoopCountChanged→Switched order was
        // not exercised anywhere before T12.)
        let mut h = perf_harness(fixture());
        h.settle();

        // Active is P1 (idx 0). Queue P4.
        h.state.apply(&EngineEvent::PatternQueued {
            index: 3,
            quantize: QuantizeGrain::NextBar,
        });
        h.idle();
        let states = cell_states(&h.ctx);
        assert_eq!(states[0], CellState::Playing, "P1 still playing");
        assert_eq!(
            states[3],
            CellState::Queued,
            "P4 queued (precedence over filled)"
        );

        // A loop-count tick lands while queued — must NOT flip any cell state.
        h.state
            .apply(&EngineEvent::PatternLoopCountChanged { count: 1 });
        h.idle();
        let states = cell_states(&h.ctx);
        assert_eq!(states[0], CellState::Playing);
        assert_eq!(states[3], CellState::Queued);

        // The switch fires at the quantize boundary: P4 becomes active, queue clears.
        h.state.apply(&EngineEvent::PatternSwitched { index: 3 });
        h.idle();
        let states = cell_states(&h.ctx);
        assert_eq!(states[3], CellState::Playing, "P4 now playing");
        assert_eq!(states[0], CellState::Filled, "P1 back to filled");
        assert_eq!(h.state.queued_pattern, None);
        assert_eq!(
            h.state.pattern_loop_count, 0,
            "PatternSwitched resets the loop count"
        );
    }
}
