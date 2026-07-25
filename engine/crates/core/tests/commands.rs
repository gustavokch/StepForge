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
