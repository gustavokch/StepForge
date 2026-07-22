import Foundation

/// Swift mirror of the Command enum + postcard encoder. Stub: the app plan
/// implements encode() -> [UInt8] to match Rust command_codec.
enum Command {
    case play
    case stop
    case setBpm(Double)
    // TODO(app-plan): Rust `Command::LoadSession { bytes: Vec<u8> }` (amendment
    // A15) has no Swift mirror yet — add `loadSession(Data)` + its postcard
    // encoder here to match.
}
