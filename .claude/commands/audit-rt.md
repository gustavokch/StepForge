---
description: Audit Rust files for real-time-safety violations on the RT-thread path
---

Audit the StepForge real-time thread path for real-time safety.

Scope: the RT-thread code path — `engine/crates/core/src/{clock,scheduler,midi}.rs`, the RT-side encoder in `engine/crates/ffi/src/event_codec.rs`, and any other RT-reachable code. If file paths are given in **$ARGUMENTS**, focus there; otherwise audit the full RT path above.

The RT thread **MUST NEVER**, on any code path it executes:

- allocate on the hot path — no `Vec`, `String`, `Box`, `format!`, `to_string()`, `.collect()`, or returning owned heap types;
- lock or block — no `std::sync::Mutex` / `RwLock` / `Barrier` / `parking_lot` blocking primitives, no I/O, no system calls;
- cross the FFI boundary or call into Swift;
- call CoreMIDI directly (`MIDISend` must run on the CoreMIDI worker thread, fed by the fixed-slot ring).

For each violation, report `file:line`, the offending construct, and a lock-free / allocation-free alternative (fixed-size `[u8; N]` buffers, `encode_event_into` into a caller buffer, `heapless` / `rtrb` / atomics, hand-off to the worker thread). Also confirm `engine/crates/core/src/lib.rs` still begins with `#![forbid(unsafe_code)]`.

Report findings as a prioritized list (blockers first). Do **not** edit files unless explicitly asked.
