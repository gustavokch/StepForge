import Foundation

/// Saves/loads the session bytes produced by engine_serialize and consumed by
/// Command.loadSession. The app plan wires the FFI round-trip + error handling.
enum SessionStore {
    static func save(_ data: Data) throws {
        try data.write(to: try url(), options: .atomic)
    }

    static func load() throws -> Data {
        try Data(contentsOf: url())
    }

    private static func url() throws -> URL {
        let dir = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true
        )
        return dir.appendingPathComponent("stepforge.session")
    }
}
