//! Lock-free bounded queues: RT -> CoreMIDI worker (MIDI ring), RT -> Swift
//! (hot events). Arc-shared heapless::mpmc::Queue (&self enqueue/dequeue,
//! Send+Sync). Drop-oldest on overflow (E8).

use crate::command::Command;
use crate::event::EngineEvent;
use heapless::mpmc::MpMcQueue;
use std::sync::Arc;

pub const MIDI_RING_DEPTH: usize = 128;
pub const HOT_EVENT_DEPTH: usize = 32;
pub const COMMAND_DEPTH: usize = 64;
pub const MAX_EVENT_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiMsg {
    pub endpoint: u32,
    pub channel: u8,
    pub status: u8,
    pub note: u8,
    pub velocity: u8,
    pub send_at_offset_micros: u32,
    pub gate_micros: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotEventSlot {
    pub len: u8,
    pub bytes: [u8; MAX_EVENT_BYTES],
}

pub type MidiOutRing = Arc<MpMcQueue<MidiMsg, MIDI_RING_DEPTH>>;
pub type HotEventChannel = Arc<MpMcQueue<HotEventSlot, HOT_EVENT_DEPTH>>;
pub type CommandQueue = Arc<MpMcQueue<Command, COMMAND_DEPTH>>;

pub fn midi_out_ring() -> MidiOutRing {
    Arc::new(MpMcQueue::new())
}
pub fn hot_event_channel() -> HotEventChannel {
    Arc::new(MpMcQueue::new())
}
pub fn command_queue() -> CommandQueue {
    Arc::new(MpMcQueue::new())
}

/// Enqueue, dropping the OLDEST entry if full. Returns slots dropped (E8).
///
/// `T` is not required to be `Copy`: on rejection the value is moved back into
/// `v` and re-offered after the oldest slot is dequeued, so any `Send`-able `T`
/// (including the non-`Copy` `Command`) works.
pub fn push_drop_oldest<T, const N: usize>(q: &MpMcQueue<T, N>, val: T) -> usize {
    let mut dropped = 0;
    let mut v = val;
    loop {
        match q.enqueue(v) {
            Ok(()) => return dropped,
            Err(rej) => {
                v = rej;
                let _ = q.dequeue();
                dropped += 1;
            }
        }
    }
}

/// Encode `ev` into a stack buffer and push as a slot (drop-oldest).
pub fn push_event(events: &HotEventChannel, ev: &EngineEvent) -> usize {
    let mut buf = [0u8; MAX_EVENT_BYTES];
    let written = postcard::to_slice(&ev, &mut buf)
        .expect("event fits MAX_EVENT_BYTES")
        .len();
    debug_assert!(written <= MAX_EVENT_BYTES);
    push_drop_oldest(
        events,
        HotEventSlot {
            len: written as u8,
            bytes: buf,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_push_then_drain() {
        let r = midi_out_ring();
        let m = MidiMsg {
            endpoint: 1,
            channel: 10,
            status: 0x90,
            note: 36,
            velocity: 100,
            send_at_offset_micros: 0,
            gate_micros: 50_000,
        };
        assert_eq!(push_drop_oldest(&r, m), 0);
        assert_eq!(r.dequeue(), Some(m));
        assert_eq!(r.dequeue(), None);
    }

    #[test]
    fn ring_drops_oldest_when_full() {
        let q: Arc<MpMcQueue<u8, 2>> = Arc::new(MpMcQueue::new());
        assert_eq!(push_drop_oldest(&q, 1u8), 0);
        assert_eq!(push_drop_oldest(&q, 2u8), 0);
        assert_eq!(push_drop_oldest(&q, 3u8), 1); // full -> drop oldest (1)
                                                  // remaining: [2,3]
        assert_eq!(q.dequeue(), Some(2));
        assert_eq!(q.dequeue(), Some(3));
    }

    #[test]
    fn push_event_round_trips_through_postcard() {
        use crate::event::EngineEvent;
        let ch = hot_event_channel();
        let dropped = push_event(
            &ch,
            &EngineEvent::Playhead {
                track_idx: 0,
                step_idx: 5,
            },
        );
        assert_eq!(dropped, 0);
        let slot = ch.dequeue().expect("one slot");
        let back: EngineEvent = postcard::from_bytes(&slot.bytes[..slot.len as usize]).unwrap();
        assert_eq!(
            back,
            EngineEvent::Playhead {
                track_idx: 0,
                step_idx: 5
            }
        );
    }
}
