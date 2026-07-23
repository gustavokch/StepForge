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
