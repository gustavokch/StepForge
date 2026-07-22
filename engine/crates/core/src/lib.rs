#![forbid(unsafe_code)]
//! sequencer_engine — musical-time core for StepForge.
//! Pure Rust: no FFI, no unsafe, no platform I/O. All `unsafe` and CoreMIDI
//! live in `sequencer_engine_ffi`.

pub mod algorithms;
pub mod clipboard;
pub mod clock;
pub mod command;
pub mod engine;
pub mod event;
pub mod midi;
pub mod models;
pub mod scheduler;
pub mod serde_ext;
pub mod undo;
