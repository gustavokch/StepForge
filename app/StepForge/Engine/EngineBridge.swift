import Foundation

/// Wraps the Rust FFI. Foundation stub: creates the engine and owns the handle
/// lifecycle (CLAUDE.md Hard Rule 5). The event drain loop, SessionMirror updates,
/// and codec live in the app plan.
final class EngineBridge {
    // Opaque token — pass it only back to the FFI. `EngineHandle` is a
    // zero-sized struct in the C header, so Swift sees an `UnsafeMutablePointer`,
    // but it must NEVER be dereferenced (CLAUDE.md Hard Rule 2: no long-lived
    // pointer into engine state).
    private var handle: UnsafeMutablePointer<EngineHandle>?
    var hasHandle: Bool { handle != nil }

    init() {
        handle = engine_new()
    }

    /// Start the engine (stub: no-op on the Rust side). Must be called before free.
    func start() {
        guard let handle else { return }
        engine_start(handle)
    }

    /// Stop the engine. Call from scene-phase teardown before deinit frees the handle.
    func stop() {
        guard let handle else { return }
        engine_stop(handle)
    }

    deinit {
        // Hard Rule 5 backstop: stop before free. The app plan wires scene-phase stop().
        if let handle {
            engine_stop(handle)
            engine_free(handle)
        }
    }
}
