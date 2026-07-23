import SwiftUI

/// Pinned track header (ui-ux-spec §3.1): mute toggle, drum/note name, speed-ratio
/// chip, length chip, note picker trigger, and "…" → opens the action drawer. Reads
/// the `Track`; mute toggles submit `SetTrackMuted`. Note name tap opens `NotePickerSheet`.
struct TrackHeader: View {
    @EnvironmentObject private var bridge: EngineBridge
    let track: Track
    let trackIdx: Int
    let onOpenActions: () -> Void

    @State private var showNotePicker = false

    var body: some View {
        HStack(spacing: 6) {
            muteButton
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Button {
                        showNotePicker = true
                    } label: {
                        Text(DrumNames.name(for: track.midiNote))
                            .font(Typography.trackName)
                            .foregroundStyle(track.muted ? Theme.textMuted : Theme.textPrimary)
                            .lineLimit(1)
                    }
                    .buttonStyle(.plain)

                    speedMenu
                    lengthMenu
                }
                Button {
                    showNotePicker = true
                } label: {
                    Text("NOTE \(track.midiNote)")
                        .font(Typography.badge)
                        .foregroundStyle(Theme.textMuted)
                }
                .buttonStyle(.plain)
            }
            Spacer(minLength: 2)
            Button(action: onOpenActions) {
                Image(systemName: "ellipsis")
                    .font(.body)
                    .foregroundStyle(Theme.textSecondary)
                    .frame(width: 24, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.Surface.low)
        .sheet(isPresented: $showNotePicker) {
            NotePickerSheet(
                trackIdx: trackIdx,
                currentNote: track.midiNote
            ) { note in
                bridge.submit(.setTrackNote(trackIdx: trackIdx, midiNote: note))
            }
        }
    }

    private var speedMenu: some View {
        Menu {
            ForEach([0.5, 1.0, 2.0, 3.0], id: \.self) { ratio in
                Button {
                    bridge.submit(.setTrackSpeedRatio(trackIdx: trackIdx, ratio: Float(ratio)))
                } label: {
                    HStack {
                        Text(SpeedRatio.label(Float(ratio)))
                        if abs(track.speedRatio - Float(ratio)) < 0.01 {
                            Image(systemName: "checkmark")
                        }
                    }
                }
            }
        } label: {
            Chip(text: SpeedRatio.label(track.speedRatio), accent: abs(track.speedRatio - 1.0) > 0.001)
        }
    }

    private var lengthMenu: some View {
        Menu {
            ForEach(1...16, id: \.self) { l in
                Button {
                    bridge.submit(.setTrackLength(trackIdx: trackIdx, length: l))
                } label: {
                    HStack {
                        Text("\(l) steps")
                        if track.length == l {
                            Image(systemName: "checkmark")
                        }
                    }
                }
            }
        } label: {
            Chip(text: "\(track.length)s", accent: track.length != 16)
        }
    }

    private var muteButton: some View {
        Button {
            bridge.submit(.setTrackMuted(trackIdx: trackIdx, muted: !track.muted))
        } label: {
            Text("M")
                .font(Typography.badge)
                .foregroundStyle(track.muted ? Theme.onPrimary : Theme.textSecondary)
                .frame(width: 20, height: 20)
                .background(track.muted ? AnyShapeStyle(Theme.primary) : AnyShapeStyle(Theme.Surface.high))
                .overlay(Capsule().stroke(track.muted ? Theme.primary : Theme.borderWeak, lineWidth: Theme.borderWidth))
                .clipShape(Capsule())
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
    }
}

/// General-MIDI drum name lookup (falls back to the note number).
enum DrumNames {
    private static let map: [UInt8: String] = [
        35: "Kick", 36: "Kick", 37: "Side Stick", 38: "Snare", 39: "Clap", 40: "Snare",
        41: "Low Floor Tom", 42: "Closed Hat", 43: "High Floor Tom", 44: "Pedal Hat",
        45: "Low Tom", 46: "Open Hat", 47: "Low-Mid Tom", 48: "Hi-Mid Tom", 49: "Crash",
        50: "High Tom", 51: "Ride", 52: "Chinese", 53: "Ride Bell", 54: "Tambourine",
        55: "Splash", 56: "Cowbell", 57: "Crash 2", 58: "Vibraslap", 59: "Ride 2",
        60: "Hi Bongo", 61: "Low Bongo", 62: "Mute Hi Conga", 63: "Open Hi Conga",
        64: "Low Conga", 65: "High Timbale", 66: "Low Timbale", 67: "High Agogo",
        68: "Low Agogo", 69: "Cabasa", 70: "Maracas", 71: "Short Whistle", 72: "Long Whistle",
        73: "Short Guiro", 74: "Long Guiro", 75: "Claves", 76: "Hi Wood Block", 77: "Low Wood Block",
        78: "Mute Cuica", 79: "Open Cuica", 80: "Mute Triangle", 81: "Open Triangle"
    ]
    static func name(for note: UInt8) -> String {
        map[note] ?? "Note \(note)"
    }
}

/// Speed-ratio chip text (spec form: `½×/1×/2×/3×`, not the mockup's "D1").
enum SpeedRatio {
    static func label(_ ratio: Float) -> String {
        switch ratio {
        case 0.5: "½×"
        case 1.0: "1×"
        case 2.0: "2×"
        case 3.0: "3×"
        default: String(format: "%.1f×", ratio)
        }
    }
}
