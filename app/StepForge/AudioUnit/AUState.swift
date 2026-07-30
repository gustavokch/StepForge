import Foundation

/// Pure pack/unpack for the AU's `fullState` dictionary. Both `fullState` and
/// `fullStateForDocument` use the same `["session": Data]` envelope: serialize on
/// get, `loadSession` on set. Pure (no handle, no side effects) → unit-testable
/// without an AU or host, and safe for the iOS app + test targets (mirrors
/// `HostTransportBuilder`: pure helper, no `#if os(macOS)`).
enum AUState {
    /// Dictionary key under which the serialized session bytes are stored.
    static let sessionKey = "session"

    /// Wrap serialized session bytes in the `[String: Any]` envelope the host
    /// reads/writes via `fullState` / `fullStateForDocument`.
    static func pack(_ data: Data) -> [String: Any] { [sessionKey: data] }

    /// Recover the session bytes from the host-provided dictionary, or `nil` if
    /// the key is missing / the value is not `Data` (defensive — hosts may pass
    /// arbitrary dictionaries through this accessor).
    static func unpack(_ dict: [String: Any]) -> Data? { dict[sessionKey] as? Data }
}
