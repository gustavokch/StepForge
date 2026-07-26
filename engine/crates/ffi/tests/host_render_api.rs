//! C-ABI round-trip for the host-driven entries (Phase 0). Exercises the full
//! `engine_new_host_driven → engine_start → engine_render → engine_stop → free`
//! lifecycle in-process, mirroring how a plugin wrapper will call it.

use sequencer_engine_ffi::{
    engine_free, engine_new, engine_new_host_driven, engine_render, engine_render_state_free,
    engine_render_state_new, engine_start, engine_stop, EngineResult, HostTransport, MidiEvent,
};

#[test]
fn host_driven_lifecycle_round_trip() {
    unsafe {
        let eng = engine_new_host_driven();
        assert!(!eng.is_null());
        let rs = engine_render_state_new();
        assert!(!rs.is_null());
        // host-driven start spawns only the state worker.
        assert!(matches!(engine_start(eng), EngineResult::Ok));

        let t = HostTransport {
            tempo_bpm: 120.0,
            sample_rate: 48_000.0,
            block_samples: 6_000,
            block_start_beat: 0.0,
            bar_start_beat: 0.0,
            is_playing: true,
            beats_per_bar: 4.0,
        };
        let mut out = [MidiEvent::zero(); 64];
        let mut count = 0usize;
        let r = engine_render(
            eng,
            rs,
            &t,
            [].as_ptr(),
            0,
            out.as_mut_ptr(),
            out.len(),
            &mut count,
        );
        assert!(matches!(r, EngineResult::Ok), "render returned {r:?}");
        // Default session has no active steps → no notes; still must not crash and
        // count must be ≤ capacity.
        assert!(count <= out.len());

        assert!(matches!(engine_stop(eng), EngineResult::Ok));
        engine_render_state_free(rs);
        engine_free(eng);
    }
}

#[test]
fn null_handle_or_state_is_rejected() {
    use sequencer_engine_ffi::HostTransport;
    unsafe {
        let rs = engine_render_state_new();
        let t = HostTransport {
            tempo_bpm: 120.0,
            sample_rate: 48_000.0,
            block_samples: 256,
            block_start_beat: 0.0,
            bar_start_beat: 0.0,
            is_playing: false,
            beats_per_bar: 4.0,
        };
        let mut out = [MidiEvent::zero(); 8];
        let mut count = 0usize;
        let r = engine_render(
            std::ptr::null_mut(),
            rs,
            &t,
            [].as_ptr(),
            0,
            out.as_mut_ptr(),
            out.len(),
            &mut count,
        );
        assert!(matches!(r, EngineResult::ErrInvalidHandle));
        engine_render_state_free(rs);
        // NULL render state is a tolerated no-op for free.
        engine_render_state_free(std::ptr::null_mut());
    }
}

#[test]
fn engine_render_rejects_standalone_engine() {
    // M2 host-pairing guard: a host that mis-pairs `engine_new` (standalone)
    // with `engine_render` must get `ErrInvalidHandle` instead of silently
    // double-dispatching (self-scheduled RT thread + host RT thread both
    // driving `process_one`). `host_driven` is false from construction, so no
    // `engine_start` is needed to exercise the guard.
    unsafe {
        let eng = engine_new(); // NOT host_driven
        assert!(!eng.is_null());
        let rs = engine_render_state_new();
        assert!(!rs.is_null());

        let t = HostTransport {
            tempo_bpm: 120.0,
            sample_rate: 48_000.0,
            block_samples: 256,
            block_start_beat: 0.0,
            bar_start_beat: 0.0,
            is_playing: true,
            beats_per_bar: 4.0,
        };
        let mut out = [MidiEvent::zero(); 8];
        let mut count = 0usize;
        let r = engine_render(
            eng,
            rs,
            &t,
            [].as_ptr(),
            0,
            out.as_mut_ptr(),
            out.len(),
            &mut count,
        );
        assert!(
            matches!(r, EngineResult::ErrInvalidHandle),
            "engine_render on standalone engine must reject (got {r:?})"
        );

        engine_render_state_free(rs);
        engine_free(eng);
    }
}
