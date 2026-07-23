import SwiftUI

/// Feel controls (ui-ux-spec §1.1 Row 2): global swing, humanize popover, and the
/// quantize-grain cycle. (Pattern management is reached via the Performance mode
/// toggle, so the Patterns button is omitted from this row.)
///
/// Swing and humanize have **no echo event** in the 18-event contract, so their
/// controls are driven by UI-local `@State` (seeded from the mirror, re-synced on
/// `FullSnapshot` for swing) and submitted to the engine on change — otherwise a
/// binding straight to `mirror.globalSwingPct` would snap back to 0 every drag tick.
/// The quantize grain is also UI-local default state (not persisted in `Session`).
struct FeelBar: View {
    @EnvironmentObject private var bridge: EngineBridge
    @State private var showHumanize = false
    @State private var humanizeTiming: Float = 0
    @State private var humanizeVelocity: Float = 0
    @State private var swingValue: Float = 0
    @State private var quantizeGrain: QuantizeGrain = .nextBeat

    var body: some View {
        HStack(spacing: Theme.Spacing.sm) {
            swingControl
            humanizeControl
            Spacer(minLength: Theme.Spacing.xs)
            quantizeControl
        }
        .padding(.horizontal, Theme.Spacing.sm)
        .padding(.vertical, 6)
        .panelStyle(Theme.Surface.highest)
        .onAppear { seedFromMirror(); }
        .onChange(of: bridge.mirror.globalSwingPct) { _, value in swingValue = value; }
        .popover(isPresented: $showHumanize) {
            HumanizeEditor(timing: $humanizeTiming, velocity: $humanizeVelocity) {
                bridge.submit(.setHumanize(timing: humanizeTiming, velocity: humanizeVelocity))
            }
            .frame(idealWidth: 280, idealHeight: 150)
            .presentationCompactAdaptation(.popover)
        }
    }

    private func seedFromMirror() {
        swingValue = bridge.mirror.globalSwingPct
        humanizeTiming = bridge.mirror.humanizeTiming
        humanizeVelocity = bridge.mirror.humanizeVelocity
    }

    private var swingControl: some View {
        HStack(spacing: 6) {
            Text("GROOVE").font(Typography.controlLabel).foregroundStyle(Theme.textMuted)
            Slider(value: Binding(get: { swingValue },
                                  set: { swingValue = $0; bridge.submit(.setGlobalSwing(pct: $0)) }),
                   in: 0...0.5)
                .frame(width: 84)
            Text("\(Int((swingValue * 100).rounded()))%")
                .font(Typography.monoValue)
                .foregroundStyle(Theme.textSecondary)
                .frame(width: 34, alignment: .trailing)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 2)
        .raisedStyle()
    }

    private var humanizeControl: some View {
        Button { showHumanize.toggle() } label: {
            HStack(spacing: 6) {
                Image(systemName: "waveform").font(.caption)
                Text("NUANCE").font(Typography.controlLabel)
            }
            .foregroundStyle(humanizeActive ? Theme.primary : Theme.textSecondary)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .raisedStyle()
    }

    private var humanizeActive: Bool { humanizeTiming > 0 || humanizeVelocity > 0 }

    private var quantizeControl: some View {
        Button { cycleQuantize() } label: {
            HStack(spacing: 4) {
                Text("GRID").font(Typography.controlLabel).foregroundStyle(Theme.textMuted)
                Text(quantizeGrain.shortLabel).font(Typography.monoValue).foregroundStyle(Theme.textPrimary)
            }
            .padding(.horizontal, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .raisedStyle()
    }

    private func cycleQuantize() {
        let all: [QuantizeGrain] = [.nextStep, .nextBeat, .nextBar, .endOfPattern]
        let i = all.firstIndex(of: quantizeGrain) ?? 0
        quantizeGrain = all[(i + 1) % all.count]
        bridge.submit(.setQuantizeGrain(grain: quantizeGrain))
    }
}

/// Humanize popover (ui-ux-spec §1.1 Row 2): Timing Jitter + Velocity
/// Randomization sliders. Commits via `onApply`.
private struct HumanizeEditor: View {
    @Binding var timing: Float
    @Binding var velocity: Float
    let onApply: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Theme.Spacing.md) {
            SectionLabel("Humanize")
            slider(label: "Timing", value: $timing)
            slider(label: "Velocity", value: $velocity)
            HStack {
                Spacer()
                Button("Apply") { onApply() }
                    .buttonStyle(.borderedProminent)
                    .tint(Theme.primary)
            }
        }
        .padding()
        .background(Theme.Surface.low)
    }

    private func slider(label: String, value: Binding<Float>) -> some View {
        HStack {
            Text(label).font(Typography.controlLabel).foregroundStyle(Theme.textSecondary).frame(width: 64, alignment: .leading)
            Slider(value: value, in: 0...1).tint(Theme.primary)
            Text("\(Int((value.wrappedValue * 100).rounded()))%")
                .font(Typography.monoValue).foregroundStyle(Theme.textSecondary).frame(width: 38, alignment: .trailing)
        }
    }
}
