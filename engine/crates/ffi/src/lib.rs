//! sequencer_engine_ffi — the C ABI shim + CoreMIDI bridge for StepForge.
//! `#![allow(unsafe_code)]`: the ONLY crate in the workspace that may use unsafe.
//! Every `extern "C"` entry is panic-safe (`catch_unwind`) and returns an
//! `EngineResult` status (CLAUDE.md Hard Rules 3, 6). Foundation stub bodies.

#![allow(unsafe_code)]

use sequencer_engine::engine::Engine;
use std::panic::{catch_unwind, AssertUnwindSafe};

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
pub enum CodecError {
    Postcard(postcard::Error),
}

impl From<postcard::Error> for CodecError {
    fn from(e: postcard::Error) -> Self {
        Self::Postcard(e)
    }
}

/// Run a void FFI body with a validated, borrowed `Engine`, catching any panic.
/// Null handle → `ErrInvalidHandle`; panic → `ErrOther`.
fn run_void(
    engine: *mut EngineHandle,
    body: impl FnOnce(&mut Engine) -> Result<(), EngineResult>,
) -> EngineResult {
    match catch_unwind(AssertUnwindSafe(|| {
        if engine.is_null() {
            return Err(EngineResult::ErrInvalidHandle);
        }
        // SAFETY: caller upholds Hard Rule 5 (no concurrent free; stop before free).
        let eng = unsafe { &mut *(engine as *mut Engine) };
        body(eng)
    })) {
        Ok(Ok(())) => EngineResult::Ok,
        Ok(Err(e)) => e,
        Err(_) => EngineResult::ErrOther,
    }
}

/// Create a new engine. Returns an opaque handle (never NULL).
#[no_mangle]
pub extern "C" fn engine_new() -> *mut EngineHandle {
    match catch_unwind(handle::new_handle) {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free an engine. NULL is a tolerated no-op. `engine_stop` must have returned first.
///
/// # Safety
/// `engine` is NULL or a handle from [`engine_new`]; no concurrent `engine_*` call.
#[no_mangle]
pub unsafe extern "C" fn engine_free(engine: *mut EngineHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: caller upholds Hard Rule 5; free_handle tolerates NULL.
        unsafe { handle::free_handle(engine) };
    }));
}

/// Start playback. Stub: no-op, returns Ok. The engine plan implements.
#[no_mangle]
pub unsafe extern "C" fn engine_start(engine: *mut EngineHandle) -> EngineResult {
    run_void(engine, |_eng| Ok(()))
}

/// Stop playback. Stub: no-op. Must be called and return before `engine_free`.
#[no_mangle]
pub unsafe extern "C" fn engine_stop(engine: *mut EngineHandle) -> EngineResult {
    run_void(engine, |_eng| Ok(()))
}

/// Submit a command as postcard bytes. Returns `ErrDecode` on malformed bytes
/// (non-fatal). Stub: decodes only; the engine plan applies the command.
///
/// # Safety
/// `cmd_ptr` is valid for `cmd_len` bytes for the duration of the call (or NULL
/// when `cmd_len == 0`); `engine` is a valid handle.
#[no_mangle]
pub unsafe extern "C" fn engine_submit_command(
    engine: *mut EngineHandle,
    cmd_ptr: *const u8,
    cmd_len: usize,
) -> EngineResult {
    run_void(engine, |_eng| {
        if cmd_ptr.is_null() && cmd_len != 0 {
            return Err(EngineResult::ErrInvalidBuffer);
        }
        let bytes: &[u8] = if cmd_len == 0 {
            &[]
        } else {
            // SAFETY: caller guarantees cmd_ptr is valid for cmd_len bytes.
            unsafe { core::slice::from_raw_parts(cmd_ptr, cmd_len) }
        };
        match command_codec::decode_command(bytes) {
            Ok(_command) => Ok(()), // engine plan applies it.
            Err(_) => Err(EngineResult::ErrDecode),
        }
    })
}

/// Drain at most one event into `*out_ptr`/`*out_len`. An empty/zero-length
/// result means the queue is drained (design decision D5 / amendment A13).
/// Stub: always drained (no events produced yet). Buffers must be freed via
/// [`engine_free_bytes`].
///
/// # Safety
/// `out_ptr`/`out_len` are valid writable pointers (or NULL); `engine` is valid.
#[no_mangle]
pub unsafe extern "C" fn engine_drain_events(
    engine: *mut EngineHandle,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> EngineResult {
    run_void(engine, |_eng| {
        if !out_ptr.is_null() {
            unsafe { *out_ptr = std::ptr::null_mut() };
        }
        if !out_len.is_null() {
            unsafe { *out_len = 0 };
        }
        Ok(())
    })
}

/// Serialize the current session into `*out_ptr`/`*out_len` as a versioned
/// postcard `SessionEnvelope`. The buffer is Rust-allocated and must be freed
/// with [`engine_free_bytes`] (Hard Rule 4).
///
/// NULL contract asymmetry: unlike [`engine_drain_events`] — a pull that
/// tolerates NULL out-params and no-ops when empty — this MUST receive non-NULL
/// `out_ptr`/`out_len`, since it has to hand back both the buffer pointer and
/// its length for the caller to read and later free (NULL → `ErrInvalidBuffer`).
///
/// # Safety
/// `out_ptr`/`out_len` are valid writable, non-NULL pointers; `engine` is valid.
#[no_mangle]
pub unsafe extern "C" fn engine_serialize(
    engine: *mut EngineHandle,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> EngineResult {
    run_void(engine, |eng| {
        if out_ptr.is_null() || out_len.is_null() {
            return Err(EngineResult::ErrInvalidBuffer);
        }
        // COW read: snapshot_arc() is a lock-free load_full() of the
        // authoritative Session — synchronous on the caller thread, no worker
        // round-trip, no allocation beyond the serialized Vec below (Task 8).
        let envelope =
            sequencer_engine::serde_ext::SessionEnvelope::wrap((*eng.snapshot_arc()).clone());
        let bytes = match postcard::to_allocvec(&envelope) {
            Ok(b) => b,
            Err(_) => return Err(EngineResult::ErrOther),
        };
        let len = bytes.len();
        let mut boxed = bytes.into_boxed_slice();
        let ptr = boxed.as_mut_ptr();
        // Transfer ownership to the caller; freed via engine_free_bytes.
        std::mem::forget(boxed);
        unsafe {
            *out_ptr = ptr;
            *out_len = len;
        }
        Ok(())
    })
}

/// Free a buffer returned by `engine_drain_events` / `engine_serialize`.
/// `engine_free_bytes(NULL, 0)` is a no-op (Hard Rule 4).
///
/// # Safety
/// `ptr`/`len` are NULL/0 or exactly a pair returned by this crate.
#[no_mangle]
pub unsafe extern "C" fn engine_free_bytes(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: ptr/len came from engine_serialize/drain; capacity == len
        // (into_boxed_slice shrinks capacity to len).
        let _vec = unsafe { Vec::from_raw_parts(ptr, len, len) };
    }));
}
