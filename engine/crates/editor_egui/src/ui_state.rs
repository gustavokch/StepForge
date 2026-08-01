//! GUI mirror of engine state — Rust port of the iOS `SessionMirror`
//! (`app/StepForge/Engine/SessionMirror.swift`). Mutated ONLY by applying
//! `EngineEvent`s + the snapshot Arc (CLAUDE.md Hard Rule 2: UI reads no
//! pointer into engine memory). GUI-thread-only (V4) ∴ heap types (HashMap /
//! HashSet / Arc) are fine — this is ⊥ the RT path.
//!
//! Dual-source model (design §Editor design): hot/large events for transient
//! liveness (playheads, queued pattern, loop count, errors) + the throttled
//! ArcSwap snapshot for ground truth (tracks, steps, lengths, mutes, notes).
//! `apply` is a faithful port of `SessionMirror.apply(_:)` — drift = UX
//! regression.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sequencer_engine::event::EngineEvent;
use sequencer_engine::models::{
    Pattern, QuantizeGrain, Session, Step, SyncSource, Track, PATTERN_SLOTS,
};

/// An engine error surfaced to the UI (`EngineEvent::Error`). Mirrors the iOS
/// `EngineErrorMirror` value type (`SessionMirror.swift:5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineError {
    pub code: i32,
    pub message: String,
}

#[derive(Default)]
pub struct UiState {
    /// Authoritative musical state. Replaced wholesale on `FullSnapshot` +
    /// throttled refresh from the ArcSwap snapshot; mutated in place on deltas.
    pub session: Option<Arc<Session>>,

    // Transient UI-only state (no wire representation beyond the events).
    pub playing: bool,
    pub queued_pattern: Option<usize>,
    pub queued_pattern_quantize: Option<QuantizeGrain>,
    /// Single active-pattern loop counter (RT resets on switch/stop).
    pub pattern_loop_count: u32,
    /// Per-track latest step index — coalesced result of `Playhead` events
    /// (one entry per track per drain batch, last-wins).
    pub playheads: HashMap<usize, usize>,
    /// Track indices with an undo snapshot available.
    pub undo_available: HashSet<usize>,
    pub last_error: Option<EngineError>,
    pub last_overflow: Option<u32>,
    pub link_peers: usize,
    pub link_enabled: bool,
}

impl UiState {
    /// Apply one decoded `EngineEvent`. Direct port of
    /// `SessionMirror.apply(_:)` (`SessionMirror.swift:82`). `Playhead` is a
    /// no-op here — it is coalesced via [`apply_playhead`] (last-wins per track
    /// per batch), mirroring the Swift split.
    pub fn apply(&mut self, ev: &EngineEvent) {
        match ev {
            EngineEvent::StepChanged {
                track_idx,
                step_idx,
                step,
            } => {
                let step = *step;
                self.mutate_step(*track_idx, *step_idx, |s| *s = step);
            }
            EngineEvent::TrackLengthChanged { track_idx, length } => {
                self.mutate_track(*track_idx, |t| t.length = *length);
            }
            EngineEvent::TrackMutedChanged { track_idx, muted } => {
                self.mutate_track(*track_idx, |t| t.muted = *muted);
            }
            EngineEvent::TrackAdded { track_idx, track } => {
                let track = track.clone();
                let idx = *track_idx;
                self.mutate_active_pattern(|p| {
                    let at = idx.min(p.tracks.len());
                    p.tracks.insert(at, track);
                });
            }
            EngineEvent::TrackRemoved { track_idx } => {
                let idx = *track_idx;
                self.mutate_active_pattern(|p| {
                    if idx < p.tracks.len() {
                        p.tracks.remove(idx);
                    }
                });
            }
            EngineEvent::PatternQueued { index, quantize } => {
                self.queued_pattern = Some(*index);
                self.queued_pattern_quantize = Some(*quantize);
            }
            EngineEvent::PatternSwitched { index } => {
                let index = *index;
                if let Some(s) = self.session.as_mut() {
                    Arc::make_mut(s).active_pattern_index = index;
                }
                self.queued_pattern = None;
                self.queued_pattern_quantize = None;
                self.pattern_loop_count = 0;
                self.playheads.clear();
                self.undo_available.clear();
            }
            EngineEvent::PatternCleared { index } => {
                let index = *index;
                let mut clear_queued = false;
                if let Some(s) = self.session.as_mut() {
                    let s = Arc::make_mut(s);
                    if index < PATTERN_SLOTS {
                        s.patterns[index] = None;
                    }
                    clear_queued = index == s.active_pattern_index;
                }
                if clear_queued {
                    self.queued_pattern = None;
                }
            }
            EngineEvent::PatternLoopCountChanged { count } => {
                self.pattern_loop_count = *count;
            }
            EngineEvent::FollowActionChanged {
                pattern_idx,
                action,
            } => {
                let pattern_idx = *pattern_idx;
                let action = action.clone();
                if let Some(s) = self.session.as_mut() {
                    let s = Arc::make_mut(s);
                    if pattern_idx < PATTERN_SLOTS {
                        if let Some(p) = s.patterns[pattern_idx].as_mut() {
                            p.follow_action = action;
                        }
                    }
                }
            }
            EngineEvent::Playhead { .. } => {} // coalesced via apply_playhead
            EngineEvent::PlayStateChanged { playing } => {
                self.playing = *playing;
                if !*playing {
                    self.pattern_loop_count = 0;
                }
            }
            EngineEvent::BpmChanged { bpm } => {
                if let Some(s) = self.session.as_mut() {
                    Arc::make_mut(s).bpm = *bpm;
                }
            }
            EngineEvent::SyncSourceChanged { source } => {
                let source = *source;
                if let Some(s) = self.session.as_mut() {
                    Arc::make_mut(s).sync_source = source;
                }
                // Mirror the engine auto-enable rule (Defect 3): Link enables,
                // otherwise disables. Keeps the mirror self-consistent even if
                // the separate LinkEnabledChanged event is dropped under
                // hot-channel overflow (drop-oldest).
                self.link_enabled = source == SyncSource::Link;
            }
            EngineEvent::UndoAvailable {
                track_idx,
                available,
            } => {
                if *available {
                    self.undo_available.insert(*track_idx);
                } else {
                    self.undo_available.remove(track_idx);
                }
            }
            EngineEvent::FullSnapshot { session } => {
                self.session = Some(Arc::new(session.clone()));
                self.queued_pattern = None;
                self.queued_pattern_quantize = None;
                self.pattern_loop_count = 0;
                self.playheads.clear();
                self.undo_available.clear();
                self.last_error = None;
                self.last_overflow = None;
            }
            EngineEvent::Serialized { .. } => {} // bridge routes Serialized → save buffer, ⊥ mirror
            EngineEvent::Error { code, message } => {
                self.last_error = Some(EngineError {
                    code: *code,
                    message: message.clone(),
                });
            }
            EngineEvent::Overflow { dropped } => {
                self.last_overflow = Some(*dropped);
            }
            EngineEvent::LinkPeersChanged { count } => {
                self.link_peers = *count;
            }
            EngineEvent::LinkEnabledChanged { enabled } => {
                self.link_enabled = *enabled;
            }
        }
    }

    /// Apply one coalesced per-track playhead (last-wins per track per drain
    /// batch). Mirrors `SessionMirror.applyPlayhead(trackIdx:stepIdx:)`.
    pub fn apply_playhead(&mut self, track_idx: usize, step_idx: usize) {
        self.playheads.insert(track_idx, step_idx);
    }

    // ---- Immutable read accessors (port of `SessionMirror.activePattern` /
    // `.tracks`). GUI widgets read these; they never mutate. Bounds-checked —
    // an out-of-range `active_pattern_index` (or no session) yields empty/None,
    // never panics (Hard Rule 3 value-layer).

    /// The active pattern (`patterns[active_pattern_index]`), if present.
    pub fn active_pattern(&self) -> Option<&Pattern> {
        let s = self.session.as_deref()?;
        let i = s.active_pattern_index;
        if i < PATTERN_SLOTS {
            s.patterns[i].as_ref()
        } else {
            None
        }
    }

    /// Tracks of the active pattern (empty slice if there is no active pattern).
    pub fn tracks(&self) -> &[Track] {
        self.active_pattern()
            .map(|p| p.tracks.as_slice())
            .unwrap_or(&[])
    }

    /// Authoritative BPM (snapshot). `120.0` before the first snapshot lands —
    /// matches `Session::default().bpm` so the TransportBar (T10c) shows a sane
    /// value pre-echo. Read-only (⊥ optimistic); edits become `Command::SetBpm`
    /// and the engine echoes `BpmChanged` back through `apply`.
    pub fn bpm(&self) -> f64 {
        self.session.as_ref().map(|s| s.bpm).unwrap_or(120.0)
    }

    /// Authoritative sync source (snapshot). `Free` before the first snapshot.
    /// Read-only in the plugin (host owns transport); the TransportBar (T10c)
    /// only labels it.
    pub fn sync_source(&self) -> SyncSource {
        self.session
            .as_ref()
            .map(|s| s.sync_source)
            .unwrap_or_default()
    }

    // ---- Feel read accessors (T10d FeelBar). Same shape as `bpm()`: snapshot
    // ground truth, a sane default before the first snapshot, read-only (⊥
    // optimistic) — edits become `Command`s and the engine echoes back via
    // `apply`. ----

    /// Authoritative global swing `[0, 0.5]` (snapshot). `0.0` before the first
    /// snapshot. The FeelBar slider (T10d) reads this; edits become
    /// `Command::SetGlobalSwing` and the engine echoes `GlobalSwingChanged`.
    pub fn swing_pct(&self) -> f32 {
        self.session
            .as_ref()
            .map(|s| s.global_swing_pct)
            .unwrap_or(0.0)
    }

    /// Authoritative humanize timing `[0, 1]` (snapshot). `0.0` before the first
    /// snapshot. Read by the FeelBar humanize popover (T10d); committed via
    /// `Command::SetHumanize`.
    pub fn humanize_timing(&self) -> f32 {
        self.session
            .as_ref()
            .map(|s| s.humanize_timing)
            .unwrap_or(0.0)
    }

    /// Authoritative humanize velocity `[0, 1]` (snapshot). `0.0` before the
    /// first snapshot. Read by the FeelBar humanize popover (T10d); committed
    /// via `Command::SetHumanize`.
    pub fn humanize_velocity(&self) -> f32 {
        self.session
            .as_ref()
            .map(|s| s.humanize_velocity)
            .unwrap_or(0.0)
    }

    /// Active pattern index (snapshot). `0` before the first snapshot. The
    /// FeelBar pattern switcher (T10d) reads this to mark the active slot.
    pub fn active_pattern_index(&self) -> usize {
        self.session
            .as_ref()
            .map(|s| s.active_pattern_index)
            .unwrap_or(0)
    }

    // Nested mutation helpers — `Arc::make_mut` gives COW mutation of the shared
    // snapshot (clone only when refcount > 1, GUI-thread alloc is fine). An
    // out-of-range index from a racy/malformed event is dropped, never panics
    // (Hard Rule 3 value-layer).

    fn mutate_active_pattern(&mut self, f: impl FnOnce(&mut Pattern)) {
        let s = match self.session.as_mut() {
            Some(s) => s,
            None => return,
        };
        let s = Arc::make_mut(s);
        let i = s.active_pattern_index;
        if i < PATTERN_SLOTS {
            if let Some(p) = s.patterns[i].as_mut() {
                f(p);
            }
        }
    }

    fn mutate_track(&mut self, track_idx: usize, f: impl FnOnce(&mut Track)) {
        self.mutate_active_pattern(|p| {
            if track_idx < p.tracks.len() {
                f(&mut p.tracks[track_idx]);
            }
        });
    }

    fn mutate_step(&mut self, track_idx: usize, step_idx: usize, f: impl FnOnce(&mut Step)) {
        self.mutate_track(track_idx, |t| {
            if step_idx < t.steps.len() {
                f(&mut t.steps[step_idx]);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sequencer_engine::models::{FollowAction, Ratchet, VelocityZone};

    fn state_with_session() -> UiState {
        UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        }
    }

    fn track(st: &UiState, idx: usize) -> &Track {
        st.session.as_ref().unwrap().patterns[0]
            .as_ref()
            .unwrap()
            .tracks
            .get(idx)
            .expect("track exists")
    }

    #[test]
    fn apply_step_changed_mutates_step() {
        let mut st = state_with_session();
        let step = Step {
            active: true,
            velocity_zone: VelocityZone::Accent,
            micro_timing_offset: 0.05,
            ratchet: Ratchet::X2,
        };
        st.apply(&EngineEvent::StepChanged {
            track_idx: 1,
            step_idx: 3,
            step,
        });
        assert_eq!(track(&st, 1).steps[3], step); // Step: Copy → local still valid
        assert!(!track(&st, 1).steps[4].active); // neighbor untouched
    }

    #[test]
    fn apply_track_fields_mutate() {
        let mut st = state_with_session();
        st.apply(&EngineEvent::TrackLengthChanged {
            track_idx: 0,
            length: 8,
        });
        st.apply(&EngineEvent::TrackMutedChanged {
            track_idx: 2,
            muted: true,
        });
        assert_eq!(track(&st, 0).length, 8);
        assert!(track(&st, 2).muted);
    }

    #[test]
    fn apply_track_add_remove() {
        let mut st = state_with_session();
        let new_track = Track::with_note(46);
        st.apply(&EngineEvent::TrackAdded {
            track_idx: 99, // 99 > len → append at min(99, 4)
            track: new_track,
        });
        assert_eq!(track(&st, 4).midi_note, 46);

        st.apply(&EngineEvent::TrackRemoved { track_idx: 0 });
        assert_eq!(track(&st, 0).midi_note, 38); // 36 removed, 38 shifts up
    }

    #[test]
    fn apply_pattern_queue_switch_clear() {
        let mut st = state_with_session();
        st.apply(&EngineEvent::PatternQueued {
            index: 2,
            quantize: QuantizeGrain::NextBeat,
        });
        assert_eq!(st.queued_pattern, Some(2));
        assert_eq!(st.queued_pattern_quantize, Some(QuantizeGrain::NextBeat));

        st.apply_playhead(1, 5);
        st.undo_available.insert(3);
        st.apply(&EngineEvent::PatternSwitched { index: 1 });
        assert_eq!(st.session.as_ref().unwrap().active_pattern_index, 1);
        assert_eq!(st.queued_pattern, None);
        assert_eq!(st.queued_pattern_quantize, None);
        assert_eq!(st.pattern_loop_count, 0);
        assert!(st.playheads.is_empty());
        assert!(st.undo_available.is_empty());

        // PatternCleared on the active pattern nulls it + clears queued.
        st.apply(&EngineEvent::PatternQueued {
            index: 1,
            quantize: QuantizeGrain::NextBar,
        });
        st.apply(&EngineEvent::PatternCleared { index: 1 });
        assert!(st.session.as_ref().unwrap().patterns[1].is_none());
        assert_eq!(st.queued_pattern, None); // 1 == active → cleared
    }

    #[test]
    fn apply_transport_and_globals() {
        let mut st = state_with_session();
        st.pattern_loop_count = 7;
        st.apply(&EngineEvent::PlayStateChanged { playing: true });
        assert!(st.playing);
        assert_eq!(st.pattern_loop_count, 7); // play keeps loop count

        st.apply(&EngineEvent::BpmChanged { bpm: 140.0 });
        assert_eq!(st.session.as_ref().unwrap().bpm, 140.0);

        st.apply(&EngineEvent::SyncSourceChanged {
            source: SyncSource::Link,
        });
        assert_eq!(st.session.as_ref().unwrap().sync_source, SyncSource::Link);
        assert!(st.link_enabled); // auto-enable rule (Defect 3)

        st.apply(&EngineEvent::SyncSourceChanged {
            source: SyncSource::MidiClock,
        });
        assert!(!st.link_enabled);

        st.apply(&EngineEvent::PlayStateChanged { playing: false });
        assert!(!st.playing);
        assert_eq!(st.pattern_loop_count, 0); // stop resets loop count
    }

    #[test]
    fn apply_playhead_coalesces_last_wins() {
        let mut st = state_with_session();
        st.apply_playhead(0, 3);
        st.apply_playhead(0, 7); // overwrite same track → last wins
        st.apply_playhead(2, 11);
        assert_eq!(st.playheads.get(&0).copied(), Some(7));
        assert_eq!(st.playheads.get(&2).copied(), Some(11));
        assert_eq!(st.playheads.len(), 2);
    }

    #[test]
    fn apply_full_snapshot_replaces_and_resets() {
        let mut st = state_with_session();
        st.queued_pattern = Some(3);
        st.pattern_loop_count = 9;
        st.playheads.insert(0, 4);
        st.undo_available.insert(1);
        st.last_error = Some(EngineError {
            code: 5,
            message: "x".into(),
        });
        st.last_overflow = Some(2);

        let snap = Session {
            bpm: 99.0,
            ..Default::default()
        };
        st.apply(&EngineEvent::FullSnapshot { session: snap });
        assert_eq!(st.session.as_ref().unwrap().bpm, 99.0); // replaced wholesale
        assert_eq!(st.queued_pattern, None);
        assert_eq!(st.queued_pattern_quantize, None);
        assert_eq!(st.pattern_loop_count, 0);
        assert!(st.playheads.is_empty());
        assert!(st.undo_available.is_empty());
        assert!(st.last_error.is_none());
        assert!(st.last_overflow.is_none());
    }

    #[test]
    fn apply_undo_error_overflow_link() {
        let mut st = state_with_session();
        st.apply(&EngineEvent::UndoAvailable {
            track_idx: 1,
            available: true,
        });
        st.apply(&EngineEvent::UndoAvailable {
            track_idx: 4,
            available: true,
        });
        st.apply(&EngineEvent::UndoAvailable {
            track_idx: 1,
            available: false,
        }); // remove
        assert!(st.undo_available.contains(&4));
        assert!(!st.undo_available.contains(&1));

        st.apply(&EngineEvent::Error {
            code: -1,
            message: "boom".into(),
        });
        assert_eq!(
            st.last_error,
            Some(EngineError {
                code: -1,
                message: "boom".into(),
            })
        );

        st.apply(&EngineEvent::Overflow { dropped: 42 });
        assert_eq!(st.last_overflow, Some(42));

        st.apply(&EngineEvent::LinkPeersChanged { count: 3 });
        assert_eq!(st.link_peers, 3);
        st.apply(&EngineEvent::LinkEnabledChanged { enabled: true });
        assert!(st.link_enabled);
    }

    #[test]
    fn apply_out_of_range_indices_dropped_safely() {
        // No session → all session-touching events are no-ops, never panic.
        let mut empty = UiState::default();
        empty.apply(&EngineEvent::StepChanged {
            track_idx: 0,
            step_idx: 0,
            step: Step::default(),
        });
        assert!(empty.session.is_none());

        let mut st = state_with_session();
        st.apply(&EngineEvent::StepChanged {
            track_idx: 99,
            step_idx: 0,
            step: Step::default(),
        });
        st.apply(&EngineEvent::StepChanged {
            track_idx: 0,
            step_idx: 99,
            step: Step::default(),
        });
        st.apply(&EngineEvent::TrackRemoved { track_idx: 99 });
        st.apply(&EngineEvent::FollowActionChanged {
            pattern_idx: 99,
            action: FollowAction::default(),
        });
        assert_eq!(track(&st, 0).midi_note, 36); // untouched, no panic

        // Bad active_pattern_index → mutate_active_pattern no-op (bound guard).
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            s.active_pattern_index = 99;
        }
        st.apply(&EngineEvent::TrackLengthChanged {
            track_idx: 0,
            length: 4,
        });
        assert_eq!(track(&st, 0).length, 16); // pattern 0 untouched
    }

    #[test]
    fn apply_playhead_and_serialized_noop() {
        let mut st = state_with_session();
        st.apply(&EngineEvent::Playhead {
            track_idx: 0,
            step_idx: 2,
        });
        assert!(st.playheads.is_empty()); // apply() ignores Playhead

        let before = st.session.as_ref().unwrap().bpm;
        st.apply(&EngineEvent::Serialized {
            bytes: vec![1, 2, 3],
        });
        assert_eq!(st.session.as_ref().unwrap().bpm, before); // Serialized no-op
    }

    #[test]
    fn apply_sequence_matches_session_mirror() {
        // Scripted edit session → assert final state == hand-computed
        // SessionMirror semantics (the faithful-port oracle).
        let mut st = state_with_session();
        st.apply(&EngineEvent::PlayStateChanged { playing: true });
        st.apply(&EngineEvent::BpmChanged { bpm: 130.0 });
        st.apply(&EngineEvent::StepChanged {
            track_idx: 0,
            step_idx: 0,
            step: Step {
                active: true,
                velocity_zone: VelocityZone::Accent,
                ..Default::default()
            },
        });
        st.apply(&EngineEvent::StepChanged {
            track_idx: 0,
            step_idx: 4,
            step: Step {
                active: true,
                velocity_zone: VelocityZone::Mid,
                ..Default::default()
            },
        });
        st.apply(&EngineEvent::TrackMutedChanged {
            track_idx: 1,
            muted: true,
        });
        st.apply(&EngineEvent::TrackLengthChanged {
            track_idx: 2,
            length: 8,
        });
        st.apply(&EngineEvent::PatternLoopCountChanged { count: 3 });
        st.apply(&EngineEvent::UndoAvailable {
            track_idx: 0,
            available: true,
        });
        st.apply_playhead(0, 4);
        st.apply_playhead(1, 4);
        st.apply(&EngineEvent::LinkPeersChanged { count: 5 });

        assert!(st.playing);
        assert_eq!(st.session.as_ref().unwrap().bpm, 130.0);
        let t0 = track(&st, 0);
        assert!(t0.steps[0].active);
        assert_eq!(t0.steps[0].velocity_zone, VelocityZone::Accent);
        assert!(t0.steps[4].active);
        assert_eq!(t0.steps[4].velocity_zone, VelocityZone::Mid);
        assert!(!t0.steps[1].active); // untouched
        assert!(track(&st, 1).muted);
        assert_eq!(track(&st, 2).length, 8);
        assert_eq!(st.pattern_loop_count, 3);
        assert!(st.undo_available.contains(&0));
        assert_eq!(st.playheads.get(&0).copied(), Some(4));
        assert_eq!(st.playheads.get(&1).copied(), Some(4));
        assert_eq!(st.link_peers, 5);
    }
}
