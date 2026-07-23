import SwiftUI

/// Uppercase technical section tag (DESIGN.md §Typography: "Use uppercase
/// sparingly for section headers to evoke a hardware-chassis vibe").
struct SectionLabel: View {
    let text: String

    /// Label-less convenience init (`SectionLabel("Foo")`).
    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text.uppercased())
            .font(Typography.sectionTag)
            .foregroundStyle(Theme.textMuted)
            .tracking(0.8)
    }
}
