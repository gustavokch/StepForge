#if os(macOS)
import AudioToolbox
import CoreAudioKit
import AVFoundation

/// Phase 1 AUv3 audio unit (`'aumi'` MIDI-FX, `SFor`/`DrmS`). Owns the
/// host-driven engine handle + render-state + lifecycle (Hard Rule 5).
/// `internalRenderBlock` runs on the host RT thread (Hard Rule 1).
///
/// The AU is the sole owner of the engine handle. `internalRenderBlock`
/// (cached once in `allocateRenderResources`) reads transport from the host
/// context blocks, walks the incoming `AURenderEvent` list directly into a
/// fixed `MidiEvent` buffer (no allocation), calls `engine_render`, and
/// forwards emitted `MidiEvent`s to the host via `MIDIOutputEventBlock`
/// (3 bytes built on the stack). A borrowed `EngineBridge` drains engine
/// events for the editor on a separate queue — it never touches the RT path.
final class StepForgeAudioUnit: AUAudioUnit {

    // MARK: - Bus (built at init; auval probes outputBusses pre-allocation)

    private var _outputBusses: AUAudioUnitBusArray!

    // MARK: - Engine ownership (Phase 0 host-driven). The AU is the sole lifecycle owner.

    /// Opaque host-driven engine handle. Created in `init`, freed in `deinit`.
    private var engine: UnsafeMutablePointer<EngineHandle>?
    /// Per-instance render state (`RenderStateHandle*`). RT-thread-only.
    private var renderState: UnsafeMutablePointer<RenderStateHandle>?
    /// Borrowed-handle bridge for the editor's command/event path (drain timer only).
    /// Does NOT own the handle; the AU does. Never called from the RT thread.
    private var bridge: EngineBridge?

    // MARK: - Fixed RT buffers (allocated once in init; no allocation on the hot path)

    /// Incoming MIDI events, walked directly from `AURenderEvent`s. Cap matches
    /// `MIDIMarshaler.inCapacity` (drop-tail bounded → RT-safe).
    private let midiIn: UnsafeMutablePointer<MidiEvent>
    /// Outgoing MIDI events from `engine_render`. 256 is the Swift-side fixed
    /// buffer cap (far exceeds anything a drum sequencer emits per block); the
    /// engine honors whatever `outCap` is passed at the `engine_render` call site.
    private let midiOut: UnsafeMutablePointer<MidiEvent>
    private static let midiOutCap: UInt = 256

    // MARK: - Cached render block (built once in allocateRenderResources)

    /// Cached in `allocateRenderResources` so the closure is not re-created per render.
    /// Nil before allocation (getter returns a silence stub) and after deallocation.
    private var _internalRenderBlock: AUInternalRenderBlock?

    override init(componentDescription: AudioComponentDescription,
                 options: AudioComponentInstantiationOptions = []) throws {
        // Fixed RT buffers first: allocate into locals, then assign to the `let`
        // properties. The cleanup `defer` frees the LOCALS — Swift forbids `self`
        // access in a defer reachable from a throwing super.init, so referencing the
        // stored properties there would not compile. The locals hold the same
        // pointers, so on a throw the buffers are freed; on success they're owned
        // by the properties (and freed in deinit).
        let inBuf = UnsafeMutablePointer<MidiEvent>.allocate(capacity: MIDIMarshaler.inCapacity)
        let outBuf = UnsafeMutablePointer<MidiEvent>.allocate(capacity: Int(StepForgeAudioUnit.midiOutCap))
        midiIn = inBuf
        midiOut = outBuf
        // If super.init or the bus construction below throws, deinit does NOT run
        // (the object never finished initializing), so these allocations would
        // leak. Deallocate on throw only — on success `didCompleteInit` flips last.
        var didCompleteInit = false
        defer { if !didCompleteInit { inBuf.deallocate(); outBuf.deallocate() } }
        try super.init(componentDescription: componentDescription, options: options)

        // Build the bus array at init so `outputBusses` is non-empty before
        // `allocateRenderResources` runs (auval probes the bus list early;
        // returning an empty array from the base class yields
        // `kAudioUnitErr_InvalidScope` = -10875 during initialization).
        // The bus array retains the bus — no separate stored ref needed.
        let format = AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 2)!
        let bus = try AUAudioUnitBus(format: format)
        _outputBusses = AUAudioUnitBusArray(audioUnit: self, busType: .output, busses: [bus])

        // Create the host-driven engine + spawn ONLY the host-driven state worker
        // (engine_start on a host-driven handle arms the state worker; it does
        // NOT spawn the self-scheduled RT/MIDI threads — the host drives those
        // via internalRenderBlock → engine_render). Never NULL per the C header.
        engine = engine_new_host_driven()
        // Arm the host-driven state worker; if it fails, LoadSession/fullState
        // restore silently no-ops — log it so the failure is visible. The bridge
        // is still created (drain/submit/serialize work regardless of the state
        // worker; only restore is affected), which keeps `bridge` contractually
        // non-nil for `bridgeForEditor()`. engine_new_host_driven() never returns
        // NULL (C header contract).
        if let e = engine {
            if engine_start(e).rawValue != 0 {   // EngineResult::Ok == 0 (sequencer_engine.h)
                print("[StepForgeAudioUnit] ERROR: engine_start failed — LoadSession/fullState restore will no-op")
            }
            // Borrowed bridge for the editor's command/event path (drain timer
            // only). The bridge does NOT own the handle (AU does) and skips
            // engine_start/stop/free.
            bridge = EngineBridge(handle: e)
            bridge?.start()
        }
        didCompleteInit = true
    }

    deinit {
        // Hard Rule 5: stop returns before free; no concurrent engine_* calls.
        // The bridge's drain queue serializes its OWN handle-touching calls
        // (submit/serialize/drainOnce), but the editor/VC holds a strong ref to
        // the borrowed bridge — `bridge = nil` does NOT deallocate it, so an
        // in-flight editor `submit` could touch the handle on `drainQueue` while
        // the AU's stop/free run here on the dealloc thread. `quiesce()` nils the
        // bridge's handle under `drainQueue.sync` (after `stop()` drained any
        // running tick), so once it returns no bridge path can reach the handle
        // and the AU's stop/free are safe from overlap.
        bridge?.stop()
        bridge?.quiesce()
        bridge = nil
        if let e = engine {
            _ = engine_stop(e)
            engine_free(e)
        }
        // Defensive: deallocateRenderResources normally frees renderState and nils
        // it; this covers the path where it never ran.
        if let rs = renderState {
            engine_render_state_free(rs)
            renderState = nil
        }
        midiIn.deallocate()
        midiOut.deallocate()
    }

    // MARK: - Bus / channel capabilities

    override var outputBusses: AUAudioUnitBusArray { _outputBusses }

    /// Expose the borrowed editor bridge to the view controller. `bridge` is
    /// contractually non-nil whenever the AU reached editor binding (the engine
    /// path is armed at init before `bridge` is constructed, and
    /// `engine_new_host_driven()` never returns NULL), so the force-unwrap makes
    /// that contract explicit rather than masking a nil with a dead, leaking
    /// standalone engine. `internal` access: the AU and its VC are in the same
    /// target/module but separate files, so `fileprivate` (per the brief) blocks
    /// cross-file use.
    func bridgeForEditor() -> EngineBridge { bridge! }

    // MARK: - Host state persistence (DAW project save/restore)
    //
    // The session bytes ride inside the *base class's* ClassInfo dict rather
    // than replacing it. The AUAudioUnit v2 bridge treats `fullState` as the
    // v2 `kAudioUnitProperty_ClassInfo` value: it must carry the standard
    // ClassInfo keys (`<type>`/`<subtype>`/`<manufacturer>`, `version`, the
    // parameter tree, …) that `super.fullState` produces. Returning a custom
    // dict in their place makes auval reject the synthesized ClassInfo —
    // `kAudioUnitErr_InvalidPropertyValue` (-10851) for `["session": Data]`,
    // or "Class Data does not have required field:`<type>`" for an empty dict.
    // So: start from `super.fullState` and add `session` on top; on set,
    // pull `session` out for the engine and forward the rest to super.
    //
    // `get` reads a lock-free COW snapshot via `bridge.serialize()`
    // (worker-free — safe even before `engine_start`); `set` submits a
    // `LoadSession` command the state worker applies. `fullStateForDocument`
    // is the document-save variant (used when the host saves the project
    // file); it routes through the same envelope so both paths stay in sync.

    override var fullState: [String: Any]? {
        get {
            guard var state = super.fullState else { return nil }
            if let data = bridge?.serialize() { state[AUState.sessionKey] = data }
            return state
        }
        set {
            if let data = newValue.flatMap(AUState.unpack) { bridge?.load(data) }
            super.fullState = newValue
        }
    }

    override var fullStateForDocument: [String: Any]? {
        get { fullState }
        set { fullState = newValue }
    }

    /// Stereo (2-ch) dummy output. The AU emits MIDI, not audio — the audio
    /// buffer is left unwritten (returning `noErr` is silence to the host).
    override var channelCapabilities: [NSNumber] { [2, 2] }

    /// Declares 1 MIDI output cable so the host populates `midiOutputEventBlock`.
    override var midiOutputNames: [String] { ["StepForge Out"] }

    // MARK: - Render resource lifecycle (Hard Rule 5)

    override func allocateRenderResources() throws {
        try super.allocateRenderResources()
        // One per-instance render-state (single-owner, RT-thread-only via engine_render).
        if renderState == nil { renderState = engine_render_state_new() }

        // Capture the host-provided blocks once, here, into the cached closure.
        // AUAudioUnit.h: "an audio unit implementation accessing this property
        // should cache it in realtime-safe storage before beginning to render."
        // Reading them at render time would re-acquire the (copy) property on each
        // call; capturing once stabilizes the reference and keeps RT clean.
        _internalRenderBlock = makeRenderBlock(
            musicalContext: self.musicalContextBlock,
            transportState: self.transportStateBlock,
            midiOutput: self.midiOutputEventBlock)
    }

    override func deallocateRenderResources() {
        if let rs = renderState {
            engine_render_state_free(rs)
            renderState = nil
        }
        // Release the cached closure (and its strong captures: host blocks, bus,
        // midiIn/midiOut pointers). No further render calls happen after this.
        _internalRenderBlock = nil
        super.deallocateRenderResources()
    }

    // MARK: - internalRenderBlock (cached; built in allocateRenderResources)

    override var internalRenderBlock: AUInternalRenderBlock {
        // Pre-/post-allocation: return a no-op silence block. After allocation,
        // returns the cached closure built in allocateRenderResources.
        return _internalRenderBlock ?? { _, _, _, _, _, _, _ in noErr }
    }

    /// Build the RT render closure. Captures only: the host blocks, the output
    /// bus (for its sample rate), and the engine/render-state + fixed MIDI
    /// buffers. No `self` capture → no retain cycle; the closure is owned by
    /// `self` via `_internalRenderBlock` and released in `deallocateRenderResources`.
    private func makeRenderBlock(
        musicalContext mcBlock: AUHostMusicalContextBlock?,
        transportState tsBlock: AUHostTransportStateBlock?,
        midiOutput moBlock: AUMIDIOutputEventBlock?
    ) -> AUInternalRenderBlock {
        // Snapshot the bus + buffers at allocation time (they do not change after
        // allocateRenderResources; on a host format change the AU is reallocated).
        let bus = _outputBusses[0]
        let engine = self.engine
        let rs = self.renderState
        let midiIn = self.midiIn
        let midiOut = self.midiOut
        let inCap = UInt(MIDIMarshaler.inCapacity)
        let outCap = StepForgeAudioUnit.midiOutCap
        // Cache the bus sample rate at capture time (the format is fixed at
        // allocateRenderResources; a host format change reallocates the whole AU).
        let sampleRate = bus.format.sampleRate

        return { actionFlags, timestamp, frameCount, outputBusNumber,
                       outputData, realtimeEventListHead, pullInputBlock in
            // (0) Guard engine + render-state. These are captured once at
            // allocateRenderResources, so the guard only fires if allocation
            // produced nil (it never does — allocateRenderResources creates `rs`).
            // It does NOT catch teardown: after deallocateRenderResources nils
            // self.renderState, this closure's captured `rs` is still non-nil.
            // Teardown safety relies on the host contract ("no render after
            // deallocateRenderResources") plus _internalRenderBlock being nil'd
            // there. kAudioUnitErr_Uninitialized yields silence, not a host retry.
            guard let engine, let rs else {
                return kAudioUnitErr_Uninitialized
            }

            // (1) Transport: read host musical + transport context blocks.
            //     AUHostMusicalContextBlock (6 params): tempo, ts-num, ts-den, beat,
            //       sampleOffsetToNextBeat, currentMeasureDownbeatPosition.
            //     AUHostTransportStateBlock (4 params): flags, samplePos, cycleStart, cycleEnd.
            //     AUHostTransportStateFlags: .changed/.moving/.recording/.cycling
            //       (no `.playing` — isPlaying == .moving).
            var tempo: Double = 120.0
            var beat: Double = 0.0
            var downBeat: Double = 0.0
            _ = mcBlock?(&tempo, nil, nil, &beat, nil, &downBeat)
            var flags: AUHostTransportStateFlags = []
            _ = tsBlock?(&flags, nil, nil, nil)
            let isPlaying = flags.contains(.moving)

            var transport = HostTransportBuilder.make(
                sampleRate: sampleRate,
                frameCount: frameCount,
                tempo: tempo,
                beat: beat,
                currentDownBeat: downBeat,
                isPlaying: isPlaying)

            // (2) Walk the AURenderEvent linked list DIRECTLY into the fixed midiIn
            //     buffer (drop-tail at inCap). NO [RawMIDI] array, no allocation
            //     (Hard Rule 1). Only classic .MIDI events (AURenderEventMIDI = 8):
            //     the union variant is AUMIDIEvent with data: (UInt8,UInt8,UInt8)
            //     and length 1–3. UMP/MIDIEventList input is intentionally ignored.
            var inCount: UInt = 0
            let blockStart = Int64(timestamp.pointee.mSampleTime)
            var ev: UnsafePointer<AURenderEvent>? = realtimeEventListHead
            while let cur = ev {
                let head = cur.pointee.head
                if head.eventType == .MIDI, inCount < inCap {
                    let midi = cur.pointee.MIDI
                    let rel = head.eventSampleTime - blockStart
                    // Clamp the sample offset to [0, frameCount] (defense-in-depth:
                    // a stray future event can't overrun the current block).
                    let offset = UInt32(Swift.min(Swift.max(0, rel), Int64(frameCount)))
                    midiIn[Int(inCount)] = MidiEvent(
                        sample_offset: offset,
                        status: midi.data.0,
                        data1: midi.data.1,
                        data2: midi.data.2)
                    inCount &+= 1
                }
                // Advance to the next event (drop-tail: once inCount == inCap,
                // remaining events are skipped — bounded, never blocks).
                ev = head.next.map { UnsafePointer($0) }
            }

            // (3) Drive the engine one block on the host RT thread. RT-safe: the
            //     only FFI on this path. midiOut receives MidiEvents with sample
            //     offsets in [0, frameCount); outCount reports how many.
            var outCount: UInt = 0
            _ = engine_render(engine, rs, &transport,
                              midiIn, inCount,
                              midiOut, outCap, &outCount)
            // Defense-in-depth: the engine is trusted to honor `outCap`, but clamp
            // so the emit loop below can't overrun midiOut if it ever didn't.
            if outCount > outCap { outCount = outCap }

            // (4) Forward emitted MIDI to the host via the classic AUMIDIOutputEventBlock.
            //     3 bytes are built on the STACK (a tuple, rebound to UInt8) — never a
            //     [UInt8] array. eventSampleTime = block-start-sample + sample_offset.
            if outCount > 0, let outBlock = moBlock {
                let baseSample = AUEventSampleTime(timestamp.pointee.mSampleTime)
                for i in 0..<Int(outCount) {
                    let e = midiOut[i]
                    var bytes: (UInt8, UInt8, UInt8) = (e.status, e.data1, e.data2)
                    let sampleTime = baseSample &+ AUEventSampleTime(e.sample_offset)
                    withUnsafePointer(to: &bytes) { bp in
                        bp.withMemoryRebound(to: UInt8.self, capacity: 3) { bytePtr in
                            _ = outBlock(sampleTime, 0, 3, bytePtr)
                        }
                    }
                }
            }

            // (5) Zero-fill the dummy audio bus. The AU emits MIDI, not audio, so
            //     the output is silence — but write it explicitly (one memset per
            //     buffer, RT-safe) so a host that taps the MIDI-FX audio out never
            //     emits stale buffer content. memset + buffer-list iteration are
            //     alloc/lock-free (Hard Rule 1); engine_render is the only FFI here.
            let buffers = UnsafeMutableAudioBufferListPointer(outputData)
            for buf in buffers {
                if let data = buf.mData {
                    _ = memset(data, 0, Int(buf.mDataByteSize))
                }
            }
            return noErr
        }
    }
}
#endif
