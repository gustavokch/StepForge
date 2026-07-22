//! CoreMIDI dispatch (unsafe). All platform MIDI I/O lives here; core's `midi.rs`
//! is pure math. The real `extern "C"` CoreMIDI bindings, `MIDISend` dispatch, and
//! all-notes-off flush are added by the engine plan.
//!
//! CoreMIDI.framework is linked for host tests via `build.rs`, and into the iOS
//! app via the target's "Link Binary With Libraries" (design decision D8).

/// Flush all notes off on a MIDI channel. No-op stub; the engine plan implements.
pub fn flush_all_notes_off(_channel: u8) {}
