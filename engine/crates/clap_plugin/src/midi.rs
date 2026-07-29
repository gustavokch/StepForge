use nih_plug::prelude::NoteEvent;
use sequencer_engine::host::MidiEvent;

/// Convert one engine `MidiEvent` (3-byte message + sample offset) into a host
/// `NoteEvent`. `NoteEvent::from_midi` normalizes velocity to [0,1] and treats
/// note-on-velocity-0 as NoteOff. Only NoteOn/NoteOff are forwarded.
pub fn midi_event_to_note(ev: &MidiEvent) -> Option<NoteEvent<()>> {
    let note = NoteEvent::from_midi(ev.sample_offset, &[ev.status, ev.data1, ev.data2]).ok()?;
    let is_note = matches!(note, NoteEvent::NoteOn { .. } | NoteEvent::NoteOff { .. });
    if is_note { Some(note) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(offset: u32, status: u8, d1: u8, d2: u8) -> MidiEvent {
        MidiEvent { sample_offset: offset, status, data1: d1, data2: d2 }
    }

    #[test]
    fn note_on_maps_with_normalized_velocity() {
        let n = midi_event_to_note(&ev(10, 0x90, 60, 127)).unwrap();
        assert!(matches!(n, NoteEvent::NoteOn { timing: 10, note: 60, .. }));
        if let NoteEvent::NoteOn { velocity, .. } = n {
            assert!((velocity - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn note_off_maps() {
        let n = midi_event_to_note(&ev(20, 0x80, 60, 0)).unwrap();
        assert!(matches!(n, NoteEvent::NoteOff { timing: 20, note: 60, .. }));
    }

    #[test]
    fn note_on_velocity_zero_becomes_note_off() {
        let n = midi_event_to_note(&ev(5, 0x90, 42, 0)).unwrap();
        assert!(matches!(n, NoteEvent::NoteOff { .. }));
    }

    #[test]
    fn non_note_status_is_dropped() {
        // CC (status 0xB0) is not a NoteOn/NoteOff → None.
        assert!(midi_event_to_note(&ev(0, 0xB0, 7, 100)).is_none());
    }
}
