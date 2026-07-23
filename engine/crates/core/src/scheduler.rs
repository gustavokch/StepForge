//! Pattern-switch scheduling across the RT/worker threading seam.
//!
//! The queued pattern + retrigger requests live in atomics ([`SchedulerClock`]),
//! written by the state worker (`QueuePattern`/`CancelQueuedPattern`/
//! `RetriggerPattern`) and consumed by the RT loop at quantize boundaries. The RT
//! loop fires a switch by pushing an all-notes-off burst + setting a
//! `switch_request`; the worker observes that and publishes the new
//! `active_pattern_index` (COW) + emits `PatternSwitched`.
//!
//! Nothing here crosses the RT path with a lock — only atomic loads/stores +
//! ring pushes (Hard Rule 1).

use crate::midi::build_all_notes_off;
use crate::midi_out::{push_drop_oldest, MidiOutRing};
use crate::models::{QuantizeGrain, Session, MAX_TRACKS};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

/// Sentinel for "no pattern queued / no switch requested".
pub const NO_PATTERN: usize = usize::MAX;

fn grain_to_u8(g: QuantizeGrain) -> u8 {
    match g {
        QuantizeGrain::NextStep => 0,
        QuantizeGrain::NextBeat => 1,
        QuantizeGrain::NextBar => 2,
        QuantizeGrain::EndOfPattern => 3,
    }
}
fn u8_to_grain(b: u8) -> QuantizeGrain {
    match b {
        1 => QuantizeGrain::NextBeat,
        2 => QuantizeGrain::NextBar,
        3 => QuantizeGrain::EndOfPattern,
        _ => QuantizeGrain::NextStep,
    }
}

/// Whether a queued grain is satisfied at the boundary the RT loop just reached.
fn grain_reached(queued: QuantizeGrain, at: QuantizeGrain) -> bool {
    use QuantizeGrain::*;
    matches!(
        (queued, at),
        (NextStep, _)
            | (NextBeat, NextBeat | NextBar | EndOfPattern)
            | (NextBar, NextBar | EndOfPattern)
            | (EndOfPattern, EndOfPattern)
    )
}

/// The finest grain boundary the RT loop reached at `global_step` (0..STEP_COUNT).
/// `0` = the pattern just wrapped (bar/pattern end); multiples of 4 = a beat;
/// otherwise a step.
fn boundary_at(global_step: u32) -> QuantizeGrain {
    use QuantizeGrain::*;
    if global_step == 0 {
        EndOfPattern
    } else if global_step.is_multiple_of(4) {
        NextBeat
    } else {
        NextStep
    }
}

/// Shared scheduler state (worker writes via `apply_command`; RT reads/fires).
pub struct SchedulerClock {
    queued_pattern: AtomicUsize,
    queued_grain: AtomicU8,
    /// RT → worker: switch `active_pattern_index` to this index.
    switch_request: AtomicUsize,
    /// `RetriggerPattern`: RT resets step counters at the next step boundary.
    retrigger_request: AtomicBool,
}
impl Default for SchedulerClock {
    fn default() -> Self {
        // Sentinels, not zero — `NO_PATTERN` means "nothing queued / no switch".
        Self {
            queued_pattern: AtomicUsize::new(NO_PATTERN),
            queued_grain: AtomicU8::new(0),
            switch_request: AtomicUsize::new(NO_PATTERN),
            retrigger_request: AtomicBool::new(false),
        }
    }
}
impl SchedulerClock {
    pub fn queue(&self, index: usize, grain: QuantizeGrain) {
        self.queued_pattern.store(index, Ordering::Release);
        self.queued_grain
            .store(grain_to_u8(grain), Ordering::Release);
    }
    pub fn cancel(&self) {
        self.queued_pattern.store(NO_PATTERN, Ordering::Release);
    }
    pub fn request_retrigger(&self) {
        self.retrigger_request.store(true, Ordering::Release);
    }
    /// RT: consume a pending queued pattern if its grain is reached at `global_step`.
    /// Returns `Some(index)` if it fires.
    pub fn take_if_due(&self, global_step: u32) -> Option<usize> {
        let idx = self.queued_pattern.load(Ordering::Acquire);
        if idx == NO_PATTERN {
            return None;
        }
        let grain = u8_to_grain(self.queued_grain.load(Ordering::Acquire));
        if grain_reached(grain, boundary_at(global_step)) {
            self.queued_pattern.store(NO_PATTERN, Ordering::Release);
            Some(idx)
        } else {
            None
        }
    }
    pub fn take_retrigger(&self) -> bool {
        self.retrigger_request.swap(false, Ordering::AcqRel)
    }
    /// RT → worker: request an `active_pattern_index` switch.
    pub fn request_switch(&self, index: usize) {
        self.switch_request.store(index, Ordering::Release);
    }
    /// Worker: consume a pending switch request.
    pub fn take_switch(&self) -> Option<usize> {
        let idx = self.switch_request.swap(NO_PATTERN, Ordering::AcqRel);
        if idx == NO_PATTERN {
            None
        } else {
            Some(idx)
        }
    }
}

/// RT fires a pattern switch: all-notes-off burst on the real endpoint, bounded
/// to `MAX_TRACKS` so it can't self-overflow the ring. RT-safe (ring push only).
pub fn all_notes_off_burst(snap: &Session, midi: &MidiOutRing) {
    let endpoint = snap.midi_destinations.first().copied().unwrap_or(0);
    for _ in 0..MAX_TRACKS {
        let _ = push_drop_oldest(
            midi,
            build_all_notes_off(endpoint, snap.global_midi_channel),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::QuantizeGrain::*;
    #[test]
    fn grain_lattice() {
        // NextStep fires at any boundary
        assert!(grain_reached(NextStep, NextStep));
        assert!(grain_reached(NextStep, EndOfPattern));
        // NextBeat fires at beat/bar/end, not a plain step
        assert!(grain_reached(NextBeat, NextBeat));
        assert!(grain_reached(NextBeat, EndOfPattern));
        assert!(!grain_reached(NextBeat, NextStep));
        // NextBar fires at bar/end only
        assert!(!grain_reached(NextBar, NextBeat));
        assert!(grain_reached(NextBar, EndOfPattern));
        // EndOfPattern only at end
        assert!(!grain_reached(EndOfPattern, NextBar));
        assert!(grain_reached(EndOfPattern, EndOfPattern));
    }
    #[test]
    fn boundary_classification() {
        assert_eq!(boundary_at(0), EndOfPattern);
        assert_eq!(boundary_at(4), NextBeat);
        assert_eq!(boundary_at(8), NextBeat);
        assert_eq!(boundary_at(1), NextStep);
        assert_eq!(boundary_at(7), NextStep);
    }
    #[test]
    fn scheduler_clock_queue_and_fire() {
        let sc = SchedulerClock::default();
        assert_eq!(sc.take_if_due(0), None); // nothing queued
        sc.queue(3, NextBar);
        assert_eq!(sc.take_if_due(4), None); // NextBar not reached at a beat
        assert_eq!(sc.take_if_due(0), Some(3)); // fires at the pattern end
        assert_eq!(sc.take_if_due(0), None); // consumed
    }
    #[test]
    fn scheduler_clock_switch_and_retrigger() {
        let sc = SchedulerClock::default();
        assert_eq!(sc.take_switch(), None);
        sc.request_switch(2);
        assert_eq!(sc.take_switch(), Some(2));
        assert_eq!(sc.take_switch(), None);
        assert!(!sc.take_retrigger());
        sc.request_retrigger();
        assert!(sc.take_retrigger());
        assert!(!sc.take_retrigger());
    }
}
