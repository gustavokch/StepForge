import SwiftUI

/// Small rectangular/capsule badge — speed ratio, follow action, sync source
/// (DESIGN.md §Components: Chips/Badges). `accent` renders in primary orange.
struct Chip: View {
    let text: String
    var accent: Bool = false

    var body: some View {
        Text(text)
            .chipStyle(foreground: accent ? Theme.onPrimary : Theme.textPrimary,
                       background: accent ? Theme.primary : Theme.Surface.high,
                       border: accent ? Theme.primary : Theme.borderWeak)
    }
}
