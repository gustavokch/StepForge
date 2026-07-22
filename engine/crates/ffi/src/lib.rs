//! sequencer_engine_ffi — the C ABI shim + CoreMIDI bridge for StepForge.
//! `#![allow(unsafe_code)]`: the ONLY crate in the workspace that may use unsafe.
//! Every `extern "C"` entry is panic-safe (`catch_unwind`) and returns an
//! `EngineResult` status (CLAUDE.md Hard Rules 3, 6). Foundation stub bodies.

#![allow(unsafe_code)]

pub mod command_codec;
pub mod coremidi;
pub mod event_codec;
mod handle;

pub use handle::EngineHandle;

/// Maximum bytes an event may occupy on the hot (fixed-slot) RT channel. Events
/// that would exceed this (e.g. `Serialized`) are routed to the separate off-RT
/// large-event channel (design decision D5; amendment A2).
pub const MAX_EVENT_BYTES: usize = 128;

/// Status returned by every `extern "C"` entry point (CLAUDE.md Hard Rule 3).
/// Discriminants are stable across releases — do not renumber existing variants.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineResult {
    /// Success.
    Ok = 0,
    /// A command/event byte buffer could not be decoded (non-fatal).
    ErrDecode = 1,
    /// A NULL or invalid handle was supplied.
    ErrInvalidHandle = 2,
    /// A buffer pointer/length was invalid.
    ErrInvalidBuffer = 3,
    /// Any other failure; details via an `EngineEvent::Error`.
    ErrOther = 4,
}

#[derive(Debug)]
pub(crate) enum CodecError {
    Postcard(postcard::Error),
}

impl From<postcard::Error> for CodecError {
    fn from(e: postcard::Error) -> Self {
        Self::Postcard(e)
    }
}
