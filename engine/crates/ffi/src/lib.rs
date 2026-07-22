//! sequencer_engine_ffi — the C ABI shim + CoreMIDI bridge for StepForge.
//! `#![allow(unsafe_code)]`: the ONLY crate in the workspace that may use unsafe.
//! Every `extern "C"` entry is panic-safe (`catch_unwind`) and returns an
//! `EngineResult` status (CLAUDE.md Hard Rules 3, 6). Foundation stub bodies.

#![allow(unsafe_code)]

mod coremidi;
mod handle;

pub use handle::EngineHandle;
