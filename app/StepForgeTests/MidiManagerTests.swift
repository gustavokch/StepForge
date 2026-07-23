import XCTest
@testable import StepForge

final class MidiManagerTests: XCTestCase {
    func testMidiManagerInitialization() {
        let manager = MidiManager()
        XCTAssertNotNil(manager.destinations)
    }

    func testMidiManagerToggleSubmitsDestinations() {
        let bridge = MockEngineBridge()
        let manager = MidiManager()
        
        manager.toggleDestination(1001, on: bridge)
        XCTAssertTrue(manager.selectedIDs.contains(1001))
        
        manager.toggleDestination(1001, on: bridge)
        XCTAssertFalse(manager.selectedIDs.contains(1001))
    }
}
