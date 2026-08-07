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
    @State private var showClearConfirm = false

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
                // Whole-pattern clipboard (issue #33) + undo (#34). Engine-side
                // clipboard; Copy emits no event, Paste/Cut/Clear publish a
                // FullSnapshot only on mutation. The sheet STAYS OPEN after
                // Cut/Paste/Clear/Undo so the user can tap Undo immediately and
                // see the restored state; Copy still dismisses (non-mutating,
                // no undo). Clear confirms before mutating; it is now undoable.
                Section("Pattern") {
                    HStack(spacing: 6) {
                        TileButton("Cut",   "scissors")         { bridge.submit(.cutPattern(index:   patternIdx)) }
                        TileButton("Copy",  "doc.on.doc")       { bridge.submit(.copyPattern(index:  patternIdx)); dismiss() }
                        TileButton("Paste", "doc.on.clipboard") { bridge.submit(.pastePattern(index: patternIdx)) }
                        TileButton("Clear", "trash")            { showClearConfirm = true }
                        TileButton("Undo",  "arrow.uturn.backward") { bridge.submit(.undoPattern(index: patternIdx)) }
                    }
                    .confirmationDialog(
                        "Clear pattern \(patternIdx + 1)?",
                        isPresented: $showClearConfirm,
                        titleVisibility: .visible
                    ) {
                        Button("Clear", role: .destructive) {
                            bridge.submit(.clearPattern(index: patternIdx))
                            Haptics.confirm()
                        }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("Removes all programmed steps. The slot stays. Undo is available from this sheet.")
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
            .onChange(of: currentFollowAction) { newAction in
                // #34: the sheet stays open after Cut/Paste/Clear/Undo, so the
                // engine's follow_action can change underneath the @State draft
                // (which SwiftUI seeds once in init). Re-sync the Stepper/Picker
                // whenever the prop updates — the egui sheet does this per-frame
                // via #42 — otherwise a later Save would clobber the
                // pasted/restored value with the stale draft.
                afterLoops = Int(newAction.afterLoops)
                actionType = newAction.action
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
}
