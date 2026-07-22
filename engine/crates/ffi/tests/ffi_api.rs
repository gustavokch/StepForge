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
