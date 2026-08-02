import XCTest
@testable import StepForge

final class SessionMirrorUndoTests: XCTestCase {
    /// Regression (PR #29): Swift twin of the Rust
    /// `full_snapshot_after_undo_available_keeps_undo_enabled`. The engine
    /// co-emits `undoAvailable` (hot) + `fullSnapshot` (large) after an
    /// algorithm/clipboard/undo command. `.fullSnapshot` must NOT clear
    /// `undoAvailable`, or the Undo button can never enable — the original bug
    /// was this arm wiping the flag on every command echo. Locks the mirror
    /// symmetry so a future refactor that re-adds `undoAvailable.removeAll()`
    /// here is caught without a manual DAW/iOS smoke.
    func testUndoAvailableSurvivesFullSnapshot() {
        var mirror = SessionMirror()
        mirror.apply(.undoAvailable(trackIdx: 2, available: true))
        mirror.apply(.fullSnapshot(session: Session()))
        XCTAssertTrue(
            mirror.undoAvailable.contains(2),
            "fullSnapshot must not wipe the undoAvailable just emitted for the same command"
        )
    }
}
