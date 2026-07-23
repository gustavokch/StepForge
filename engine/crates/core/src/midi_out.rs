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
/// Depth of the off-RT large-payload channel (D5/A2). Sized small: large
/// payloads (`Serialized`/`FullSnapshot`/`Error`) are rare (full-state reads,
/// not per-tick) and individually sizeable, so a shallow queue bounds memory.
pub const LARGE_EVENT_DEPTH: usize = 8;

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
/// Off-RT large-payload event channel (D5/A2). Carries owned `EngineEvent`s
/// that are too large for the 128-byte hot slot — `Serialized`, `FullSnapshot`,
/// `Error`. Produced by the state worker (never RT); drained alongside the hot
/// channel by Swift. `MpMcQueue<EngineEvent, _>` is `Send+Sync` because
/// `EngineEvent: Send`.
pub type LargeEventChannel = Arc<MpMcQueue<EngineEvent, LARGE_EVENT_DEPTH>>;

pub fn midi_out_ring() -> MidiOutRing {
    Arc::new(MpMcQueue::new())
}
pub fn hot_event_channel() -> HotEventChannel {
    Arc::new(MpMcQueue::new())
}
pub fn command_queue() -> CommandQueue {
    Arc::new(MpMcQueue::new())
}
pub fn large_event_channel() -> LargeEventChannel {
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
///
/// Intended ONLY for small events (`Playhead`, transport, `Overflow`) — the
/// `.expect()` panics if the encoded form exceeds `MAX_EVENT_BYTES`. Large
/// payloads (`Serialized`/`FullSnapshot`/`Error`) MUST go through
/// [`push_large_event`] on the off-RT [`LargeEventChannel`] (A2/D5).
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

/// Drop-oldest enqueue of an OWNED `EngineEvent` onto the off-RT large-payload
/// channel (A2/D5). Used by the state worker for `Serialized`/`FullSnapshot`/
/// `Error` — payloads that can exceed the 128-byte hot-channel slot. Reuses
/// [`push_drop_oldest`], which already handles non-`Copy` owned `T`. Returns
/// the number of slots dropped.
pub fn push_large_event(q: &LargeEventChannel, ev: EngineEvent) -> usize {
    push_drop_oldest(q.as_ref(), ev)
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

    /// Large channel carries an owned non-`Copy` `EngineEvent::Serialized`
    /// (a `Vec<u8>` payload that would blow the 128-byte hot slot) intact.
    #[test]
    fn large_channel_round_trips_owned_serialized_event() {
        let ch = large_event_channel();
        // 256 bytes > MAX_EVENT_BYTES (128): would panic in push_event.
        let bytes = vec![0xABu8; 256];
        let ev = EngineEvent::Serialized { bytes };
        let dropped = push_large_event(&ch, ev.clone());
        assert_eq!(dropped, 0);
        let back = ch.dequeue().expect("one large event");
        assert_eq!(back, ev);
        assert!(ch.dequeue().is_none(), "channel drained");
    }

    /// Large channel is drop-oldest (E8) — confirms non-`Copy` `EngineEvent`
    /// flows through `push_drop_oldest` without panic.
    #[test]
    fn large_channel_drops_oldest_when_full() {
        let ch = large_event_channel();
        // Fill (LARGE_EVENT_DEPTH = 8) + 1 overflow.
        for i in 0..(LARGE_EVENT_DEPTH + 1) {
            let dropped = push_large_event(
                &ch,
                EngineEvent::Serialized {
                    bytes: vec![i as u8],
                },
            );
            // Only the (N+1)-th push drops exactly one entry.
            if i < LARGE_EVENT_DEPTH {
                assert_eq!(dropped, 0, "push {i} should fit");
            } else {
                assert_eq!(dropped, 1, "push {i} should drop oldest");
            }
        }
        // Oldest (vec![0]) dropped; remaining are [1 ..= 8].
        for expected in 1..=(LARGE_EVENT_DEPTH as u8) {
            match ch.dequeue() {
                Some(EngineEvent::Serialized { bytes }) => {
                    assert_eq!(bytes, vec![expected]);
                }
                other => panic!("expected Serialized{{ {expected} }}, got {other:?}"),
            }
        }
        assert!(ch.dequeue().is_none(), "channel drained");
    }
}
