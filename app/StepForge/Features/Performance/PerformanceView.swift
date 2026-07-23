import SwiftUI

/// Mode Selector: Jam vs Arrangement
enum PerformanceMode: String, CaseIterable, Identifiable {
    case jam = "Jam"
    case arrangement = "Arrangement"
    var id: String { rawValue }
}

/// Helper struct to present pattern options sheet by item
private struct PatternIdxItem: Identifiable {
    let id: Int
}

/// Performance mode (ui-ux-spec §1.2) — live pattern jamming.
/// Renders 3x3 pattern grid, queueing, retrigger gestures, quantize grain selector,
/// follow action editor, and track activity LEDs.
struct PerformanceView: View {
    @EnvironmentObject private var bridge: EngineBridge

    @State private var mode: PerformanceMode = .jam
    @State private var quantizeGrain: QuantizeGrain = .nextBeat
    @State private var selectedPatternIdxForOptions: Int? = nil

    var body: some View {
        VStack(spacing: Theme.Spacing.md) {
            topBar
            modeSelector
            patternGrid
            Divider()
                .background(Theme.borderWeak)
            trackActivitySection
        }
        .padding(Theme.Spacing.md)
        .background(Theme.Surface.lowest)
        .sheet(item: Binding(
            get: { selectedPatternIdxForOptions.map { PatternIdxItem(id: $0) } },
            set: { selectedPatternIdxForOptions = $0?.id }
        )) { item in
            if let pattern = bridge.mirror.patterns.indices.contains(item.id) ? bridge.mirror.patterns[item.id] : nil {
                PatternOptionsSheet(
                    patternIdx: item.id,
                    currentFollowAction: pattern.followAction,
                    onSaveFollowAction: { action in
                        bridge.submit(.setFollowAction(patternIdx: item.id, action: action))
                    }
                )
            }
        }
    }

    // MARK: - Top Bar
    private var topBar: some View {
        HStack(spacing: Theme.Spacing.md) {
            // Enlarged Play/Stop
            Button {
                bridge.submit(bridge.mirror.playing ? .stop : .play)
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: bridge.mirror.playing ? "stop.fill" : "play.fill")
                        .font(.title2)
                    Text(bridge.mirror.playing ? "STOP" : "PLAY")
                        .font(Typography.controlLabel)
                        .bold()
                }
                .foregroundStyle(bridge.mirror.playing ? Theme.primary : Theme.textPrimary)
                .padding(.horizontal, Theme.Spacing.md)
                .frame(height: 48)
            }
            .buttonStyle(.plain)
            .raisedStyle(radius: Theme.Radius.md)

            // Enlarged Patterns Button with Loop Progress Ring
            HStack(spacing: Theme.Spacing.sm) {
                ZStack {
                    Circle()
                        .stroke(Theme.borderWeak, lineWidth: 3)
                    if bridge.mirror.playing {
                        Circle()
                            .trim(from: 0, to: loopProgress)
                            .stroke(Theme.primary, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                            .rotationEffect(.degrees(-90))
                    }
                    Image(systemName: "square.grid.3x3.fill")
                        .font(.system(size: 14))
                        .foregroundStyle(bridge.mirror.playing ? Theme.primary : Theme.textSecondary)
                }
                .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text("PATTERNS")
                        .font(Typography.sectionTag)
                        .foregroundStyle(Theme.textMuted)
                    Text("Pattern \(bridge.mirror.activePatternIndex + 1)")
                        .font(Typography.trackName)
                        .foregroundStyle(Theme.textPrimary)
                }
            }
            .padding(.horizontal, Theme.Spacing.md)
            .frame(height: 48)
            .panelStyle(Theme.Surface.high, radius: Theme.Radius.md)

            Spacer()

            // Enlarged Quantize Grain Selector
            HStack(spacing: 4) {
                ForEach(QuantizeGrain.allCases, id: \.self) { grain in
                    Button {
                        quantizeGrain = grain
                        bridge.submit(.setQuantizeGrain(grain: grain))
                    } label: {
                        Text(grain.shortLabel)
                            .font(Typography.badge)
                            .foregroundStyle(quantizeGrain == grain ? Theme.onPrimary : Theme.textSecondary)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 8)
                            .background(quantizeGrain == grain ? Theme.primary : Theme.Surface.low)
                            .cornerRadius(Theme.Radius.sm)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(4)
            .panelStyle(Theme.Surface.default, radius: Theme.Radius.md)
        }
    }

    // MARK: - Mode Selector
    private var modeSelector: some View {
        HStack {
            Text("MODE")
                .font(Typography.sectionTag)
                .foregroundStyle(Theme.textMuted)

            Picker("Mode", selection: $mode) {
                ForEach(PerformanceMode.allCases) { m in
                    Text(m.rawValue).tag(m)
                }
            }
            .pickerStyle(.segmented)
            .onChange(of: mode) { _, newMode in
                switch newMode {
                case .jam:
                    quantizeGrain = .nextBeat
                    bridge.submit(.setQuantizeGrain(grain: .nextBeat))
                case .arrangement:
                    quantizeGrain = .endOfPattern
                    bridge.submit(.setQuantizeGrain(grain: .endOfPattern))
                }
            }
        }
        .padding(.horizontal, Theme.Spacing.sm)
    }

    // MARK: - 3x3 Pattern Grid
    private var patternGrid: some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: Theme.Spacing.sm), count: 3), spacing: Theme.Spacing.sm) {
            ForEach(0..<9, id: \.self) { idx in
                patternCell(for: idx)
            }
        }
    }

    private func patternCell(for idx: Int) -> some View {
        let pattern = bridge.mirror.patterns.indices.contains(idx) ? bridge.mirror.patterns[idx] : nil
        let isFilled = (pattern != nil)
        let isActive = (bridge.mirror.activePatternIndex == idx)
        let isQueued = (bridge.mirror.queuedPatternIndex == idx)

        return ZStack(alignment: .topTrailing) {
            VStack(spacing: Theme.Spacing.xs) {
                Text("Pattern \(idx + 1)")
                    .font(Typography.trackName)
                    .foregroundStyle(cellTextColor(isFilled: isFilled, isActive: isActive, isQueued: isQueued))

                if isQueued {
                    Text("QUEUED")
                        .font(Typography.badge)
                        .foregroundStyle(Theme.primaryDim)
                } else if isActive {
                    Text("PLAYING")
                        .font(Typography.badge)
                        .foregroundStyle(Theme.primary)
                } else if isFilled {
                    Text("FILLED")
                        .font(Typography.badge)
                        .foregroundStyle(Theme.textSecondary)
                } else {
                    Text("EMPTY")
                        .font(Typography.badge)
                        .foregroundStyle(Theme.textMuted)
                }

                if let fa = pattern?.followAction, fa.action != .none {
                    Text("\(fa.action.shortLabel) (x\(fa.afterLoops))")
                        .chipStyle(foreground: Theme.primaryDim, background: Theme.Surface.low, border: Theme.borderStrong)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 90)
            .background(cellBackground(isFilled: isFilled, isActive: isActive, isQueued: isQueued))
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.lg)
                    .stroke(cellBorderColor(isFilled: isFilled, isActive: isActive, isQueued: isQueued),
                            lineWidth: isActive ? 2 : 1)
                    .allowsHitTesting(false)
            )
            .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.lg))
            .overlay(alignment: .bottom) {
                if isActive && bridge.mirror.playing {
                    GeometryReader { geo in
                        Rectangle()
                            .fill(Theme.primary)
                            .frame(width: geo.size.width * loopProgress, height: 3)
                    }
                    .frame(height: 3)
                }
            }
            .allowsHitTesting(false)

            if isFilled {
                Button {
                    selectedPatternIdxForOptions = idx
                } label: {
                    Image(systemName: "gearshape.fill")
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.textMuted)
                        .padding(8)
                }
                .buttonStyle(.plain)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            if isActive {
                bridge.submit(.retriggerPattern(quantize: .nextBeat))
            } else if isFilled {
                if isQueued {
                    bridge.submit(.cancelQueuedPattern)
                } else {
                    bridge.submit(.queuePattern(index: idx, quantize: quantizeGrain))
                }
            }
        }
        .onLongPressGesture {
            if isActive {
                bridge.submit(.retriggerPattern(quantize: .nextStep))
            } else if isFilled {
                selectedPatternIdxForOptions = idx
            }
        }
    }

    private func cellBackground(isFilled: Bool, isActive: Bool, isQueued: Bool) -> Color {
        if isActive { return Theme.Surface.high }
        if isQueued { return Theme.Surface.high }
        if isFilled { return Theme.Surface.default }
        return Theme.Surface.low
    }

    private func cellBorderColor(isFilled: Bool, isActive: Bool, isQueued: Bool) -> Color {
        if isActive { return Theme.primary }
        if isQueued { return Theme.primaryDim }
        if isFilled { return Theme.borderStrong }
        return Theme.borderWeak
    }

    private func cellTextColor(isFilled: Bool, isActive: Bool, isQueued: Bool) -> Color {
        if isActive { return Theme.primary }
        if isQueued { return Theme.primaryDim }
        if isFilled { return Theme.textPrimary }
        return Theme.textMuted
    }

    // MARK: - Loop Progress Helper
    private var loopProgress: Double {
        guard bridge.mirror.playing else { return 0.0 }
        let currentStep = bridge.mirror.playheadStep ?? 0
        let totalSteps = bridge.mirror.tracks.first?.length ?? 16
        return Double(currentStep + 1) / Double(max(1, totalSteps))
    }

    // MARK: - Simplified Track Rows & Activity LEDs
    private var trackActivitySection: some View {
        VStack(alignment: .leading, spacing: Theme.Spacing.xs) {
            Text("TRACK ACTIVITY & MUTE")
                .font(Typography.sectionTag)
                .foregroundStyle(Theme.textMuted)
                .padding(.horizontal, Theme.Spacing.xs)

            ScrollView {
                VStack(spacing: Theme.Spacing.xs) {
                    ForEach(Array(bridge.mirror.tracks.enumerated()), id: \.element.id) { tIdx, track in
                        trackActivityRow(tIdx: tIdx, track: track)
                    }
                }
            }
            .frame(maxHeight: 180)
        }
    }

    private func trackActivityRow(tIdx: Int, track: Track) -> some View {
        let playhead = bridge.mirror.playheads[tIdx] ?? 0
        let isHit = bridge.mirror.playing &&
                    track.steps.indices.contains(playhead) &&
                    track.steps[playhead].active

        return HStack(spacing: Theme.Spacing.md) {
            // Activity LED Indicator
            Circle()
                .fill(isHit ? Theme.primary : Theme.Surface.lowest)
                .overlay(Circle().stroke(isHit ? Theme.primary : Theme.borderWeak, lineWidth: 1).allowsHitTesting(false))
                .shadow(color: isHit ? Theme.primary : .clear, radius: 4)
                .frame(width: 14, height: 14)

            // Track Name
            VStack(alignment: .leading, spacing: 2) {
                Text(DrumNames.name(for: track.midiNote))
                    .font(Typography.trackName)
                    .foregroundStyle(track.muted ? Theme.textMuted : Theme.textPrimary)
                Text("Track \(tIdx + 1)")
                    .font(Typography.badge)
                    .foregroundStyle(Theme.textMuted)
            }

            Spacer()

            // Mute Toggle Button
            Button {
                bridge.submit(.setTrackMuted(trackIdx: tIdx, muted: !track.muted))
            } label: {
                Image(systemName: track.muted ? "speaker.slash.fill" : "speaker.wave.2.fill")
                    .font(.system(size: 14))
                    .foregroundStyle(track.muted ? Theme.onPrimary : Theme.textSecondary)
                    .padding(8)
                    .background(track.muted ? Theme.primary : Theme.Surface.high)
                    .clipShape(Circle())
                    .overlay(Circle().stroke(track.muted ? Theme.primary : Theme.borderWeak, lineWidth: 1).allowsHitTesting(false))
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, Theme.Spacing.md)
        .padding(.vertical, Theme.Spacing.xs)
        .background(Theme.Surface.low)
        .cornerRadius(Theme.Radius.sm)
    }
}
