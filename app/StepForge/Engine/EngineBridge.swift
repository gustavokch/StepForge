import Foundation
import SwiftUI
import os

/// Wraps the Rust FFI. Owns the engine handle lifecycle (CLAUDE.md Hard Rule 5)
/// and is the **sole** path between SwiftUI and the engine. SwiftUI observes the
/// `@Published mirror` (a value type); the UI never holds a pointer into engine
/// memory (Hard Rule 2).
///
/// Threading (amendments A3/E7/E11 + Hard Rule 5 "no concurrent engine_* calls"):
/// a serial `DispatchQueue` is the serialization point for **every** handle-touching
/// call — the ~120 Hz `DispatchSourceTimer` drain, `submit`, `serialize`, `start`,
/// and `stop` all run through it (submit/serialize via `drainQueue.sync`). That
/// guarantees no two `engine_*` calls overlap on the handle (the Rust `run_void`
/// takes a unique `&mut Engine`, so any overlap would be UB). Within a drain tick
/// the loop pulls events (`engine_drain_events` — one per call, empty/zero =
/// drained), frees each Rust buffer immediately via `engine_free_bytes` (Hard Rule
/// 4), coalesces `Playhead` per-track, clears stale playheads when a pattern switch
/// is seen in the batch, then makes **one** MainActor hop per batch (E7).
///
/// Forward-compatible: against the (currently stubbed) engine, `drain` returns
/// empty and the bridge simply idles — it compiles and runs today, and lights up
/// unchanged once the engine emits events.
class EngineBridge: ObservableObject, @unchecked Sendable {

    /// The value-type mirror SwiftUI reads. File-private setter: only this file
    /// (the bridge + its mock subclass) mutates it, always on the main thread.
    @Published fileprivate(set) var mirror = SessionMirror()

    /// Lock-protected snapshot of the sync-critical mirror fields, refreshed on the
    /// MainActor at the tail of each drain batch. Off-main readers — the CoreMIDI
    /// input callback wired by `MidiManager.bind(to:)` — must NOT touch `mirror`
    /// (mutated MainActor-only); they read `currentSyncSource` / `currentBpm`
    /// instead (Issue #1 data race). `mirror` stays the `@Published` source of
    /// truth for SwiftUI; this snapshot is unpublished, so it never triggers a
    /// redraw and adds no main-thread churn. `let` + `withLock` = interior
    /// mutability, so it is safe to capture from any thread.
    private struct SyncSnapshot {
        var syncSource: SyncSource = .free
        var bpm: Double = 120.0
    }
    private let syncState = OSAllocatedUnfairLock(initialState: SyncSnapshot())

    /// Off-main-safe read of the current sync source (one lock acquisition).
    var currentSyncSource: SyncSource { syncState.withLock { $0.syncSource } }
    /// Off-main-safe read of the current BPM (one lock acquisition).
    var currentBpm: Double { syncState.withLock { $0.bpm } }

    /// Copy the mirror's sync fields into the lock-protected snapshot. Called on
    /// the MainActor as the tail of each drain batch, and by the mock's optimistic
    /// echo, so off-main readers (`currentSyncSource` / `currentBpm`) see the
    /// latest applied state. (Issue #1) `fileprivate` so the same-file subclass
    /// `MockEngineBridge` can call it.
    fileprivate func refreshSyncSnapshot() {
        syncState.withLock { snap in
            snap.syncSource = mirror.syncSource
            snap.bpm = mirror.bpm
        }
    }

    /// Opaque handle. Pass it only back to the FFI; **never** dereference it
    /// (Hard Rule 2). Created by `makeHandle()` (overridable so the mock is FFI-free).
    private var handle: UnsafeMutablePointer<EngineHandle>?
    var hasHandle: Bool { handle != nil }

    /// Single serialization queue for all handle-touching FFI calls (Hard Rule 5).
    private let drainQueue = DispatchQueue(label: "engine.drain", qos: .userInitiated)
    private var drainTimer: DispatchSourceTimer?
    private var didStop = false

    init() { handle = makeHandle() }

    /// Handle factory. The real engine calls `engine_new()`; the mock overrides to
    /// return nil so it never touches (or links) the FFI.
    func makeHandle() -> UnsafeMutablePointer<EngineHandle>? { engine_new() }

    // MARK: - Lifecycle (Hard Rule 5: stop returns before free)

    /// Spawn the RT/state/CoreMIDI threads (engine side) and begin draining.
    func start() {
        drainQueue.sync {
            guard self.handle != nil, self.drainTimer == nil else { return; }
            if let h = self.handle { _ = engine_start(h); }
            let timer = DispatchSource.makeTimerSource(queue: self.drainQueue)
            timer.schedule(deadline: .now() + .milliseconds(8), repeating: .milliseconds(8)) // ~120 Hz
            timer.setEventHandler { [weak self] in self?.drainOnce(); }
            timer.resume()
            self.drainTimer = timer
        }
    }

    /// Cancel the drain timer and stop the engine. Must return before `engine_free`
    /// (Hard Rule 5). `drainQueue.sync` serializes against any in-flight drain/submit.
    func stop() {
        drainQueue.sync {
            self.drainTimer?.cancel()
            self.drainTimer = nil
            if let h = self.handle { _ = engine_stop(h); }
            self.didStop = true
        }
    }

    deinit {
        drainTimer?.cancel()
        let h = handle
        let stopped = didStop
        drainQueue.sync {
            if let h, !stopped { _ = engine_stop(h) }
            if let h { engine_free(h) }
        }
    }

    // MARK: - Commands (non-blocking; ErrDecode is non-fatal — Hard Rule 3)

    /// Encode + submit a command. Serialized on `drainQueue` so it can't overlap any
    /// other handle call (submit is fast: encode + MPSC push, so the MainActor stall
    /// is bounded by at most one drain tick).
    func submit(_ command: Command) {
        let bytes = command.encode()
        drainQueue.sync {
            guard let h = self.handle else { return; }
            bytes.withUnsafeBufferPointer { buf in
                guard let base = buf.baseAddress else { return; }
                _ = engine_submit_command(h, base, UInt(buf.count))
            }
        }
    }

    func requestSnapshot() { submit(.requestFullSnapshot); }

    /// Serialize the current session to a `SessionEnvelope` (`Data`). The handle
    /// call is serialized on `drainQueue`; the buffer is copied and freed via
    /// `engine_free_bytes` exactly once (Hard Rule 4 — `free_bytes` is not a handle
    /// call, so it need not be serialized).
    func serialize() -> Data? {
        var ptr: UnsafeMutablePointer<UInt8>? = nil
        var len: UInt = 0
        drainQueue.sync {
            guard let h = self.handle else { return; }
            _ = engine_serialize(h, &ptr, &len)
        }
        guard let p = ptr, len > 0 else { return nil; }
        defer { engine_free_bytes(p, len); }
        return Data(bytes: p, count: Int(len))
    }

    func load(_ data: Data) { submit(.loadSession(bytes: Array(data))); }

    // MARK: - Drain (runs on drainQueue; mutates mirror on main only)

    private func drainOnce() {
        guard let h = handle else { return; }
        var events: [EngineEvent] = []
        var playheads: [Int: Int] = [:]   // trackIdx -> latest stepIdx (coalesced)
        while true {
            var outPtr: UnsafeMutablePointer<UInt8>? = nil
            var outLen: UInt = 0
            _ = engine_drain_events(h, &outPtr, &outLen)
            guard outLen > 0, let p = outPtr else { break; }   // empty = drained (A13)
            let copy = Array(UnsafeBufferPointer(start: p, count: Int(outLen)))
            engine_free_bytes(p, outLen)                        // borrowed + freed immediately
            guard let event = EngineEvent.decode(copy) else {
                print("[EngineBridge] ERROR: Failed to decode EngineEvent of length \(outLen) bytes!")
                continue
            } // malformed → drop
            if case .playhead(let t, let s) = event {
                playheads[t] = s
            } else {
                if case .patternSwitched = event {
                    playheads.removeAll(keepingCapacity: true)
                }
                events.append(event)
            }
        }
        guard !events.isEmpty || !playheads.isEmpty else { return; }
        // One MainActor hop per batch (E7).
        DispatchQueue.main.async { [weak self] in
            guard let self else { return; }
            for event in events { self.mirror.apply(event); }
            for (t, s) in playheads { self.mirror.applyPlayhead(trackIdx: t, stepIdx: s); }
            // Refresh the off-main-readable sync snapshot from freshly-applied
            // state — last, so it reflects this batch. (Issue #1)
            self.refreshSyncSnapshot()
        }
    }
}

/// In-process, FFI-free test/preview double. Overrides `makeHandle()` to return nil
/// so it never calls `engine_new()` (and never needs the xcframework linked).
/// Subclasses `EngineBridge` so it is a valid `@EnvironmentObject` of the concrete
/// type. `submit` echoes each command's musical effect onto the mirror optimistically
/// (so UI tests can assert tap→update against the stubbed engine); drain/start/stop
/// are no-ops. Seeded with `SessionMirror.demoSeed` for rich previews/screenshots.
final class MockEngineBridge: EngineBridge, @unchecked Sendable {
    override func makeHandle() -> UnsafeMutablePointer<EngineHandle>? { nil }

    override init() {
        super.init()
        mirror = .demoSeed
    }

    override func start() { /* no threads, no drain */ }
    override func stop() { /* nothing to stop; handle is nil */ }
    override func requestSnapshot() { /* mirror already seeded */ }
    override func serialize() -> Data? { nil }
    override func load(_ data: Data) { /* ignore in mock */ }

    override func submit(_ command: Command) {
        mirror.applyOptimistic(command)
        refreshSyncSnapshot()   // keep the off-main-readable snapshot consistent with the mock's mirror
    }
}
