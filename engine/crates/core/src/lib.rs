#![forbid(unsafe_code)]
//! sequencer_engine — musical-time core for StepForge.
//! Pure Rust: no FFI, no unsafe, no platform I/O. All `unsafe` and CoreMIDI
//! live in `sequencer_engine_ffi`.

pub mod models;
pub mod serde_ext;
