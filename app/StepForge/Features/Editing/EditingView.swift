import SwiftUI

/// Editing-mode root (ui-ux-spec §1.1 — default mode): transport, feel controls,
/// track management, and the step grid. Reads state from `bridge.mirror`; gestures
/// submit `Command`s (the UI never mutates state directly — Hard Rule 2).
struct EditingView: View {
    @EnvironmentObject private var bridge: EngineBridge
    @Environment(\.horizontalSizeClass) private var hSize
    @Environment(\.verticalSizeClass) private var vSize

    /// 8 or 16 columns shown at once (ui-ux-spec §2.1). Cell size is derived so
    /// exactly `visibleSteps` fit the width; the full 16 scroll horizontally.
    @State private var visibleSteps: Int = 16
    @State private var didSizeSteps = false

    /// Long-press target for the ratchet popover.
    @State private var ratchetTarget: TrackStepRef? = nil
    /// Track selected for the action drawer.
    @State private var drawerTarget: DrawerTarget? = nil

    var body: some View {
        VStack(spacing: Theme.Spacing.sm) {
            TransportBar(visibleSteps: $visibleSteps)
            FeelBar()
            TrackManagementBar()
            TrackList(visibleSteps: visibleSteps,
                      ratchetTarget: $ratchetTarget,
                      drawerTarget: $drawerTarget)
                .pinchZoom(onZoomIn: { withAnimation { visibleSteps = 8 } },
                           onZoomOut: { withAnimation { visibleSteps = 16 } })
        }
        .padding(.horizontal, Theme.Spacing.sm)
        .padding(.top, Theme.Spacing.xs)
        .padding(.bottom, Theme.Spacing.sm)
        .background(Theme.Surface.lowest)
        .onAppear {
            Haptics.prepare()
            guard !didSizeSteps else { return }
            didSizeSteps = true
            visibleSteps = GridMetrics.defaultVisibleSteps(hSize: hSize, vSize: vSize)
        }
        .onChange(of: hSize) { _, _ in
            visibleSteps = GridMetrics.defaultVisibleSteps(hSize: hSize, vSize: vSize)
        }
        .onChange(of: vSize) { _, _ in
            visibleSteps = GridMetrics.defaultVisibleSteps(hSize: hSize, vSize: vSize)
        }
        .confirmationDialog(
            ratchetTarget.map { "Ratchet · \($0.step + 1)" } ?? "",
            isPresented: ratchetPresented,
            titleVisibility: .visible
        ) {
            ForEach([Ratchet.off, .x2, .x3, .x4], id: \.self) { r in
                Button(r.label) { applyRatchet(r) }
            }
            Button("Cancel", role: .cancel) {}
        }
        .sheet(item: $drawerTarget) { target in
            ActionDrawer(trackIdx: target.track)
                .presentationDetents([.height(132)])
        }
    }

    private var ratchetPresented: Binding<Bool> {
        Binding(get: { ratchetTarget != nil }, set: { presented in if !presented { ratchetTarget = nil } })
    }

    private func applyRatchet(_ ratchet: Ratchet) {
        if let t = ratchetTarget {
            bridge.submit(.setRatchet(trackIdx: t.track, stepIdx: t.step, ratchet: ratchet))
        }
        ratchetTarget = nil
    }
}

/// Identifiable (track, step) pair for the ratchet popover.
struct TrackStepRef: Identifiable, Equatable {
    let track: Int
    let step: Int
    var id: String { "\(track)-\(step)" }
}

/// Identifiable track index for the action-drawer sheet.
struct DrawerTarget: Identifiable {
    let track: Int
    var id: Int { track }
}
