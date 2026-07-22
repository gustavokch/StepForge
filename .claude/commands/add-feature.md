---
description: Add a cross-layer feature touching both the Rust core and the Swift shell, with no orphans
---

Add the following cross-layer feature end-to-end across the Rust/Swift boundary, with no orphaned half-implementations:

**Feature:** $ARGUMENTS

Work through every step that applies, in order, and state which you completed:

1. **Rust models** — add/adjust types in `engine/crates/core/src/models.rs` (with serde derives, no `#[repr(C)]`).
2. **Rust contract** — add the `Command` variant (`command.rs`) and/or `EngineEvent` variant (`event.rs`).
3. **Rust codecs** — update `engine/crates/ffi/src/{command_codec,event_codec}.rs` (postcard). If a new event can exceed `MAX_EVENT_BYTES`, route it via the off-RT large-event channel instead of the hot fixed-slot channel; the RT encoder must keep using `encode_event_into` into a caller buffer (never `Vec`).
4. **Rust tests** — add a codec round-trip + a C-ABI test in `engine/crates/ffi/tests/`; for algorithms, add a `proptest` asserting the invariants (e.g. `length`/`midi_note`/`speed_ratio` unchanged).
5. **Swift mirror** — update the `Command`/`EngineEvent` mirrors + encoder/decoder in `app/StepForge/Engine/`, and `SessionMirror.apply(_:)` for any new event.
6. **Regenerate** — run `engine/scripts/build_engine.sh` (regenerates the header) and `cd app && xcodegen generate` if project structure changed.

Constraints: keep the RT path allocation-free; every `extern "C"` stays panic-safe (`catch_unwind` + `EngineResult`); `engine/crates/core` must remain `#![forbid(unsafe_code)]`. **Do not touch SwiftUI views** unless the feature explicitly requires UI — keep view changes as a separate, requested step.

Finish by running `cd engine && cargo test` and confirming it is green; report any step you skipped and why.
