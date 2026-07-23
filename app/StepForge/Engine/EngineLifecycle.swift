import SwiftUI

/// Maps SwiftUI scene phase to engine lifecycle (CLAUDE.md Hard Rule 5: stop
/// returns before free). Pure mapping — the bridge is supplied by the caller, so
/// this stays trivially testable and free of ownership concerns.
enum EngineLifecycle {
    static func handle(_ phase: ScenePhase, on bridge: EngineBridge) {
        switch phase {
        case .active:
            bridge.start()
            bridge.requestSnapshot()
        case .inactive:
            break
        case .background:
            bridge.stop()
        @unknown default:
            break
        }
    }
}
