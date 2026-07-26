//! Host-driven render types. The plugin host calls `engine_render` once per
//! audio block on its RT thread; these POD structs cross that C ABI. Plain
//! scalar structs (not data-carrying enums) — `#[repr(C)]` is allowed.
//!
//! `Engine::render_host` (in `engine.rs`, same impl block as `process_one`)
//! consumes a `HostRenderState` owned by the caller, keeping `Engine` lock-free
//! and `Send+Sync` without `UnsafeCell`/`unsafe` in core.

use crate::engine::RtState;

/// Host transport snapshot for one audio block. Filled by the plugin wrapper
/// from AU `musicalContextBlock`/`transportStateBlock` or nih_plug `Transport`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HostTransport {
    /// Tempo in beats-per-minute.
    pub tempo_bpm: f64,
    /// Audio sample rate (Hz).
    pub sample_rate: f64,
    /// Number of samples in this block.
    pub block_samples: u32,
    /// Absolute host beat position at the first sample of this block.
    pub block_start_beat: f64,
    /// Beat position of the current bar's downbeat (≤ `block_start_beat`).
    pub bar_start_beat: f64,
    /// Host transport playing state.
    pub is_playing: bool,
    /// Beats per bar (time-signature numerator, e.g. 4.0 for 4/4). Reserved for
    /// non-4/4 support in a later phase; the Phase 0 accumulator assumes 4/4
    /// (four 16ths per beat) and does not yet read this field. Included now so
    /// the committed header needs no ABI-breaking field addition later.
    pub beats_per_bar: f64,
}

/// One 3-byte MIDI message with a sample offset within the current block. Used
/// for both host → engine input and engine → host output.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiEvent {
    /// Sample offset within the block, in `[0, block_samples)`.
    pub sample_offset: u32,
    /// Full status byte including channel (e.g. `0x90 | ch`).
    pub status: u8,
    /// MIDI data byte 1 (note / controller).
    pub data1: u8,
    /// MIDI data byte 2 (velocity / value).
    pub data2: u8,
}

impl MidiEvent {
    pub const fn zero() -> Self {
        Self {
            sample_offset: 0,
            status: 0,
            data1: 0,
            data2: 0,
        }
    }
}

/// Maximum simultaneous deferred MIDI events. A note's gate (default 50 ms)
/// often spans several audio blocks, and a high-swing note-on can land past the
/// block that fired its boundary. Both are held here until due. Bounded → RT-safe.
pub const PENDING_OFF_DEPTH: usize = 64;

#[derive(Clone, Copy)]
struct PendingMidiEvent {
    abs_sample: u64,
    status: u8,
    data1: u8,
    data2: u8,
    active: bool,
}

/// Fixed-size, single-threaded (host-RT-owner) scheduled-MIDI queue. Holds both
/// deferred note-offs (gate spans past the block) and deferred note-ons (swing
/// pushes the note past the block that fired its boundary). No locks, no
/// allocation — `render_host` is the only accessor.
pub struct PendingMidiQueue {
    slots: [PendingMidiEvent; PENDING_OFF_DEPTH],
}

impl PendingMidiQueue {
    pub fn new() -> Self {
        Self {
            slots: [PendingMidiEvent {
                abs_sample: 0,
                status: 0,
                data1: 0,
                data2: 0,
                active: false,
            }; PENDING_OFF_DEPTH],
        }
    }

    /// Schedule a 3-byte MIDI message at absolute sample time. Finds an inactive
    /// slot; if none, evicts the slot with the largest `abs_sample` (drop-furthest).
    pub fn schedule(&mut self, abs_sample: u64, status: u8, data1: u8, data2: u8) {
        let mut victim = 0usize;
        let mut victim_abs = u64::MIN;
        for (i, s) in self.slots.iter_mut().enumerate() {
            if !s.active {
                s.active = true;
                s.abs_sample = abs_sample;
                s.status = status;
                s.data1 = data1;
                s.data2 = data2;
                return;
            }
            if s.abs_sample > victim_abs {
                victim_abs = s.abs_sample;
                victim = i;
            }
        }
        // Full — evict the furthest-future slot if this one is sooner.
        if abs_sample < self.slots[victim].abs_sample {
            self.slots[victim] = PendingMidiEvent {
                abs_sample,
                status,
                data1,
                data2,
                active: true,
            };
        }
    }

    /// Emit events whose `abs_sample` falls in `[block_start_abs, block_end_abs)`
    /// as `MidiEvent`s (offset relative to `block_start_abs`) and deactivate them.
    pub fn drain_due(
        &mut self,
        block_start_abs: u64,
        block_end_abs: u64,
        mut out: impl FnMut(MidiEvent),
    ) {
        for s in self.slots.iter_mut() {
            if s.active && s.abs_sample >= block_start_abs && s.abs_sample < block_end_abs {
                out(MidiEvent {
                    sample_offset: (s.abs_sample - block_start_abs) as u32,
                    status: s.status,
                    data1: s.data1,
                    data2: s.data2,
                });
                s.active = false;
            }
        }
    }

    /// Drop every scheduled event. Called on transport stop after CC 123
    /// all-notes-off — the host killed the notes, so individual events still
    /// queued (note-offs or deferred note-ons) are stale and must not fire.
    pub fn clear(&mut self) {
        for s in self.slots.iter_mut() {
            s.active = false;
        }
    }
}

impl Default for PendingMidiQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-instance host-render state, owned by the plugin wrapper (one per engine
/// handle) and passed to `Engine::render_host` each block. Persistent across
/// blocks; single-owner → no synchronization needed.
pub struct HostRenderState {
    /// Reused RT tick state (per-track step indices, RNG, bar position).
    pub rt: RtState,
    pub pending: PendingMidiQueue,
    /// Beat position of the next 16th-step boundary to fire.
    pub next_step_beat: f64,
    /// Absolute sample time at the start of the next block to render.
    pub sample_time: u64,
    /// Last seen `block_start_beat` (seek/discontinuity detection).
    pub last_block_start_beat: f64,
    pub was_playing: bool,
    pub initialized: bool,
}

impl HostRenderState {
    pub fn new() -> Self {
        Self {
            rt: RtState::new(1), // reseeded by `begin_play` on play-start
            pending: PendingMidiQueue::new(),
            next_step_beat: 0.0,
            sample_time: 0,
            last_block_start_beat: f64::NAN,
            was_playing: false,
            initialized: false,
        }
    }
}

impl Default for HostRenderState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_queue_schedules_and_drains_due_only() {
        let mut q = PendingMidiQueue::new();
        // Two events scheduled in the future: a note-off and a deferred note-on.
        q.schedule(1_000, 0x80, 36, 0);
        q.schedule(2_000, 0x9A, 38, 100);
        let mut emitted = Vec::new();
        q.drain_due(0, 1_500, |ev| emitted.push(ev));
        // Only the abs=1_000 one is due within [0,1_500).
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].sample_offset, 1_000);
        assert_eq!(emitted[0].data1, 36);
        // Draining again in a later block picks up the survivor — with its data2.
        emitted.clear();
        q.drain_due(1_500, 3_000, |ev| emitted.push(ev));
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].data1, 38);
        assert_eq!(
            emitted[0].data2, 100,
            "a deferred note-on keeps its velocity"
        );
        // Fully drained.
        emitted.clear();
        q.drain_due(3_000, 4_000, |_| panic!("no more"));
        assert!(emitted.is_empty());
    }

    #[test]
    fn clear_drops_all_scheduled_events() {
        // Transport stop emits CC 123 all-notes-off, then clears the queue so no
        // stale events (note-off OR deferred note-on) fire for notes the host
        // already killed.
        let mut q = PendingMidiQueue::new();
        q.schedule(1_000, 0x80, 36, 0);
        q.schedule(2_000, 0x9A, 38, 100);
        q.clear();
        let mut emitted = Vec::new();
        q.drain_due(0, 10_000, |ev| emitted.push(ev));
        assert!(emitted.is_empty(), "clear drops all scheduled events");
    }

    #[test]
    fn host_render_state_default_is_stopped_and_uninitialized() {
        let rs = HostRenderState::new();
        assert!(!rs.was_playing);
        assert!(!rs.initialized);
        assert_eq!(rs.sample_time, 0);
    }
}
