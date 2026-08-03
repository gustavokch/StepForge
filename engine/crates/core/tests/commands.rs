use sequencer_engine::command::Command;
use sequencer_engine::engine::Engine;
use sequencer_engine::event::EngineEvent;
use sequencer_engine::models::SyncSource;

#[test]
fn commands_emit_events() {
    let eng = Engine::new();

    // Play
    eng.apply_command(Command::Play);
    let ev = eng.hot_events.dequeue().expect("Play should emit an event");
    let ev: EngineEvent = postcard::from_bytes(&ev.bytes[..ev.len as usize]).unwrap();
    assert_eq!(ev, EngineEvent::PlayStateChanged { playing: true });

    // Stop
    eng.apply_command(Command::Stop);
    let ev = eng.hot_events.dequeue().expect("Stop should emit an event");
    let ev: EngineEvent = postcard::from_bytes(&ev.bytes[..ev.len as usize]).unwrap();
    assert_eq!(ev, EngineEvent::PlayStateChanged { playing: false });

    // SetSyncSource
    eng.apply_command(Command::SetSyncSource {
        source: SyncSource::Link,
    });
    let ev = eng
        .hot_events
        .dequeue()
        .expect("SetSyncSource should emit an event");
    let ev: EngineEvent = postcard::from_bytes(&ev.bytes[..ev.len as usize]).unwrap();
    assert_eq!(
        ev,
        EngineEvent::SyncSourceChanged {
            source: SyncSource::Link
        }
    );
}

#[test]
fn pattern_clipboard_publish_gates_on_mutation() {
    // Only mutating variants publish a FullSnapshot. `CopyPattern` is
    // clipboard-only (session byte-identical) → must NOT publish (the mirror
    // learns nothing from a Copy; see `Command::CopyPattern`'s contract).
    // `PastePattern` honors its bool — a no-op (empty clipboard) publishes
    // nothing. `CutPattern`/`ClearPattern` mutate → exactly one snapshot each.
    let eng = Engine::new();
    while eng.large_events.dequeue().is_some() {}
    while eng.hot_events.dequeue().is_some() {}

    // PastePattern on an empty clipboard is a no-op → no snapshot, no hot event.
    eng.apply_command(Command::PastePattern { index: 0 });
    assert!(eng.large_events.dequeue().is_none());
    assert!(eng.hot_events.dequeue().is_none());

    // CopyPattern fills the clipboard but leaves the session unchanged → no publish.
    eng.apply_command(Command::CopyPattern { index: 0 });
    assert!(
        eng.large_events.dequeue().is_none(),
        "CopyPattern must not emit a FullSnapshot"
    );
    assert!(eng.hot_events.dequeue().is_none());

    // Clipboard now holds pattern 0 → PastePattern mutates slot 1 → one snapshot.
    eng.apply_command(Command::PastePattern { index: 1 });
    assert!(matches!(
        eng.large_events
            .dequeue()
            .expect("PastePattern should emit a FullSnapshot"),
        EngineEvent::FullSnapshot { .. }
    ));
    assert!(eng.large_events.dequeue().is_none());

    // CutPattern clears slot 0's steps → mutates → one snapshot.
    eng.apply_command(Command::CutPattern { index: 0 });
    assert!(matches!(
        eng.large_events
            .dequeue()
            .expect("CutPattern should emit a FullSnapshot"),
        EngineEvent::FullSnapshot { .. }
    ));
    assert!(eng.large_events.dequeue().is_none());

    // ClearPattern resets slot 2's steps → mutates → one snapshot.
    eng.apply_command(Command::ClearPattern { index: 2 });
    assert!(matches!(
        eng.large_events
            .dequeue()
            .expect("ClearPattern should emit a FullSnapshot"),
        EngineEvent::FullSnapshot { .. }
    ));
    assert!(eng.large_events.dequeue().is_none());
}
