import Foundation

/// Pure mapper: AU musical/transport context values → the `HostTransport` C struct
/// consumed by `engine_render` (Phase 0). No handle, no side effects → unit-testable.
///
/// `beats_per_bar` defaults to 4.0: the Phase-0 accumulator assumes 4/4 and does not
/// yet read this field (host.rs). It is plumbed through so a later phase can honor
/// non-4/4 time signatures without an ABI change.
enum HostTransportBuilder {
    static func make(
        sampleRate: Double,
        frameCount: UInt32,
        tempo: Double,
        beat: Double,
        currentDownBeat: Double,
        beatsPerBar: Double = 4.0,
        isPlaying: Bool
    ) -> HostTransport {
        HostTransport(
            tempo_bpm: tempo,
            sample_rate: sampleRate,
            block_samples: frameCount,
            block_start_beat: beat,
            bar_start_beat: currentDownBeat,
            is_playing: isPlaying,
            beats_per_bar: beatsPerBar
        )
    }
}
