use sequencer_engine::host::HostTransport;

/// Map host transport primitives to the engine's `HostTransport`. Both sides use
/// quarter-notes for beat position, so the mapping is direct (spec §Transport).
pub fn map_transport(
    tempo: Option<f64>,
    playing: bool,
    pos_beats: Option<f64>,
    bar_start_pos_beats: Option<f64>,
    sample_rate: f32,
    block_samples: u32,
) -> HostTransport {
    HostTransport {
        tempo_bpm: tempo.unwrap_or(120.0),
        sample_rate: sample_rate as f64,
        block_samples,
        block_start_beat: pos_beats.unwrap_or(0.0),
        bar_start_beat: bar_start_pos_beats.unwrap_or(0.0),
        is_playing: playing,
        beats_per_bar: 4.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_fields_fall_back() {
        let t = map_transport(None, false, None, None, 48000.0, 256);
        assert_eq!(t.tempo_bpm, 120.0);
        assert_eq!(t.block_start_beat, 0.0);
        assert_eq!(t.bar_start_beat, 0.0);
        assert!(!t.is_playing);
        assert_eq!(t.sample_rate, 48000.0);
        assert_eq!(t.block_samples, 256);
        assert_eq!(t.beats_per_bar, 4.0); // engine assumes 4/4 today
    }

    #[test]
    fn some_fields_map_directly() {
        let t = map_transport(Some(140.0), true, Some(17.5), Some(16.0), 44100.0, 512);
        assert_eq!(t.tempo_bpm, 140.0);
        assert!(t.is_playing);
        assert_eq!(t.block_start_beat, 17.5);
        assert_eq!(t.bar_start_beat, 16.0);
        assert_eq!(t.sample_rate, 44100.0);
        assert_eq!(t.block_samples, 512);
    }
}
