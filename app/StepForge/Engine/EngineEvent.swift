import Foundation

/// Swift mirror of EngineEvent + postcard decoder. Stub: the app plan implements
/// decode([UInt8]) -> EngineEvent? to match Rust event_codec.
enum EngineEvent {
    case playStateChanged(Bool)
    case bpmChanged(Double)
}
