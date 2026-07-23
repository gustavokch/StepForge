#if os(iOS)
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
#elseif os(macOS)
import AppKit

/// Haptic feedback for macOS.
enum Haptics {
    static func prepare() {}

    /// Fires when a drag crosses a velocity-zone boundary.
    static func zoneCross() {
        NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .default)
    }

    /// Fires on step deletion.
    static func delete() {
        NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .default)
    }

    /// Fires on a destructive confirm (remove track, trash, etc.).
    static func confirm() {
        NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .default)
    }
}
#else
enum Haptics {
    static func prepare() {}
    static func zoneCross() {}
    static func delete() {}
    static func confirm() {}
}
#endif
