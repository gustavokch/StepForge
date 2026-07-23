use uuid::Uuid;
use sequencer_engine::models::Track;

#[test]
fn test_uuid_serialization() {
    let u = Uuid::nil();
    let bytes = postcard::to_allocvec(&u).unwrap();
    println!("Uuid nil length: {}", bytes.len());
    let t = Track::default();
    let t_bytes = postcard::to_allocvec(&t).unwrap();
    println!("Track length: {}", t_bytes.len());
}
