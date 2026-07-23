//! Pattern queue + quantize-grain + follow-actions. Transitions emit an
//! all-notes-off burst (bounded) so stale notes don't ring across patterns.

use crate::midi::build_all_notes_off;
use crate::midi_out::push_drop_oldest;
use crate::midi_out::MidiOutRing;
use crate::models::{QuantizeGrain, Session, MAX_TRACKS};

#[derive(Default)]
pub struct Scheduler {
    queued: Option<(usize, QuantizeGrain)>,
}

impl Scheduler {
    pub fn queue(&mut self, index: usize, grain: QuantizeGrain) {
        self.queued = Some((index, grain));
    }
    pub fn cancel(&mut self) {
        self.queued = None;
    }
    /// Called by the RT loop at a pattern boundary. Returns the new active index
    /// (if a queued pattern fires at this grain) and pushes all-notes-off.
    pub fn on_boundary(
        &mut self,
        s: &Session,
        midi: &MidiOutRing,
        at_grain: QuantizeGrain,
    ) -> Option<usize> {
        let fire =
            matches!(self.queued, Some((_, g)) if g == at_grain || grain_reached(g, at_grain));
        if fire {
            let (idx, _) = self.queued.take()?;
            // all-notes-off burst, bounded to MAX_TRACKS so it can't self-overflow
            for _tr in 0..MAX_TRACKS {
                let _ = push_drop_oldest(midi, build_all_notes_off(0, s.global_midi_channel));
            }
            return Some(idx);
        }
        None
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::QuantizeGrain::*;
    #[test]
    fn queued_pattern_fires_at_grain() {
        let mut sch = Scheduler::default();
        let s = Session::default();
        let midi = crate::midi_out::midi_out_ring();
        sch.queue(3, NextBar);
        assert_eq!(sch.on_boundary(&s, &midi, NextStep), None); // too early
        assert_eq!(sch.on_boundary(&s, &midi, NextBar), Some(3)); // fires
        assert!(sch.queued.is_none());
    }
}
