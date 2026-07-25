//! Pure MIDI dispatch math. No CoreMIDI, no unsafe. Produces fixed MidiMsg slots.

use crate::clock::Rng;
use crate::midi_out::MidiMsg;
use crate::models::{Ratchet, VelocityZone};

pub const VEL_LOW: u8 = 42;
pub const VEL_MID: u8 = 85;
pub const VEL_ACCENT: u8 = 127;
pub const DEFAULT_GATE_MICROS: u32 = 50_000;
const NOTE_ON: u8 = 0x90;
/// Reserved for `process()` (Task 11), which emits note-offs from `gate_micros`.
#[allow(dead_code)]
const NOTE_OFF: u8 = 0x80;
const CC: u8 = 0xB0;
const ALL_NOTES_OFF_CC: u8 = 123;

pub fn velocity_for_zone(zone: VelocityZone) -> u8 {
    match zone {
        VelocityZone::Low => VEL_LOW,
        VelocityZone::Mid => VEL_MID,
        VelocityZone::Accent => VEL_ACCENT,
    }
}

/// Humanize base velocity by ±(humanize_velocity * zone_weight * 5) MIDI units (E4).
pub fn humanize_velocity(base: u8, humanize_velocity: f32, zone_weight: f32, rng: &mut Rng) -> u8 {
    let mag = (humanize_velocity * zone_weight * 5.0).round() as i32;
    if mag == 0 {
        return base;
    }
    let jitter = rng.range(-mag, mag);
    (base as i32 + jitter).clamp(1, 127) as u8
}

pub fn ratchet_count(r: Ratchet) -> u32 {
    match r {
        Ratchet::Off => 1,
        Ratchet::X2 => 2,
        Ratchet::X3 => 3,
        Ratchet::X4 => 4,
    }
}

pub fn build_note_on(
    endpoint: u32,
    channel: u8,
    note: u8,
    velocity: u8,
    send_at_offset_micros: u32,
) -> MidiMsg {
    MidiMsg {
        endpoint,
        channel,
        status: NOTE_ON | (channel & 0x0F),
        note,
        velocity,
        send_at_offset_micros,
        gate_micros: DEFAULT_GATE_MICROS,
    }
}
pub fn build_all_notes_off(endpoint: u32, channel: u8) -> MidiMsg {
    MidiMsg {
        endpoint,
        channel,
        status: CC | (channel & 0x0F),
        note: ALL_NOTES_OFF_CC,
        velocity: 0,
        send_at_offset_micros: 0,
        gate_micros: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn velocity_table_and_humanize_bounds() {
        assert_eq!(velocity_for_zone(VelocityZone::Low), VEL_LOW);
        let mut rng = Rng::new(7);
        let h = humanize_velocity(VEL_MID, 1.0, 1.0, &mut rng);
        // ±5 around VEL_MID — relative so velocity retuning doesn't re-break this.
        let lo = VEL_MID.saturating_sub(5);
        let hi = VEL_MID.saturating_add(5);
        assert!((lo..=hi).contains(&h), "±5 around mid (VEL_MID={VEL_MID})");
        let zero = humanize_velocity(VEL_MID, 0.0, 1.0, &mut rng);
        assert_eq!(zero, VEL_MID);
    }
    #[test]
    fn ratchet_counts() {
        assert_eq!(ratchet_count(Ratchet::Off), 1);
        assert_eq!(ratchet_count(Ratchet::X4), 4);
    }
    #[test]
    fn note_on_carries_channel_and_gate() {
        let m = build_note_on(3, 10, 36, 100, 2_000);
        assert_eq!(m.status, 0x9A);
        assert_eq!(m.gate_micros, DEFAULT_GATE_MICROS);
        assert_eq!(m.send_at_offset_micros, 2_000);
    }
}
