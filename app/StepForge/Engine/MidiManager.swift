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

    init() {
        setupClient()
        refreshDestinations()
    }

    private func setupClient() {
        var c: MIDIClientRef = 0
        let status = MIDIClientCreate("StepForgeSwift" as CFString, nil, nil, &c)
        if status == noErr {
            client = c
        }
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
