//! GUI mirror of engine state (Phase 0 subset; Phase 1 ports the full
//! SessionMirror apply logic).
use sequencer_engine::models::Session;
use std::sync::Arc;

#[derive(Default)]
pub struct UiState {
    pub playing: bool,
    pub session: Option<Arc<Session>>,
    /// Rolling last-N engine error messages (Phase 0 surfacing).
    pub errors: Vec<String>,
}
