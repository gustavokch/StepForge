---
description: Generate a proptest asserting the invariants of a StepForge algorithm
---

Generate a `proptest` for a StepForge algorithm. **$ARGUMENTS** names the algorithm/function (e.g. `vary`, `roll`) and/or the invariant to check.

Steps:

1. Locate the algorithm in `engine/crates/core/src/algorithms/` and read its documented invariants (also in `docs/specs/amendments.md` and `CLAUDE.md`).
2. Add a `proptest` in the algorithm's `#[cfg(test)]` module (or `crates/core/tests/`) using `proptest::prelude::*`.
3. Build input strategies for valid inputs: a random `Track` (random `length` in `1..=16`, random `steps`, a mix of accents), `strength` in `0.0..=1.0`, and a **deterministic** RNG strategy where the algorithm accepts an `rng` (so failures are reproducible, not flaky).
4. Assert the invariants for **every** generated input, e.g.:
   - `track.length` is unchanged before/after;
   - `track.midi_note` and `track.speed_ratio` are unchanged;
   - for `vary`: every step that was `VelocityZone::Accent` before is byte-for-byte unchanged after;
   - density is bounded by `strength` where the spec defines a relationship.
5. Run `cargo test -p sequencer_engine` and report. Do **not** weaken an invariant to make a test pass — if a run fails, treat it as a real bug and say so.

Add `proptest` to `crates/core` `[dev-dependencies]` (it is not present yet).
