//! Engine: owns the RT thread, channels, and musical state. Foundation ships a
//! default-state shell; the RT loop, queues, and clock live in the engine plan.

use crate::models::Session;

pub struct Engine {
    pub session: Session,
}

impl Engine {
    /// Construct an engine with a fresh default session.
    pub fn new() -> Self {
        Self {
            session: Session::default(),
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
