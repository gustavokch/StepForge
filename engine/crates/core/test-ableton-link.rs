use ableton_link_rs::Link;
fn main() {
    let mut link = Link::new(120.0);
    link.enable(true);
    let session_state = link.capture_app_session_state();
    println!("Tempo: {}", session_state.tempo());
}
