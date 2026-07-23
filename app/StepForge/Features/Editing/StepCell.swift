import SwiftUI

/// A single step cell. **Value-in** (`trackIdx`, `stepIdx`, `step`, `isPlaying`,
/// `isWithinLength`) so SwiftUI's structural diff re-renders only the cell whose
/// inputs changed — a `StepChanged` flips one cell, a playhead advance flips ≤2.
/// The gesture suite submits `Command`s and **never mutates state**; the engine's
/// echoed `StepChanged` updates the cell through the mirror (Hard Rule 2).
struct StepCell: View {
    @EnvironmentObject private var bridge: EngineBridge
    let trackIdx: Int
    let stepIdx: Int
    let step: Step
    let isPlaying: Bool
    let isWithinLength: Bool
    let onRatchetRequest: () -> Void

    var body: some View {
        RoundedRectangle(cornerRadius: Theme.Radius.sm, style: .continuous)
            .fill(fill)
            .overlay(border.allowsHitTesting(false))
            .overlay(playheadMark.allowsHitTesting(false))
            .overlay(ratchetMark.allowsHitTesting(false))
            .opacity(isWithinLength ? 1 : 0.22)   // dim steps beyond `length` (non-destructive window)
            .contentShape(Rectangle())
            .stepGestures(
                isActive: step.active && isWithinLength,
                onPlaceMid: { bridge.submit(.setStep(trackIdx: trackIdx, stepIdx: stepIdx, zone: .mid)) },
                onAccent: { bridge.submit(.setStep(trackIdx: trackIdx, stepIdx: stepIdx, zone: .accent)) },
                onLow: { bridge.submit(.setStep(trackIdx: trackIdx, stepIdx: stepIdx, zone: .low)) },
                onDelete: { bridge.submit(.deleteStep(trackIdx: trackIdx, stepIdx: stepIdx)); Haptics.delete() },
                onRatchetRequest: onRatchetRequest
            )
            .accessibilityLabel(accessibilityLabel)
    }

    private var fill: Color {
        step.active ? Theme.velocity(step.velocityZone) : Theme.Surface.low
    }

    private var border: some View {
        let activeAccent = step.active && step.velocityZone == .accent
        return RoundedRectangle(cornerRadius: Theme.Radius.sm, style: .continuous)
            .stroke(activeAccent ? Theme.primary : Theme.borderWeak,
                    lineWidth: activeAccent ? 1.5 : Theme.borderWidth)
    }

    @ViewBuilder private var playheadMark: some View {
        if isPlaying {
            VStack(spacing: 0) {
                Rectangle().fill(Theme.textPrimary).frame(height: 2)
                Spacer()
            }
        }
    }

    @ViewBuilder private var ratchetMark: some View {
        if step.ratchet != .off {
            VStack {
                Spacer()
                HStack(spacing: 1) {
                    ForEach(0..<step.ratchet.repeats, id: \.self) { _ in
                        Capsule().fill(Theme.textPrimary.opacity(0.85)).frame(width: 2, height: 6)
                    }
                }
                .padding(2)
            }
        }
    }

    private var accessibilityLabel: String {
        let state = step.active ? step.velocityZone.label : "empty"
        let ratchet = step.ratchet != .off ? ", ratchet \(step.ratchet.label)" : ""
        return "Step \(stepIdx + 1), \(state)\(ratchet)"
    }
}

/// Value equality over the *visual* inputs only — the gesture closures are
/// recreated each render and aren't comparable, so they're excluded. This lets
/// `.equatable()` skip body re-evaluation for cells whose visible state is
/// unchanged (a `StepChanged` flips exactly one cell).
extension StepCell: Equatable {
    static func == (lhs: StepCell, rhs: StepCell) -> Bool {
        lhs.trackIdx == rhs.trackIdx
            && lhs.stepIdx == rhs.stepIdx
            && lhs.step == rhs.step
            && lhs.isPlaying == rhs.isPlaying
            && lhs.isWithinLength == rhs.isWithinLength
    }
}

private extension VelocityZone {
    var label: String {
        switch self { case .low: "low"; case .mid: "mid"; case .accent: "accent"; }
    }
}
