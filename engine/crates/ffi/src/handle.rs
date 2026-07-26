//! Engine handle lifecycle. `EngineHandle` is an opaque handle to a heap
//! `Arc<Engine>`, exchanged across the FFI as a raw pointer. Wraps
//! `Arc::into_raw` / `from_raw` (unsafe). Task 20a changes Box→Arc for
//! shared ownership across the RT/worker/CoreMIDI threads.
//!
//! Lifecycle invariant (CLAUDE.md Hard Rule 5): `engine_stop` must return before
//! `engine_free`, and `engine_free` must not run concurrently with any other
//! `engine_*` call on the same handle.

use sequencer_engine::engine::Engine;
use std::sync::Arc;

/// Opaque handle exchanged across the FFI. Never dereferenced in Swift.
#[repr(C)]
pub struct EngineHandle {
    _private: [u8; 0],
}

/// Allocate a new engine and return an opaque handle (never NULL).
pub fn new_handle() -> *mut EngineHandle {
    let engine = Arc::new(Engine::new());
    Arc::into_raw(engine) as *mut EngineHandle
}

/// Free a handle. NULL is a tolerated no-op (Hard Rule 5).
///
/// Defensively joins any remaining thread handles BEFORE dropping the `Arc<Engine>`
/// (a dropped `JoinHandle` only detaches; we must join to prevent use-after-free
/// if the caller free'd without calling `engine_stop` first).
///
/// # Safety
/// `handle` must be NULL or a pointer previously returned by [`new_handle`],
/// and no other `engine_*` call may be in flight on it.
pub unsafe fn free_handle(handle: *mut EngineHandle) {
    if handle.is_null() {
        return;
    }
    // Reconstruct the Arc<Engine> from the raw pointer.
    let engine = Arc::from_raw(handle as *const Engine);

    // Defensive: if shutdown wasn't signaled (e.g., free without stop),
    // signal it now and join any still-running threads.
    // This prevents a runaway RT thread from reading freed memory.
    if !engine.shutdown.load(std::sync::atomic::Ordering::Acquire) {
        engine
            .shutdown
            .store(true, std::sync::atomic::Ordering::Release);
    }

    // Join RT handle if still Some
    {
        let maybe_handle = engine.rt_handle.lock().unwrap().take();
        if let Some(handle) = maybe_handle {
            let _ = handle.join();
        }
    }
    // Join worker handle if still Some
    {
        let maybe_handle = engine.worker_handle.lock().unwrap().take();
        if let Some(handle) = maybe_handle {
            let _ = handle.join();
        }
    }

    // The Arc<Engine> is dropped here, cleaning up the engine state.
}

use sequencer_engine::host::HostRenderState;

/// Opaque handle to a `HostRenderState` (one per host-driven engine instance).
/// Owned by the plugin wrapper; never dereferenced in C.
#[repr(C)]
pub struct RenderStateHandle {
    _private: [u8; 0],
}

/// Allocate a host-driven engine handle (never NULL).
pub fn new_host_handle() -> *mut EngineHandle {
    let engine = Arc::new(Engine::new_host_driven());
    Arc::into_raw(engine) as *mut EngineHandle
}

/// Allocate a fresh render-state handle (never NULL).
pub fn new_render_state() -> *mut RenderStateHandle {
    Box::into_raw(Box::new(HostRenderState::new())) as *mut RenderStateHandle
}

/// Free a render-state handle. NULL is a tolerated no-op.
///
/// # Safety
/// `handle` is NULL or a pointer from [`new_render_state`]; no concurrent use.
pub unsafe fn free_render_state(handle: *mut RenderStateHandle) {
    if handle.is_null() {
        return;
    }
    unsafe { drop(Box::from_raw(handle as *mut HostRenderState)) };
}
