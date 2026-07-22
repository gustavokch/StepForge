import Foundation

/// Swift mirror of the Command enum + postcard encoder. Stub: the app plan
/// implements encode() -> [UInt8] to match Rust command_codec.
enum Command {
    case play
    case stop
    case setBpm(Double)
}
