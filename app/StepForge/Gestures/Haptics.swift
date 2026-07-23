import UIKit

/// Haptic feedback (ui-ux-spec §2.3 velocity-zone crossing, §3.3 delete). Prepared
/// and fired on the MainActor from gesture handlers.
enum Haptics {
    private static let impact = UIImpactFeedbackGenerator(style: .light)
    private static let notice = UINotificationFeedbackGenerator()

    static func prepare() {
        impact.prepare(); notice.prepare()
    }

    /// Fires when a drag crosses a velocity-zone boundary.
    static func zoneCross() { impact.impactOccurred() }
    /// Fires on step deletion.
    static func delete() { impact.impactOccurred(intensity: 0.7) }
    /// Fires on a destructive confirm (remove track, trash, etc.).
    static func confirm() { notice.notificationOccurred(.success) }
}
