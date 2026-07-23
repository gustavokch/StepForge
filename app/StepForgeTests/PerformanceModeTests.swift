import XCTest
@testable import StepForge

final class PerformanceModeTests: XCTestCase {
    func testQueuePatternDispatchesCommand() {
        let bridge = MockEngineBridge()
        
        bridge.submit(.queuePattern(index: 2, quantize: .nextBar))
        XCTAssertEqual(bridge.mirror.queuedPatternIndex, 2)
        
        bridge.submit(.cancelQueuedPattern)
        XCTAssertNil(bridge.mirror.queuedPatternIndex)
    }

    func testFollowActionSettingDispatches() {
        let bridge = MockEngineBridge()
        let action = FollowAction(afterLoops: 4, action: .playNext)
        
        bridge.submit(.setFollowAction(patternIdx: 0, action: action))
        XCTAssertEqual(bridge.mirror.patterns[0]?.followAction.afterLoops, 4)
        XCTAssertEqual(bridge.mirror.patterns[0]?.followAction.action, .playNext)
    }
}
