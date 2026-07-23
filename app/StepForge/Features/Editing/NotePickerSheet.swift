import SwiftUI

/// Hybrid Note Picker: GM Drum grid + 2-octave mini piano keyboard.
struct NotePickerSheet: View {
    let trackIdx: Int
    let currentNote: UInt8
    let onSelect: (UInt8) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var mode: PickerMode = .gmDrums

    enum PickerMode: String, CaseIterable, Identifiable {
        case gmDrums = "GM Drums"
        case piano = "Piano Roll"
        var id: String { rawValue }
    }

    private static let gmSoundNames: [(note: UInt8, name: String)] = [
        (35, "Acoustic Bass Drum"), (36, "Bass Drum 1 (Kick)"),
        (37, "Side Stick"), (38, "Acoustic Snare"),
        (39, "Hand Clap"), (40, "Electric Snare"),
        (41, "Low Floor Tom"), (42, "Closed Hi-Hat"),
        (43, "High Floor Tom"), (44, "Pedal Hi-Hat"),
        (45, "Low Tom"), (46, "Open Hi-Hat"),
        (47, "Low-Mid Tom"), (48, "Hi-Mid Tom"),
        (49, "Crash Cymbal 1"), (50, "High Tom")
    ]

    var body: some View {
        NavigationStack {
            VStack(spacing: 16) {
                Picker("Mode", selection: $mode) {
                    ForEach(PickerMode.allCases) { m in
                        Text(m.rawValue).tag(m)
                    }
                }
                .pickerStyle(.segmented)
                .padding(.horizontal)

                if mode == .gmDrums {
                    ScrollView {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 140))], spacing: 10) {
                            ForEach(Self.gmSoundNames, id: \.note) { item in
                                Button {
                                    onSelect(item.note)
                                    dismiss()
                                } label: {
                                    VStack(alignment: .leading, spacing: 4) {
                                        Text(item.name)
                                            .font(Typography.trackName)
                                            .foregroundColor(item.note == currentNote ? Theme.primary : Theme.textPrimary)
                                        Text("MIDI \(item.note)")
                                            .font(Typography.badge)
                                            .foregroundColor(Theme.textSecondary)
                                    }
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding(12)
                                    .background(item.note == currentNote ? Theme.Surface.high : Theme.Surface.default)
                                    .cornerRadius(Theme.Radius.md)
                                    .overlay(
                                        RoundedRectangle(cornerRadius: Theme.Radius.md)
                                            .stroke(item.note == currentNote ? Theme.primary : Color.clear, lineWidth: 1)
                                    )
                                }
                            }
                        }
                        .padding(.horizontal)
                    }
                } else {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 2) {
                            ForEach(36...60, id: \.self) { note in
                                let isBlack = [1, 3, 6, 8, 10].contains(note % 12)
                                Button {
                                    onSelect(UInt8(note))
                                    dismiss()
                                } label: {
                                    VStack {
                                        Spacer()
                                        Text("\(note)")
                                            .font(Typography.badge)
                                            .foregroundColor(isBlack ? .white : .black)
                                            .padding(.bottom, 8)
                                    }
                                    .frame(width: isBlack ? 28 : 36, height: isBlack ? 120 : 180)
                                    .background(note == Int(currentNote) ? Theme.primary : (isBlack ? Color.black : Color.white))
                                    .cornerRadius(Theme.Radius.sm)
                                }
                            }
                        }
                        .padding()
                    }
                }
            }
            .navigationTitle("Select Track Note")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { dismiss() }
                }
            }
            .background(Theme.Surface.lowest.ignoresSafeArea())
        }
    }
}
