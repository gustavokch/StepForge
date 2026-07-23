import SwiftUI

/// Pinch-to-zoom as an 8↔16 shortcut (ui-ux-spec §2.1). Fires once per decisive
/// gesture (a minimum ± excursion) to avoid jitter, then re-arms on release.
struct PinchZoomModifier: ViewModifier {
    let onZoomIn: () -> Void    // pinch out → fewer columns (8)
    let onZoomOut: () -> Void   // pinch in  → more columns (16)
    @State private var fired = false

    func body(content: Content) -> some View {
        content.gesture(
            MagnifyGesture()
                .onChanged { value in
                    guard !fired else { return }
                    if value.magnification > 1.4 { fired = true; onZoomIn() }
                    else if value.magnification < 0.7 { fired = true; onZoomOut() }
                }
                .onEnded { _ in fired = false }
        )
    }
}

extension View {
    func pinchZoom(onZoomIn: @escaping () -> Void, onZoomOut: @escaping () -> Void) -> some View {
        modifier(PinchZoomModifier(onZoomIn: onZoomIn, onZoomOut: onZoomOut))
    }
}
