//! Command byte codec (Swift → Rust). Decode is total: malformed input returns
//! `Err`, never panics (CLAUDE.md Hard Rule 3). `encode_command` is for tests /
//! off-RT use; Swift owns the production encoder.

use crate::CodecError;
use sequencer_engine::command::Command;

/// Decode a Command from postcard bytes. Total (never panics).
pub fn decode_command(bytes: &[u8]) -> Result<Command, CodecError> {
    Ok(postcard::from_bytes(bytes)?)
}

/// Encode a Command to a freshly-allocated Vec (off-RT / test use only).
pub fn encode_command(command: &Command) -> Result<Vec<u8>, CodecError> {
    Ok(postcard::to_allocvec(command)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sequencer_engine::models::VelocityZone;

    #[test]
    fn commands_roundtrip() {
        let cmds = [
            Command::SetBpm { bpm: 140.0 },
            Command::AddTrack,
            Command::SetStep {
                track_idx: 0,
                step_idx: 3,
                zone: VelocityZone::Accent,
            },
            Command::LoadSession { bytes: vec![9, 9] },
        ];
        for c in cmds {
            let bytes = encode_command(&c).unwrap();
            let back = decode_command(&bytes).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn garbage_command_bytes_do_not_panic() {
        // Truncated / invalid bytes must yield Err, not a panic (Hard Rule 3).
        let res = decode_command(&[0xff, 0xff, 0xff, 0xff]);
        assert!(res.is_err());
    }
}
