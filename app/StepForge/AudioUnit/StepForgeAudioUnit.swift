#if os(macOS)
import AudioToolbox
import CoreAudioKit
import AVFoundation

/// Phase 1 AUv3 audio unit (`'aumi'` MIDI-FX, `SFor`/`DrmS`). Owns the
/// host-driven engine handle + render-state + lifecycle (Hard Rule 5).
/// `internalRenderBlock` runs on the host RT thread (Hard Rule 1).
///
/// This skeleton renders silence and is loadable by `auval`; Task 5 wires
/// `engine_render` (transport + MIDI I/O). The dummy stereo output bus exists
/// because some hosts reject a bus-less `'aumi'`; the engine emits MIDI, not
/// audio, so the audio buffer is left untouched (returning `noErr` is treated
/// as silence by the host).
final class StepForgeAudioUnit: AUAudioUnit {

    private var outputBus: AUAudioUnitBus?
    private var _outputBusses: AUAudioUnitBusArray!

    override init(componentDescription: AudioComponentDescription,
                 options: AudioComponentInstantiationOptions = []) throws {
        try super.init(componentDescription: componentDescription, options: options)
        // Build the bus array at init so `outputBusses` is non-empty before
        // `allocateRenderResources` runs (auval probes the bus list early;
        // returning an empty array from the base class yields
        // `kAudioUnitErr_InvalidScope` = -10875 during initialization).
        let format = AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 2)!
        outputBus = try AUAudioUnitBus(format: format)
        _outputBusses = AUAudioUnitBusArray(audioUnit: self, busType: .output, busses: [outputBus!])
    }

    override var outputBusses: AUAudioUnitBusArray {
        return _outputBusses
    }

    override var channelCapabilities: [NSNumber] { [2, 2] }   // 2-ch stereo out

    override var internalRenderBlock: AUInternalRenderBlock {
        // Skeleton: render silence to the dummy audio bus. RT-safe (no alloc/lock).
        // Returns `noErr` without writing — the host treats an unwritten buffer as
        // silence, which is the correct behavior for a MIDI-FX that emits no audio.
        // MIDI rides the MIDI-output event list (wired in Task 5), not this buffer.
        return { actionFlags, timestamp, frameCount, outputBusNumber,
                        outputData, realtimeEventList, pullInputBlock in
            return noErr
        }
    }
}
#endif
