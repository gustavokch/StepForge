//! Integration test for `engine::process` — the RT tick. Drives the dispatch
//! math end-to-end against a real `Engine` snapshot (no worker thread): asserts
//! a step-0 Accent hit pushes a MidiMsg (note 36) and a Playhead event lands on
//! the hot channel. RT-safety is audited in the source, not exercised here.

use sequencer_engine::engine::{process, Engine, RtState};
use sequencer_engine::models::{Pattern, Session, Step, VelocityZone};

fn session_with_one_hit() -> Session {
    let mut s = Session::default();
    let mut p = Pattern::default();
    p.tracks[0].steps[0] = Step {
        active: true,
        velocity_zone: VelocityZone::Accent,
        ..Step::default()
    };
    p.tracks[0].midi_note = 36;
    s.patterns[0] = Some(p);
    s.bpm = 120.0; // 16th period = 60/120/4 = 0.125s = 125_000us
    s
}

#[test]
fn process_advances_playhead_and_emits_note_on() {
    let eng = Engine::new();
    eng.publish(session_with_one_hit());
    let mut rt = RtState::new(1);
    eng.begin_play(&mut rt);

    let period = 125_000u64;
    let mut notes = 0;
    for i in 0..4 {
        let outcome = process(
            &mut rt,
            &eng.snapshot_arc(),
            true,
            i * period,
            &eng.midi,
            &eng.hot_events,
        );
        notes += outcome.notes_pushed;
    }
    // Only step 0 is active; ratio 1.0 fires one step per tick. Tick 0 lands on
    // step 0 (Accent) -> one note-on across the four ticks.
    assert!(notes >= 1, "at least the step-0 note-on");
    assert_eq!(eng.midi.dequeue().map(|m| m.note), Some(36));
    // A Playhead event was emitted on the hot channel.
    let slot = eng.hot_events.dequeue().expect("a playhead");
    let _ev: sequencer_engine::event::EngineEvent =
        postcard::from_bytes(&slot.bytes[..slot.len as usize]).unwrap();
}
