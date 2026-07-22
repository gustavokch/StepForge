//! Engine handle lifecycle. `EngineHandle` is an opaque handle to a heap
//! `Engine`, exchanged across the FFI as a raw pointer. Wraps `Box::into_raw` /
//! `from_raw` (unsafe). Stub bodies — the engine plan adds the RT thread, etc.
//!
//! Lifecycle invariant (CLAUDE.md Hard Rule 5): `engine_stop` must return before
//! `engine_free`, and `engine_free` must not run concurrently with any other
//! `engine_*` call on the same handle.

use sequencer_engine::engine::Engine;

/// Opaque handle exchanged across the FFI. Never dereferenced in Swift.
#[repr(C)]
pub struct EngineHandle {
    _private: [u8; 0],
}

/// Allocate a new engine and return an opaque handle (never NULL).
pub fn new_handle() -> *mut EngineHandle {
    let engine = Box::new(Engine::new());
    Box::into_raw(engine) as *mut EngineHandle
}

/// Free a handle. NULL is a tolerated no-op (Hard Rule 5).
///
/// # Safety
/// `handle` must be NULL or a pointer previously returned by [`new_handle`],
/// and no other `engine_*` call may be in flight on it.
pub unsafe fn free_handle(handle: *mut EngineHandle) {
    if handle.is_null() {
        return;
    }
    // Reconstruct the Box<Engine> and drop it.
    let _ = Box::from_raw(handle as *mut Engine);
}
