---
description: Verify FFI-boundary code obeys the C-ABI safety and ownership rules
---

Enforce the StepForge FFI boundary rules. Scope: `engine/crates/ffi/src/{lib.rs,handle.rs,coremidi.rs,command_codec.rs,event_codec.rs}` and `app/StepForge/Engine/EngineBridge.swift`. If **$ARGUMENTS** names files, focus there.

Verify every one of these holds:

- **No Swift object crosses the boundary** — only raw pointers (`*const u8` / `*mut u8` / opaque handle), lengths, and primitives cross `extern "C"`. No Swift class/struct instance is passed through a `@convention(c)` boundary.
- **Null-safe pointer handling** — every Rust pointer deref is inside `unsafe`; out-params (`*mut *mut u8`, `*mut usize`) and the engine handle are NULL-checked (`engine_free` / `engine_free_bytes` tolerate NULL; other entries return `ErrInvalidHandle` / `ErrInvalidBuffer`).
- **Panics never cross** — every `extern "C"` body is wrapped in `catch_unwind` and returns `EngineResult`; the postcard codecs are total (`Result`, no `panic!` / `unwrap` / `expect` on the FFI path).
- **Bytes, not structs** — commands and events cross as postcard bytes; no data-carrying `#[repr(C)]` enum is passed across the ABI.
- **Buffer ownership** — buffers returned by `engine_drain_events` / `engine_serialize` are Rust-allocated and freed only by `engine_free_bytes`, exactly once; command bytes passed in are Swift-owned and borrowed by Rust for the call only.
- **CoreMIDI ownership stays in Swift** — `MIDIClientRef` / endpoint lifecycle lives in Swift; Rust stores only integer endpoint IDs.
- **RT thread never calls FFI** — `extern "C"` entries are invoked from Swift (main/background), never from the engine's RT thread.

Report each violation as `file:line: <rule> — <problem> → <fix>`. Rewrite the offending code if asked; otherwise just report.
