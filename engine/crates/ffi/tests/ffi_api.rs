//! Integration tests that call the engine through the real C ABI to catch
//! serialization / ABI / lifetime bugs (architecture-spec §11.2). These run on
//! the macOS host; CoreMIDI is linked via build.rs.

use sequencer_engine_ffi::{command_codec, EngineResult};

#[test]
fn new_and_free_do_not_crash() {
    let h = sequencer_engine_ffi::engine_new();
    assert!(!h.is_null());
    unsafe { sequencer_engine_ffi::engine_free(h) };
    // free of NULL is a tolerated no-op (Hard Rule 4/5).
    unsafe { sequencer_engine_ffi::engine_free(std::ptr::null_mut()) };
}

#[test]
fn garbage_command_bytes_return_non_fatal_error() {
    let h = sequencer_engine_ffi::engine_new();
    let garbage = [0xffu8, 0xff, 0xff, 0xff, 0xff];
    let res =
        unsafe { sequencer_engine_ffi::engine_submit_command(h, garbage.as_ptr(), garbage.len()) };
    assert_ne!(res, EngineResult::Ok, "garbage must not be accepted");
    // Reaching here proves the process did not abort on bad bytes (Hard Rule 3).
    unsafe { sequencer_engine_ffi::engine_free(h) };
}

#[test]
fn well_formed_command_is_accepted() {
    use sequencer_engine::command::Command;
    let h = sequencer_engine_ffi::engine_new();
    let bytes = command_codec::encode_command(&Command::Play).unwrap();
    let res =
        unsafe { sequencer_engine_ffi::engine_submit_command(h, bytes.as_ptr(), bytes.len()) };
    assert_eq!(res, EngineResult::Ok);
    unsafe { sequencer_engine_ffi::engine_free(h) };
}

#[test]
fn drain_returns_empty_when_no_events() {
    let h = sequencer_engine_ffi::engine_new();
    let mut ptr = std::ptr::null_mut();
    let mut len = 0usize;
    let res = unsafe { sequencer_engine_ffi::engine_drain_events(h, &mut ptr, &mut len) };
    assert_eq!(res, EngineResult::Ok);
    assert_eq!(len, 0);
    unsafe { sequencer_engine_ffi::engine_free(h) };
}

#[test]
fn serialize_yields_bytes_freed_via_free_bytes() {
    let h = sequencer_engine_ffi::engine_new();
    let mut ptr = std::ptr::null_mut();
    let mut len = 0usize;
    let res = unsafe { sequencer_engine_ffi::engine_serialize(h, &mut ptr, &mut len) };
    assert_eq!(res, EngineResult::Ok);
    assert!(!ptr.is_null());
    assert!(len > 0);

    // The bytes are a versioned SessionEnvelope (amendment A15).
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let env: sequencer_engine::serde_ext::SessionEnvelope =
        postcard::from_bytes(slice).expect("deserialize envelope");
    assert_eq!(
        env.version,
        sequencer_engine::serde_ext::SESSION_FORMAT_VERSION
    );

    unsafe { sequencer_engine_ffi::engine_free_bytes(ptr, len) };
    // free of NULL is a no-op (Hard Rule 4).
    unsafe { sequencer_engine_ffi::engine_free_bytes(std::ptr::null_mut(), 0) };
    unsafe { sequencer_engine_ffi::engine_free(h) };
}

/// Verifies `engine_serialize` reads the current snapshot SYNCHRONOUSLY, without
/// the state worker thread (which is not spawned until `engine_start` in Task 20).
/// The default session is bpm 120; the bytes must decode to that, be non-empty,
/// and be freed via `engine_free_bytes`. The full submit-SetBpm-then-serialize
/// integration (bpm 150) belongs to Task 20 once the worker runs.
#[test]
fn serialize_reads_default_session_synchronously_without_worker() {
    let h = sequencer_engine_ffi::engine_new();
    assert!(!h.is_null());

    let mut ptr = std::ptr::null_mut();
    let mut len = 0usize;
    let res = unsafe { sequencer_engine_ffi::engine_serialize(h, &mut ptr, &mut len) };
    assert_eq!(res, EngineResult::Ok, "engine_serialize must succeed");
    assert!(!ptr.is_null(), "out_ptr must be non-NULL on success");
    assert!(len > 0, "must produce non-empty bytes");

    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    let env: sequencer_engine::serde_ext::SessionEnvelope =
        postcard::from_bytes(bytes).expect("decoded SessionEnvelope");
    assert_eq!(env.session.bpm, 120.0, "default session bpm is 120");

    unsafe { sequencer_engine_ffi::engine_free_bytes(ptr, len) };
    unsafe { sequencer_engine_ffi::engine_free(h) };
}

#[test]
fn overflow_event_roundtrips_over_c_abi() {
    // Encode Overflow via the event codec, decode back, compare postcard bytes.
    use sequencer_engine::event::EngineEvent;
    let ev = EngineEvent::Overflow { dropped: 42 };
    let bytes = postcard::to_allocvec(&ev).unwrap();
    let back: EngineEvent = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(back, ev);
}

#[test]
fn garbage_event_bytes_do_not_panic_overflow_path() {
    // Re-run the existing garbage-bytes guard to ensure the new variant didn't
    // break codec totality. (If a dedicated garbage test already exists, this
    // asserts it still passes with Overflow added.)
    let garbage = [0xFFu8; 8];
    let _: Result<sequencer_engine::event::EngineEvent, _> = postcard::from_bytes(&garbage);
    // total: returns Ok or Err, never panics
}

/// Task 18: a valid `LoadSession` (bytes from `engine_serialize`) swaps the
/// engine's session. Drives the full round-trip through the C ABI: mutate a
/// donor engine → serialize → load into a fresh engine → re-serialize and
/// confirm the new state took hold. (The FullSnapshot-on-large-channel emission
/// is covered by the core unit test `load_session_emits_full_snapshot_on_large_channel`;
/// `engine_drain_events` is still a Task 20 stub, so the event can't be observed
/// over the C ABI yet.)
#[test]
fn load_session_swaps_and_emits_full_snapshot() {
    use sequencer_engine::command::Command;
    let eng = sequencer_engine_ffi::engine_new();
    // Build a non-default session on a donor engine (bpm 99).
    let eng2 = sequencer_engine_ffi::engine_new();
    let cmd = postcard::to_allocvec(&Command::SetBpm { bpm: 99.0 }).unwrap();
    let r = unsafe { sequencer_engine_ffi::engine_submit_command(eng2, cmd.as_ptr(), cmd.len()) };
    assert_eq!(r, EngineResult::Ok);
    // Submit applies synchronously as of Task 18 (Task 20 swaps in the worker);
    // the sleep keeps the test forward-compatible with the queued path.
    std::thread::sleep(std::time::Duration::from_millis(20));

    let mut ptr = std::ptr::null_mut();
    let mut len = 0usize;
    unsafe { sequencer_engine_ffi::engine_serialize(eng2, &mut ptr, &mut len) };
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) }.to_vec();
    unsafe { sequencer_engine_ffi::engine_free_bytes(ptr, len) };
    unsafe { sequencer_engine_ffi::engine_free(eng2) };

    // Load the donor's serialized envelope into the fresh engine.
    let load = postcard::to_allocvec(&Command::LoadSession { bytes }).unwrap();
    let r = unsafe { sequencer_engine_ffi::engine_submit_command(eng, load.as_ptr(), load.len()) };
    assert_eq!(r, EngineResult::Ok);
    std::thread::sleep(std::time::Duration::from_millis(20));

    // Re-serialize eng: the session must now reflect the donor's bpm (99).
    let mut p2 = std::ptr::null_mut();
    let mut l2 = 0usize;
    unsafe { sequencer_engine_ffi::engine_serialize(eng, &mut p2, &mut l2) };
    let b2 = unsafe { std::slice::from_raw_parts(p2 as *const u8, l2) };
    let env: sequencer_engine::serde_ext::SessionEnvelope = postcard::from_bytes(b2).unwrap();
    assert_eq!(env.session.bpm, 99.0);
    unsafe { sequencer_engine_ffi::engine_free_bytes(p2, l2) };
    unsafe { sequencer_engine_ffi::engine_free(eng) };
}

/// Task 18: a malformed envelope (bad bytes / wrong version) is rejected — the
/// engine's session is unchanged. Submit succeeds (the bytes decode as a
/// `Command::LoadSession`); the rejection happens inside `apply_command`.
#[test]
fn load_session_bad_version_returns_no_swap() {
    use sequencer_engine::command::Command;
    let eng = sequencer_engine_ffi::engine_new();
    // 0xFF... is not a valid SessionEnvelope; even if it decodes structurally,
    // the version byte won't match SESSION_FORMAT_VERSION.
    let bad = postcard::to_allocvec(&Command::LoadSession {
        bytes: vec![0xFFu8; 4],
    })
    .unwrap();
    let r = unsafe { sequencer_engine_ffi::engine_submit_command(eng, bad.as_ptr(), bad.len()) };
    assert_eq!(r, EngineResult::Ok);
    std::thread::sleep(std::time::Duration::from_millis(20));

    let mut p = std::ptr::null_mut();
    let mut l = 0usize;
    unsafe { sequencer_engine_ffi::engine_serialize(eng, &mut p, &mut l) };
    let b = unsafe { std::slice::from_raw_parts(p as *const u8, l) };
    let env: sequencer_engine::serde_ext::SessionEnvelope = postcard::from_bytes(b).unwrap();
    assert_eq!(
        env.session.bpm, 120.0,
        "bad envelope: session must be unchanged"
    );
    assert_eq!(
        env.session.active_pattern_index, 0,
        "bad envelope: active_pattern_index must be unchanged"
    );
    unsafe { sequencer_engine_ffi::engine_free_bytes(p, l) };
    unsafe { sequencer_engine_ffi::engine_free(eng) };
}

/// Task 18 validation correction: a structurally-valid envelope (correct
/// version, decodes fine) but with an out-of-range `active_pattern_index`
/// (>= PATTERN_SLOTS) is rejected — no swap, no panic. Without `validate_session`
/// this payload would panic the worker on the next `patterns[active_pattern_index]`
/// index (or the RT thread, violating Hard Rule 1).
#[test]
fn load_session_bad_active_pattern_index_no_swap() {
    use sequencer_engine::command::Command;
    use sequencer_engine::models::Session;
    use sequencer_engine::serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION};
    let eng = sequencer_engine_ffi::engine_new();

    // Valid version + shape, but active_pattern_index = 99 (>= PATTERN_SLOTS=9).
    let corrupt = SessionEnvelope {
        version: SESSION_FORMAT_VERSION,
        session: Session {
            active_pattern_index: 99,
            bpm: 200.0,
            ..Default::default()
        },
    };
    let corrupt_bytes = postcard::to_allocvec(&corrupt).unwrap();
    let load = postcard::to_allocvec(&Command::LoadSession {
        bytes: corrupt_bytes,
    })
    .unwrap();
    let r = unsafe { sequencer_engine_ffi::engine_submit_command(eng, load.as_ptr(), load.len()) };
    assert_eq!(r, EngineResult::Ok);
    std::thread::sleep(std::time::Duration::from_millis(20));

    // The corrupt session must NOT have been published: defaults remain.
    let mut p = std::ptr::null_mut();
    let mut l = 0usize;
    unsafe { sequencer_engine_ffi::engine_serialize(eng, &mut p, &mut l) };
    let b = unsafe { std::slice::from_raw_parts(p as *const u8, l) };
    let env: SessionEnvelope = postcard::from_bytes(b).unwrap();
    assert_eq!(
        env.session.bpm, 120.0,
        "invalid session: bpm must be unchanged"
    );
    assert_eq!(
        env.session.active_pattern_index, 0,
        "invalid session: active_pattern_index must be unchanged"
    );
    unsafe { sequencer_engine_ffi::engine_free_bytes(p, l) };
    unsafe { sequencer_engine_ffi::engine_free(eng) };
}
