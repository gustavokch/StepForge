use nih_plug::params::Params;
use nih_plug_egui::EguiState;
use parking_lot::RwLock;
use std::sync::Arc;

/// All non-parameter plugin state. Zero automation params — StepForge is driven
/// via `Command`s; `Params` carries only persisted fields.
#[derive(Params)]
pub struct StepForgeParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    /// Postcard `SessionEnvelope` bytes, serialized fresh on the GUI thread.
    #[persist = "session"]
    pub session: Arc<RwLock<Vec<u8>>>,
}

impl Default for StepForgeParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(900, 600),
            session: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_constructs() {
        let p = StepForgeParams::default();
        assert!(p.session.read().is_empty());
    }

    #[test]
    fn session_bytes_round_trip_as_json() {
        // #[persist] serializes via serde_json; a bare Vec<u8> becomes a JSON
        // number array and must round-trip byte-for-byte.
        let p = StepForgeParams::default();
        let payload = vec![0xDEu8, 0xAD, 0xBE, 0xEF];
        *p.session.write() = payload.clone();
        let s = serde_json::to_string(&*p.session.read()).unwrap();
        let back: Vec<u8> = serde_json::from_str(&s).unwrap();
        assert_eq!(back, payload);
    }
}
