//! Shared headless-render harness for the editor's widget tests. The grid,
//! `ActionDrawer`, and `NotePickerSheet` test modules each drove a
//! near-identical harness (ctx + `UiState` + recording sink + frame/idle/click/
//! press helpers); the two overlay modules' copies were byte-identical, so they
//! live here as [`Harness`]. The grid keeps its own `Harness` (it renders the
//! grid in isolation rather than the full editor), but reuses the shared
//! [`Rec`] + [`raw_input`].

use std::sync::{Arc, Mutex};

use egui::{Context, Modifiers, PointerButton, Pos2, RawInput, Rect};
use sequencer_engine::command::Command;

use crate::{render, CommandSink, UiState};

/// Recording [`CommandSink`] for tests — every emitted command is appended to an
/// inner `Vec`, read back via [`Rec::cmds`]. Shared by every widget test module.
#[derive(Default, Clone)]
pub(crate) struct Rec(Arc<Mutex<Vec<Command>>>);
impl CommandSink for Rec {
    fn push(&self, c: Command) {
        self.0.lock().unwrap().push(c);
    }
}
impl Rec {
    pub(crate) fn cmds(&self) -> Vec<Command> {
        self.0.lock().unwrap().clone()
    }
}

/// Fixed 1400×800 screen so every test lays out against the same geometry.
pub(crate) fn raw_input() -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(
            Pos2::new(0.0, 0.0),
            egui::Vec2::new(1400.0, 800.0),
        )),
        ..Default::default()
    }
}

/// Headless harness: a fresh [`Context`], a [`UiState`], and a recording sink,
/// driven one frame per call. `frame` runs the *full* editor [`render`]
/// (transport + feel + track-management + grid + overlays), so the overlay tests
/// see their floating `egui::Area`s. (The grid keeps its own `Harness` that
/// renders the grid in isolation; it reuses [`Rec`] / [`raw_input`].)
pub(crate) struct Harness {
    pub(crate) ctx: Context,
    pub(crate) state: UiState,
    pub(crate) sink: Rec,
}
impl Harness {
    pub(crate) fn new(state: UiState) -> Self {
        Self {
            ctx: Context::default(),
            state,
            sink: Rec::default(),
        }
    }
    pub(crate) fn frame(&self, raw: RawInput) {
        let _ = self
            .ctx
            .run(raw, |ctx| render(ctx, &self.state, &self.sink));
    }
    pub(crate) fn idle(&self) {
        self.frame(raw_input());
    }
    /// A floating `egui::Area`'s widgets don't receive clicks until the Area's
    /// interaction state has settled across a few frames (its rect must age into
    /// `memory.areas` so the press-target lookup resolves to the Area's layer).
    /// At 60 fps this is ~50 ms — invisible in a host — but a headless harness
    /// drives one frame per call, so an overlay just opened via `write` needs a
    /// few idle frames before its buttons click. The grid's ratchet gets this for
    /// free (its Alt+click open gesture is already two frames); a cold-open does
    /// not, so we settle explicitly.
    pub(crate) fn settle(&self) {
        for _ in 0..4 {
            self.idle();
        }
    }
    /// Press + release a primary click at `pos` across two frames (egui needs the
    /// release to register a click), with no modifier keys.
    pub(crate) fn click_primary(&self, pos: Pos2) {
        let mods = Modifiers::default();
        let mut press = raw_input();
        press.events.push(egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: mods,
        });
        self.frame(press);
        let mut release = raw_input();
        release.events.push(egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: mods,
        });
        self.frame(release);
    }
    pub(crate) fn press_key(&self, key: egui::Key) {
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
    pub(crate) fn cmds(&self) -> Vec<Command> {
        self.sink.cmds()
    }
}
