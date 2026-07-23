import SwiftUI

/// Composes the step-cell gestures (ui-ux-spec §2.2) and routes them to command
/// closures. **Never mutates the mirror** — the closures submit `Command`s and the
/// engine's echoed `EngineEvent` updates state (the mock bridge echoes optimistically).
///
/// Gesture model:
/// - single tap on empty → place at `Mid` (instant); sets a placement timestamp
/// - single tap on filled → no-op (prevents accidental erase while programming)
/// - double tap on filled → delete, **guarded** ~150 ms after a placement so a
///   quick second tap on a just-placed step does not delete it (SwiftUI gives the
///   higher-count tap priority, so we additionally gate to be safe)
/// - long press (~450 ms) → request ratchet popover
/// - vertical drag on filled → `Accent` (up) / `Low` (down)
struct StepGestureModifier: ViewModifier {
    let isActive: Bool
    let onPlaceMid: () -> Void
    let onAccent: () -> Void
    let onLow: () -> Void
    let onDelete: () -> Void
    let onRatchetRequest: () -> Void

    @State private var lastPlacementAt: Date = .distantPast
    // Must exceed SwiftUI's double-tap recognition window (~0.3-0.35 s) so a
    // normal-speed second tap on a just-placed step is not read as a delete.
    private let deleteGuard: TimeInterval = 0.4

    func body(content: Content) -> some View {
        content
            // long-press wins for a held touch → ratchet popover
            .onLongPressGesture(minimumDuration: 0.45, maximumDistance: 12) {
                onRatchetRequest()
            } onPressingChanged: { _ in }
            // double-tap → delete (declared before count:1 so SwiftUI prefers it)
            .onTapGesture(count: 2) {
                guard isActive else { return }
                if Date().timeIntervalSince(lastPlacementAt) >= deleteGuard { onDelete() }
            }
            // single-tap → place (empty) / no-op (filled) + placement timestamp
            .onTapGesture(count: 1) {
                if !isActive {
                    onPlaceMid()
                    lastPlacementAt = Date()
                }
            }
            // vertical drag → velocity zone
            .gesture(
                DragGesture(minimumDistance: 8)
                    .onEnded { value in
                        guard isActive else { return }
                        if value.translation.height < -8 {
                            onAccent(); Haptics.zoneCross()
                        } else if value.translation.height > 8 {
                            onLow(); Haptics.zoneCross()
                        }
                    }
            )
    }
}

extension View {
    /// Attach the full step-gesture suite.
    func stepGestures(
        isActive: Bool,
        onPlaceMid: @escaping () -> Void,
        onAccent: @escaping () -> Void,
        onLow: @escaping () -> Void,
        onDelete: @escaping () -> Void,
        onRatchetRequest: @escaping () -> Void
    ) -> some View {
        modifier(StepGestureModifier(
            isActive: isActive,
            onPlaceMid: onPlaceMid, onAccent: onAccent, onLow: onLow,
            onDelete: onDelete, onRatchetRequest: onRatchetRequest))
    }
}
