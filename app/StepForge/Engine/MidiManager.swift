import Foundation
import CoreMIDI
import Combine

struct MidiDestination: Identifiable, Hashable {
    let id: UInt32
    let name: String
}

final class MidiManager: ObservableObject {
    @Published private(set) var destinations: [MidiDestination] = []
    @Published var selectedIDs: Set<UInt32> = []

    private var client: MIDIClientRef = 0
    private var inputPort: MIDIPortRef = 0
    private weak var bridge: EngineBridge?

    private var lastClockTime: UInt64 = 0
    private var clockDeltas: [UInt64] = []

    init() {
        setupClient()
        refreshDestinations()
    }

    func bind(to bridge: EngineBridge) {
        self.bridge = bridge
    }

    private func setupClient() {
        var c: MIDIClientRef = 0
        let status = MIDIClientCreate("StepForgeSwift" as CFString, nil, nil, &c)
        if status == noErr {
            client = c
            
            var inPort: MIDIPortRef = 0
            let portStatus = MIDIInputPortCreateWithBlock(c, "StepForge Input" as CFString, &inPort) { [weak self] packetList, _ in
                self?.handleMidiInput(packetList)
            }
            if portStatus == noErr {
                inputPort = inPort
                connectToAllSources()
            }
        }
    }

    func connectToAllSources() {
        guard inputPort != 0 else { return }
        let count = MIDIGetNumberOfSources()
        for i in 0..<count {
            let source = MIDIGetSource(i)
            MIDIPortConnectSource(inputPort, source, nil)
        }
    }

    private func handleMidiInput(_ packetList: UnsafePointer<MIDIPacketList>) {
        guard let bridge = bridge else { return }
        
        let packets = packetList.pointee
        var packet = packets.packet
        
        for _ in 0..<packets.numPackets {
            let data = withUnsafeBytes(of: packet.data) { Array($0) }
            for i in 0..<Int(packet.length) {
                let byte = data[i]
                if byte == 0xF8 { // Timing Clock
                    if bridge.mirror.syncSource == .midiClock {
                        bridge.submit(.midiClockTick)
                        estimateBPM(hostTime: mach_absolute_time())
                    }
                } else if byte == 0xFA { // Start
                    if bridge.mirror.syncSource == .midiClock {
                        bridge.submit(.play)
                    }
                } else if byte == 0xFC { // Stop
                    if bridge.mirror.syncSource == .midiClock {
                        bridge.submit(.stop)
                    }
                }
            }
            packet = MIDIPacketNext(&packet).pointee
        }
    }

    private func estimateBPM(hostTime: UInt64) {
        if lastClockTime > 0 {
            let delta = hostTime - lastClockTime
            clockDeltas.append(delta)
            if clockDeltas.count > 24 {
                clockDeltas.removeFirst()
            }
            
            if clockDeltas.count == 24 {
                let avgDelta = clockDeltas.reduce(0, +) / UInt64(clockDeltas.count)
                var timebaseInfo = mach_timebase_info_data_t()
                mach_timebase_info(&timebaseInfo)
                let nanosPerTick = Double(timebaseInfo.numer) / Double(timebaseInfo.denom)
                let avgNanos = Double(avgDelta) * nanosPerTick
                
                let quarterNoteNanos = avgNanos * 24.0
                let bpm = 60.0 / (quarterNoteNanos / 1_000_000_000.0)
                
                if abs((bridge?.mirror.bpm ?? 120.0) - bpm) > 0.5 {
                    bridge?.submit(.setBpm(bpm: bpm))
                }
            }
        }
        lastClockTime = hostTime
    }

    func refreshDestinations() {
        var list: [MidiDestination] = []
        let count = MIDIGetNumberOfDestinations()
        for i in 0..<count {
            let endpoint = MIDIGetDestination(i)
            var param: Unmanaged<CFString>?
            let err = MIDIObjectGetStringProperty(endpoint, kMIDIPropertyDisplayName, &param)
            let name: String
            if err == noErr, let cfStr = param?.takeRetainedValue() {
                name = cfStr as String
            } else {
                name = "MIDI Output \(i + 1)"
            }
            list.append(MidiDestination(id: UInt32(endpoint), name: name))
        }
        destinations = list
    }

    func toggleDestination(_ id: UInt32, on bridge: EngineBridge) {
        if selectedIDs.contains(id) {
            selectedIDs.remove(id)
        } else {
            selectedIDs.insert(id)
        }
        bridge.submit(.setMidiDestinations(endpoints: Array(selectedIDs)))
    }

    deinit {
        if client != 0 {
            MIDIClientDispose(client)
        }
    }
}
