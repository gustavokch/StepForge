import SwiftUI

/// Editing ⇄ Performance mode toggle (ui-ux-spec §1). Lives in `Features/` so
/// all three targets — iOS app, macOS app, and the AU extension — see it. The
/// AU's `PluginEditorView` mirrors the standalone `RootView` mode switch
/// without depending on `Root/` (which isn't in the AU sources).
enum AppMode { case editing, performance }
