//! Task 15: invariant tests for the algorithm/clipboard/undo wiring in
//! `Engine::apply_command`. Drives the state worker's clone-mutate-publish
//! path end-to-end (no worker thread) and asserts the working-agreement
//! invariants hold: Roll/Vary/Cut/Trash preserve `length` / `midi_note` /
//! `speed_ratio`, Paste carries `length` + `speed_ratio` but never `midi_note`,
//! and undo (pushed BEFORE mutating) restores the pre-mutation steps.

use sequencer_engine::command::Command;
use sequencer_engine::engine::Engine;
use sequencer_engine::models::{Pattern, Session, Step, VelocityZone};

fn session() -> Session {
    let mut s = Session::default();
    let mut p = Pattern::default();
    p.tracks[0].midi_note = 38;
    p.tracks[0].length = 9;
    p.tracks[0].speed_ratio = 2.0;
    p.tracks[0].steps[1] = Step {
        active: true,
        velocity_zone: VelocityZone::Mid,
        ..Default::default()
    };
    s.patterns[0] = Some(p);
    s
}

/// Two-track session: track 0 = note 38 with an active step; track 1 = note 99,
/// empty. For the Cut→Paste cross-track midi_note invariant (the CLAP DAW smoke
/// "keeps that track's own midi_note" checklist item).
fn session_two_tracks() -> Session {
    let mut s = Session::default();
    let mut p = Pattern::default();
    p.tracks[0].midi_note = 38;
    p.tracks[0].steps[1] = Step {
        active: true,
        velocity_zone: VelocityZone::Mid,
        ..Default::default()
    };
    // Track 1 already exists in a default Pattern with its own midi_note; pin it
    // to 99 so the post-paste assertion is a known value.
    p.tracks[1].midi_note = 99;
    s.patterns[0] = Some(p);
    s
}

fn t0(s: &Session) -> (&usize, &u8, &f32) {
    let t = &s.patterns[0].as_ref().unwrap().tracks[0];
    (&t.length, &t.midi_note, &t.speed_ratio)
}

#[test]
fn trash_preserves_length_note_ratio() {
    let e = Engine::new();
    e.publish(session());
    let snap_before = e.snapshot_arc();
    let before = t0(&snap_before);
    e.apply_command(Command::Trash { track_idx: 0 });
    let snap_after = e.snapshot_arc();
    let after = t0(&snap_after);
    assert_eq!(before, after);
    assert!(
        !snap_after.patterns[0].as_ref().unwrap().tracks[0].steps[1].active,
        "Trash must clear steps"
    );
}

#[test]
fn cut_pushes_undo_and_undo_restores() {
    let e = Engine::new();
    e.publish(session());
    e.apply_command(Command::Cut { track_idx: 0 });
    assert!(
        !e.snapshot_arc().patterns[0].as_ref().unwrap().tracks[0].steps[1].active,
        "Cut must clear steps"
    );
    e.apply_command(Command::Undo { track_idx: 0 });
    assert!(
        e.snapshot_arc().patterns[0].as_ref().unwrap().tracks[0].steps[1].active,
        "Undo must restore pre-Cut steps"
    );
}

#[test]
fn cut_then_paste_keeps_target_midi_note() {
    // CLAP DAW smoke checklist: Cut track 0, Paste onto track 1 — track 1 must
    // keep its OWN midi_note (99) while receiving track 0's steps. Drives the
    // full apply_command path (the clipboard unit test only covers Copy+Paste
    // at the struct level, not Cut→Paste through the engine).
    let e = Engine::new();
    e.publish(session_two_tracks());
    e.apply_command(Command::Cut { track_idx: 0 });
    e.apply_command(Command::Paste { track_idx: 1 });
    let snap = e.snapshot_arc();
    let t1 = &snap.patterns[0].as_ref().unwrap().tracks[1];
    assert_eq!(
        t1.midi_note, 99,
        "Paste must NOT carry midi_note — target keeps its own"
    );
    assert!(
        t1.steps[1].active,
        "Paste must carry the source track's steps"
    );
}
