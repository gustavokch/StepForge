//! Session persistence helpers. The on-disk format is postcard with a version
//! tag so future schema changes can migrate (amendment A15). NO #[repr(C)] —
//! the envelope crosses the FFI as bytes (amendment A4).

use crate::models::Session;

/// Wire/disk format version. Bump and add a migration when `Session` changes shape.
pub const SESSION_FORMAT_VERSION: u8 = 1;

/// A versioned, serializable session envelope.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct SessionEnvelope {
    pub version: u8,
    pub session: Session,
}

impl SessionEnvelope {
    pub fn wrap(session: Session) -> Self {
        Self { version: SESSION_FORMAT_VERSION, session }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_envelope_roundtrips() {
        let session = Session::default();
        let envelope = SessionEnvelope::wrap(session.clone());
        let bytes = postcard::to_allocvec(&envelope).expect("serialize");
        let back: SessionEnvelope = postcard::from_bytes(&bytes).expect("deserialize");
        assert_eq!(back.version, SESSION_FORMAT_VERSION);
        assert_eq!(back.session, session);
    }
}
