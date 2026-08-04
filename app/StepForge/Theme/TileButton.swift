import SwiftUI

/// Raised action tile: SF Symbol over an uppercase `sectionTag` label, on a
/// `.raisedStyle()` surface with a `.plain` button style. The shared renderer
/// for the per-track `ActionDrawer` actions and the whole-pattern clipboard
/// tiles in `PatternOptionsSheet` (DESIGN.md §Shapes — single tile spec, no
/// per-call-site drift).
struct TileButton: View {
    let label: String
    let icon: String
    let action: () -> Void

    /// Positional init keeps the call sites terse (`TileButton("Cut", "scissors") { … }`),
    /// matching the prior private builders in `ActionDrawer` / `PatternOptionsSheet`.
    init(_ label: String, _ icon: String, action: @escaping () -> Void) {
        self.label = label
        self.icon = icon
        self.action = action
    }

    var body: some View {
        Button(action: action) {
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
