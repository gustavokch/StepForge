use sequencer_engine::event::EngineEvent;
use sequencer_engine::models::Track;

#[test]
fn test_event_sizes() {
    let track = Track::default();
    let ev = EngineEvent::TrackAdded {
        track_idx: 0,
        track,
    };
    let bytes = postcard::to_allocvec(&ev).unwrap();
    println!("TrackAdded size: {}", bytes.len());
    // Since it's > 128, we now send it via large_events which has no 128 byte limit
    assert!(bytes.len() > 128);
}
