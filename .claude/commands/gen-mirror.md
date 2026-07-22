---
description: Generate Swift mirror structs and SessionMirror.apply() boilerplate from a Rust model
---

Generate the Swift side of a Rust model that participates in an `EngineEvent`. **$ARGUMENTS** is the Rust type (and, if relevant, the event variant) — e.g. `RollConfig`, or `Step` with `StepChanged`.

Steps:

1. Read the Rust type from `engine/crates/core/src/models.rs` and the event variant carrying it from `engine/crates/core/src/event.rs`.
2. Emit a Swift **value-type** mirror struct under `app/StepForge/Engine/` (e.g. `StepMirror`, `RollConfigMirror`), mapping logical Rust fields to Swift: `VelocityZone`→`VelocityZone`, `Ratchet`→`Ratchet`, `QuantizeGrain`→`QuantizeGrain`, `Uuid`→`UUID`, `usize`→`Int`, `u8`→`UInt8`, `f32`/`f64`→`Float`/`Double`, `Vec<T>`→`[T]`, fixed `[Step; 16]`→`[StepMirror]` (count 16), `bool`→`Bool`, `Option<T>`→`T?`. Add `Identifiable`/`Hashable` only if the Rust type has an `id` or the UI needs identity. No engine pointers — mirror only.
3. If the type is session-scoped, add the field to `SessionMirror` (with a default).
4. Add the matching `case` to `SessionMirror.apply(_:)` that updates the mirror from the decoded `EngineEvent`.

Constraints: the Swift mirror mirrors *logical* fields — Swift decodes the postcard `EngineEvent` bytes (in `EngineEvent.swift`), not the Rust struct, so the mirror need not match byte layout. Do not generate the postcard decoder itself. Mutations land on the MainActor only; the mirror is SwiftUI's single source of truth. Do not touch SwiftUI views unless asked.
