import SwiftUI

struct PatternOptionsSheet: View {
    let patternIdx: Int
    let currentFollowAction: FollowAction
    let loopsRemaining: Int?
    let onSaveFollowAction: (FollowAction) -> Void
    @EnvironmentObject private var bridge: EngineBridge
    @Environment(\.dismiss) private var dismiss

    @State private var afterLoops: Int
    @State private var actionType: FollowActionType

    init(patternIdx: Int, currentFollowAction: FollowAction, loopsRemaining: Int?, onSaveFollowAction: @escaping (FollowAction) -> Void) {
        self.patternIdx = patternIdx
        self.currentFollowAction = currentFollowAction
        self.loopsRemaining = loopsRemaining
        self.onSaveFollowAction = onSaveFollowAction
        _afterLoops = State(initialValue: Int(currentFollowAction.afterLoops))
        _actionType = State(initialValue: currentFollowAction.action)
    }

    var body: some View {
        NavigationStack {
            Form {
                // Whole-pattern clipboard (CLAP parity — issue #33). Engine-side
                // clipboard; Copy emits no event, Paste/Cut/Clear publish a
                // FullSnapshot only on mutation. Dismiss after each so a Paste
                // (which overwrites follow_action) never leaves a stale draft.
                Section("Pattern") {
                    HStack(spacing: 6) {
                        patternAction("Cut",   "scissors")         { bridge.submit(.cutPattern(index:   patternIdx)); dismiss() }
                        patternAction("Copy",  "doc.on.doc")       { bridge.submit(.copyPattern(index:  patternIdx)); dismiss() }
                        patternAction("Paste", "doc.on.clipboard") { bridge.submit(.pastePattern(index: patternIdx)); dismiss() }
                        patternAction("Clear", "trash")            { bridge.submit(.clearPattern(index: patternIdx)); Haptics.confirm(); dismiss() }
                    }
                }

                Section("Follow Action") {
                    if let loopsLeft = loopsRemaining, actionType != .none {
                        Text("\(loopsLeft) loops remaining")
                            .font(Typography.badge)
                            .foregroundStyle(Theme.primaryDim)
                    }
                    Stepper("After Loops: \(afterLoops)", value: $afterLoops, in: 1...16)
                    
                    Picker("Action Type", selection: $actionType) {
                        Text("None").tag(FollowActionType.none)
                        Text("Play Next").tag(FollowActionType.playNext)
                        Text("Play Previous").tag(FollowActionType.playPrevious)
                        Text("Stop").tag(FollowActionType.stop)
                        Text("Play Random").tag(FollowActionType.playRandom)
                    }
                }
            }
            .navigationTitle("Pattern \(patternIdx + 1) Options")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        onSaveFollowAction(FollowAction(afterLoops: UInt32(afterLoops), action: actionType))
                        dismiss()
                    }
                }
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }

    // Cloned from `ActionDrawer.action` — icon + sectionTag label, raised tile.
    private func patternAction(_ label: String, _ icon: String, _ perform: @escaping () -> Void) -> some View {
        Button(action: perform) {
            VStack(spacing: 4) {
                Image(systemName: icon).font(.title3)
                Text(label).font(Typography.sectionTag)
            }
            .foregroundStyle(Theme.textPrimary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .raisedStyle()
        }
        .buttonStyle(.plain)
    }
}
