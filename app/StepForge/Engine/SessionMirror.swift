import Foundation

/// Value-type mirror of engine state, updated only by applying EngineEvents on
/// the MainActor. Real shape + apply(_:) land in the app plan.
struct SessionMirror {
    var bpm: Double = 120
    var playing: Bool = false
}
