//! Event byte codec (Rust → Swift). The RT-side encoder writes into a caller-
//! provided fixed buffer (allocation-free — CLAUDE.md Hard Rule 1). Decode is total.

use crate::CodecError;
use sequencer_engine::event::EngineEvent;

/// Encode an event into a fixed buffer; returns the number of bytes written.
/// Allocation-free — safe to call from the RT thread (Hard Rule 1).
pub fn encode_event_into(event: &EngineEvent, buf: &mut [u8]) -> Result<usize, CodecError> {
    let used = postcard::to_slice(event, buf)?;
    Ok(used.len())
}

/// Decode an event from bytes. Total (never panics).
pub fn decode_event(bytes: &[u8]) -> Result<EngineEvent, CodecError> {
    Ok(postcard::from_bytes(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_events_roundtrip_into_fixed_buffer() {
        let events = [
            EngineEvent::PlayStateChanged { playing: true },
            EngineEvent::BpmChanged { bpm: 123.0 },
            EngineEvent::Playhead {
                track_idx: 2,
                step_idx: 7,
            },
        ];
        let mut buf = [0u8; crate::MAX_EVENT_BYTES];
        for e in events {
            let n = encode_event_into(&e, &mut buf).unwrap();
            let back = decode_event(&buf[..n]).unwrap();
            assert_eq!(back, e);
        }
    }

    #[test]
    fn garbage_event_bytes_do_not_panic() {
        let res = decode_event(&[0xfe, 0xfe, 0xfe]);
        assert!(res.is_err());
    }
}
