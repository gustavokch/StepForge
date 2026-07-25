//! Regenerates the golden postcard fixtures consumed by the Swift `PostcardTests`
//! (`app/StepForgeTests/Fixtures/*.bin`). Those `.bin` files are the byte-for-byte
//! oracle for the Swift codec (architecture-spec §11.2): whenever the Rust
//! `Command` / `EngineEvent` / model wire layout changes — e.g. a variant is
//! inserted, reordering every later variant tag — re-run this to refresh them.
//!
//! Run (from `engine/`, rustup toolchain on PATH):
//!     cargo test -p sequencer_engine_core --test regen_fixtures -- --ignored
//!
//! Today only the event fixtures are emitted (the command/model fixtures are
//! unchanged). Extend the `fixtures` table to regenerate more.

#![cfg(test)]

use sequencer_engine::event::EngineEvent;
use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    // core manifest = engine/crates/core → repo root is three `..`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../app/StepForgeTests/Fixtures")
}

#[test]
#[ignore = "fixture-regeneration tool — run explicitly with --ignored"]
fn regen_event_fixtures() {
    let dir = fixtures_dir();
    assert!(dir.exists(), "fixtures dir not found at {}", dir.display());

    let fixtures: &[(&str, EngineEvent)] = &[
        ("ev_bpm_123", EngineEvent::BpmChanged { bpm: 123.0 }),
        (
            "ev_playhead_2_7",
            EngineEvent::Playhead {
                track_idx: 2,
                step_idx: 7,
            },
        ),
        ("ev_playstate_true", EngineEvent::PlayStateChanged { playing: true }),
        ("ev_overflow_7", EngineEvent::Overflow { dropped: 7 }),
        (
            "ev_undoavail_1_true",
            EngineEvent::UndoAvailable {
                track_idx: 1,
                available: true,
            },
        ),
        (
            "ev_error_m7_boom",
            EngineEvent::Error {
                code: -7,
                message: "boom".to_string(),
            },
        ),
    ];

    for (name, ev) in fixtures {
        let bytes = postcard::to_allocvec(ev).expect("serialize event");
        fs::write(dir.join(format!("{name}.bin")), bytes)
            .unwrap_or_else(|e| panic!("write {name}.bin: {e}"));
    }
}
