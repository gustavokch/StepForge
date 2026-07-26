//! sequencer_engine_ffi — the C ABI shim + CoreMIDI bridge for StepForge.
//! `#![allow(unsafe_code)]`: the ONLY crate in the workspace that may use unsafe.
//! Every `extern "C"` entry is panic-safe (`catch_unwind`) and returns an
//! `EngineResult` status (CLAUDE.md Hard Rules 3, 6). Foundation stub bodies.

#![allow(unsafe_code)]

use sequencer_engine::engine::Engine;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

pub mod command_codec;
pub mod coremidi;
pub mod event_codec;
mod handle;

pub use handle::EngineHandle;
pub use handle::RenderStateHandle;
pub use sequencer_engine::host::{HostRenderState, HostTransport, MidiEvent};

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
///
/// The handle stores an `Arc<Engine>` as a raw pointer from `Arc::into_raw`.
/// We borrow it as `&Engine` for the call duration (the Arc's memory is stable
/// as long as the handle is valid per Hard Rule 5).
fn run_void(
    engine: *mut EngineHandle,
    body: impl FnOnce(&Engine) -> Result<(), EngineResult>,
) -> EngineResult {
    match catch_unwind(AssertUnwindSafe(|| {
        if engine.is_null() {
            return Err(EngineResult::ErrInvalidHandle);
        }
        // SAFETY: caller upholds Hard Rule 5 (no concurrent free; stop before free).
        // The handle is `*const Engine` from `Arc::into_raw`, cast to `*mut EngineHandle`.
        // We reinterpret it as `&Engine` for borrowing (the Arc allocation is stable).
        let eng = unsafe { &*(engine as *const Engine) };
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

/// Start playback. Spawns the RT, worker, and CoreMIDI worker threads.
///
/// Returns `ErrOther` if CoreMIDI client/port creation fails (non-fatal —
/// playback runs but MIDI won't send). The handle becomes shared across
/// the three threads via `Arc<Engine>` (Task 20a).
///
/// # Safety
/// `engine` is NULL or a handle from [`engine_new`]; caller upholds Hard Rule 5
/// (no concurrent `engine_*` calls on the same handle). Must not be called on a
/// handle that is already running or that has been freed.
#[no_mangle]
pub unsafe extern "C" fn engine_start(engine: *mut EngineHandle) -> EngineResult {
    match catch_unwind(AssertUnwindSafe(|| {
        if engine.is_null() {
            return Err(EngineResult::ErrInvalidHandle);
        }
        // Reconstruct the Arc<Engine> from the raw pointer.
        // This is safe because the handle was created via Arc::into_raw
        // and the caller upholds Hard Rule 5 (no concurrent free).
        let eng = unsafe { Arc::from_raw(engine as *const Engine) };

        // Reset shutdown flag to false so worker and RT loops run (crucial if start is called again after stop)
        eng.shutdown
            .store(false, std::sync::atomic::Ordering::Release);

        // Host-driven mode (plugin port, Phase 0): spawn ONLY the state worker.
        // The host drives dispatch via `engine_render`; no self-scheduled RT
        // thread, no CoreMIDI worker, no Link poller.
        if eng.host_driven {
            let eng_worker = Arc::clone(&eng);
            let worker_handle = std::thread::spawn(move || {
                eng_worker.run_worker_loop();
            });
            *eng.worker_handle.lock().unwrap() = Some(worker_handle);
            std::mem::forget(eng);
            return Ok(());
        }

        // Create CoreMIDI client, output port, and virtual source (engine owns these per Rule 7).
        // Non-fatal if CoreMIDI client creation fails (e.g. sandboxed / un-entitled);
        // worker and RT threads must still spawn so playback and UI commands work.
        let (client, port, source) =
            unsafe { coremidi::create_client_and_port() }.unwrap_or((0, 0, 0));

        // Store client/port/source in the engine for disposal in engine_stop
        *eng.coremidi_client.lock().unwrap() = client;
        *eng.coremidi_port.lock().unwrap() = port;
        *eng.coremidi_source.lock().unwrap() = source;

        // Clone Arc for each thread
        let eng_rt = Arc::clone(&eng);
        let eng_worker = Arc::clone(&eng);
        let eng_coremidi = Arc::clone(&eng);

        // Spawn RT thread with InstantClock
        let clock = coremidi::InstantClock::new();
        let rt_handle = std::thread::spawn(move || {
            eng_rt.run_rt_loop(&clock);
        });

        // Spawn state worker thread
        let worker_handle = std::thread::spawn(move || {
            eng_worker.run_worker_loop();
        });

        // Spawn CoreMIDI worker thread only if port != 0 or source != 0
        let coremidi_handle = if port != 0 || source != 0 {
            Some(std::thread::spawn(move || {
                coremidi::run_coremidi_worker(&eng_coremidi, port, source);
            }))
        } else {
            None
        };

        // Spawn the off-RT Ableton Link poller (B2). Always spawned; it idles
        // (no Link calls) while Link is disabled. This is the only thread that
        // touches ableton-link-rs — never the RT thread (Hard Rule 1).
        let eng_link = Arc::clone(&eng);
        let link_poller_handle = std::thread::spawn(move || {
            eng_link.run_link_poller();
        });

        // Store handles in the engine
        *eng.rt_handle.lock().unwrap() = Some(rt_handle);
        *eng.worker_handle.lock().unwrap() = Some(worker_handle);
        *eng.coremidi_handle.lock().unwrap() = coremidi_handle;
        *eng.link_poller_handle.lock().unwrap() = Some(link_poller_handle);

        // Forget the Arc so it doesn't drop - the handle still owns it
        std::mem::forget(eng);

        Ok(())
    })) {
        Ok(Ok(())) => EngineResult::Ok,
        Ok(Err(e)) => e,
        Err(_) => EngineResult::ErrOther,
    }
}

/// Stop playback. Signals shutdown, joins all three threads, and disposes
/// the CoreMIDI client/port. Must return before `engine_free` (Rule 5).
///
/// # Safety
/// `engine` is NULL or a handle from [`engine_new`] that was previously started
/// with [`engine_start`]; caller upholds Hard Rule 5 (no concurrent `engine_*`
/// calls on the same handle). NULL returns `ErrInvalidHandle`.
#[no_mangle]
pub unsafe extern "C" fn engine_stop(engine: *mut EngineHandle) -> EngineResult {
    run_void(engine, |eng| {
        // Signal shutdown
        eng.shutdown
            .store(true, std::sync::atomic::Ordering::Release);

        // Join RT thread
        if let Some(handle) = eng.rt_handle.lock().unwrap().take() {
            let _ = handle.join();
        }

        // Join worker thread
        if let Some(handle) = eng.worker_handle.lock().unwrap().take() {
            let _ = handle.join();
        }

        // Join CoreMIDI worker thread
        if let Some(handle) = eng.coremidi_handle.lock().unwrap().take() {
            let _ = handle.join();
        }

        // Join Link poller thread (B2)
        if let Some(handle) = eng.link_poller_handle.lock().unwrap().take() {
            let _ = handle.join();
        }

        // Dispose CoreMIDI client and port
        let client = *eng.coremidi_client.lock().unwrap();
        let port = *eng.coremidi_port.lock().unwrap();
        let source = *eng.coremidi_source.lock().unwrap();
        if client != 0 {
            let _ = unsafe { coremidi::dispose_client_and_port(client, port, source) };
            *eng.coremidi_client.lock().unwrap() = 0;
            *eng.coremidi_port.lock().unwrap() = 0;
            *eng.coremidi_source.lock().unwrap() = 0;
        }

        Ok(())
    })
}

/// Submit a command as postcard bytes. Returns `ErrDecode` on malformed bytes
/// (non-fatal).
///
/// The command is enqueued into the lock-free MPSC queue for the state worker
/// to apply (Task 20a). If the queue is full, the oldest command is dropped
/// and an `Overflow` event is emitted (E8). Returns `Ok` even on overflow
/// (the event makes the drop observable).
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
    run_void(engine, |eng| {
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
            Ok(command) => {
                // Task 20a: enqueue into the command queue (drop-oldest on full).
                // The state worker will apply this command.
                use sequencer_engine::midi_out::push_drop_oldest;
                let dropped = push_drop_oldest(&eng.commands, command);
                if dropped > 0 {
                    // Emit Overflow event on the hot channel
                    use sequencer_engine::event::EngineEvent;
                    use sequencer_engine::midi_out::push_event;
                    let dropped_u32 = dropped as u32;
                    let _ = push_event(
                        &eng.hot_events,
                        &EngineEvent::Overflow {
                            dropped: dropped_u32,
                        },
                    );
                }
                Ok(())
            }
            Err(e) => {
                println!("[Rust FFI] ERROR: decode_command failed: {:?}", e);
                Err(EngineResult::ErrDecode)
            }
        }
    })
}

/// Drain at most one event into `*out_ptr`/`*out_len`. An empty/zero-length
/// result means both channels are drained. Tries the hot channel first, then
/// the large channel. Buffers must be freed via [`engine_free_bytes`].
///
/// NULL-tolerant: if `out_ptr`/`out_len` are NULL, the event is dequeued and
/// discarded (useful for just draining without reading).
///
/// # Safety
/// `out_ptr`/`out_len` are valid writable pointers (or NULL); `engine` is valid.
#[no_mangle]
pub unsafe extern "C" fn engine_drain_events(
    engine: *mut EngineHandle,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> EngineResult {
    run_void(engine, |eng| {
        // Try hot channel first (for playhead coalescing, E7)
        if let Some(slot) = eng.hot_events.dequeue() {
            // Box the event bytes for Swift to own
            let len = slot.len as usize;
            let mut bytes: Box<[u8]> = slot.bytes[..len].into();
            let ptr = bytes.as_mut_ptr();
            std::mem::forget(bytes); // Transfer ownership to caller

            // NULL-tolerant: if out-params are NULL, just discard
            if !out_ptr.is_null() {
                unsafe { *out_ptr = ptr };
            }
            if !out_len.is_null() {
                unsafe { *out_len = len };
            }
            return Ok(());
        }

        // Hot channel empty, try large channel
        if let Some(event) = eng.large_events.dequeue() {
            // Encode the large event into bytes
            if let Ok(vec) = postcard::to_allocvec(&event) {
                let len = vec.len();
                let mut boxed = vec.into_boxed_slice();
                let ptr = boxed.as_mut_ptr();
                std::mem::forget(boxed); // Transfer ownership

                if !out_ptr.is_null() {
                    unsafe { *out_ptr = ptr };
                }
                if !out_len.is_null() {
                    unsafe { *out_len = len };
                }
                return Ok(());
            }
            // Serialization failed - shouldn't happen for EngineEvent,
            // but treat as no-event if it does
        }

        // Both channels drained
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

/// Create a host-driven engine. Returns an opaque handle (never NULL). Pair with
/// [`engine_render_state_new`] and drive via [`engine_render`].
#[no_mangle]
pub extern "C" fn engine_new_host_driven() -> *mut EngineHandle {
    match catch_unwind(handle::new_host_handle) {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

/// Allocate a per-instance render-state handle for [`engine_render`].
#[no_mangle]
pub extern "C" fn engine_render_state_new() -> *mut RenderStateHandle {
    match catch_unwind(AssertUnwindSafe(handle::new_render_state)) {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a render-state handle. NULL is a tolerated no-op.
///
/// # Safety
/// `handle` is NULL or from [`engine_render_state_new`]; no concurrent use.
#[no_mangle]
pub unsafe extern "C" fn engine_render_state_free(handle: *mut RenderStateHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        handle::free_render_state(handle)
    }));
}

/// Advance the engine by one host audio block on the host's RT thread. Writes
/// outgoing MIDI `MidiEvent`s into `midi_out` (sample offsets within the block)
/// and returns the count in `*midi_out_count`.
///
/// # Safety
/// `engine`/`rs`/`transport` valid (or NULL for `engine`/`rs` → error); `midi_in`
/// valid for `midi_in_count` entries; `midi_out` valid for `midi_out_cap` entries;
/// `midi_out_count` writable (or NULL).
#[no_mangle]
pub unsafe extern "C" fn engine_render(
    engine: *mut EngineHandle,
    rs: *mut RenderStateHandle,
    transport: *const HostTransport,
    midi_in: *const MidiEvent,
    midi_in_count: usize,
    midi_out: *mut MidiEvent,
    midi_out_cap: usize,
    midi_out_count: *mut usize,
) -> EngineResult {
    match catch_unwind(AssertUnwindSafe(|| {
        if engine.is_null() || rs.is_null() || transport.is_null() {
            return Err(EngineResult::ErrInvalidHandle);
        }
        if midi_in.is_null() && midi_in_count != 0 {
            return Err(EngineResult::ErrInvalidBuffer);
        }
        // SAFETY: caller upholds Hard Rule 5 (no concurrent free); handles come
        // from engine_new_host_driven / engine_render_state_new.
        let eng = unsafe { &*(engine as *const Engine) };
        // M2: enforce the host-pairing contract — `engine_render` only drives
        // host-mode engines. A host that mis-pairs `engine_new` (standalone)
        // with `engine_render` would otherwise double-dispatch (self-scheduled
        // RT thread + host RT thread both driving `process_one`).
        if !eng.host_driven {
            return Err(EngineResult::ErrInvalidHandle);
        }
        let state = unsafe { &mut *(rs as *mut HostRenderState) };
        let transport = unsafe { &*transport };
        let midi_in: &[MidiEvent] = if midi_in_count == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(midi_in, midi_in_count) }
        };
        let midi_out: &mut [MidiEvent] = if midi_out_cap == 0 || midi_out.is_null() {
            &mut []
        } else {
            unsafe { core::slice::from_raw_parts_mut(midi_out, midi_out_cap) }
        };
        let written = eng.render_host(state, transport, midi_in, midi_out);
        if !midi_out_count.is_null() {
            unsafe { *midi_out_count = written };
        }
        Ok(())
    })) {
        Ok(Ok(())) => EngineResult::Ok,
        Ok(Err(e)) => e,
        Err(_) => EngineResult::ErrOther,
    }
}
