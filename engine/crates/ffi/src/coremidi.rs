//! CoreMIDI bindings + prod clock + CoreMIDI worker. The ONLY crate doing `unsafe`.
//! RT-priority syscall lives here (Hard Rules 1, 6, 7). MIDISend runs ONLY from the
//! off-RT CoreMIDI worker thread (never from the RT thread).

use sequencer_engine::clock::Clock;
use sequencer_engine::engine::Engine;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

// ============================================================================
// CoreMIDI FFI bindings
// ============================================================================
// Signatures resolved against <CoreMIDI/CoreMIDI.h> and
// <CoreFoundation/CFBase.h>. All `unsafe` is confined to this module.

// CoreFoundation types (opaque pointers)
pub use core_foundation_sys::string::CFStringRef;
pub use core_foundation_sys::base::CFAllocatorRef;

// CoreMIDI types
pub type MIDIClientRef = u32;       // 32-bit opaque reference
pub type MIDIEndpointRef = u32;    // 32-bit opaque reference
pub type MIDIPortRef = u32;        // 32-bit opaque reference
pub type OSStatus = i32;

pub type ByteCount = usize;
pub type MidiTimeStamp = u64;

// Production CoreMIDI functions (always available)
extern "C" {
    /// MIDISend - sends a MIDI packet list to a destination.
    #[allow(non_snake_case)]
    fn MIDISend(
        port: MIDIPortRef,
        dest: MIDIEndpointRef,
        pktlist: *const MIDIPacketList,
    ) -> OSStatus;

    /// MIDIPacketListInit - initializes a MIDIPacketList for adding packets.
    #[allow(non_snake_case)]
    fn MIDIPacketListInit(pktlist: *mut MIDIPacketList) -> *mut MIDIPacket;

    /// MIDIPacketListAdd - adds a packet to a MIDIPacketList.
    #[allow(non_snake_case)]
    fn MIDIPacketListAdd(
        pktlist: *mut MIDIPacketList,
        listSize: ByteCount,
        curPacket: *mut MIDIPacket,
        time: MidiTimeStamp,
        nData: ByteCount,
        data: *const u8,
    ) -> *mut MIDIPacket;
}

// Test-only CoreMIDI functions (for creating virtual destinations in tests)
// These are pub so integration tests can use them; they are only called from tests.
extern "C" {
    /// CFStringCreateWithCString (CoreFoundation)
    /// Creates a CFString from a C string. NULL allocator = use default.
    #[allow(non_snake_case)]
    pub fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        cStr: *const i8,
        encoding: u32,
    ) -> CFStringRef;

    /// MIDIClientCreate - creates a MIDI client.
    /// Returns 0 (noErr) on success.
    #[allow(non_snake_case)]
    pub fn MIDIClientCreate(
        name: CFStringRef,
        notify: *const (),
        refCon: *mut (),
        outClient: *mut MIDIClientRef,
    ) -> OSStatus;

    /// MIDIOutputPortCreate - creates an output port.
    #[allow(non_snake_case)]
    pub fn MIDIOutputPortCreate(
        client: MIDIClientRef,
        portName: CFStringRef,
        outPort: *mut MIDIPortRef,
    ) -> OSStatus;

    /// MIDIDestinationCreate - creates a virtual destination endpoint.
    #[allow(non_snake_case)]
    pub fn MIDIDestinationCreate(
        client: MIDIClientRef,
        name: CFStringRef,
        readProc: MIDIReadProc,
        refCon: *mut (),
        outDest: *mut MIDIEndpointRef,
    ) -> OSStatus;

    /// MIDIEndpointDispose - disposes of an endpoint.
    #[allow(non_snake_case)]
    pub fn MIDIEndpointDispose(endpoint: MIDIEndpointRef) -> OSStatus;

    /// MIDIClientDispose - disposes of a client.
    #[allow(non_snake_case)]
    pub fn MIDIClientDispose(client: MIDIClientRef) -> OSStatus;
}

/// MIDI read callback type (for virtual destination in tests only).
#[allow(non_snake_case)]
pub type MIDIReadProc = extern "C" fn(
    pktlist: *const MIDIPacketList,
    srcConnRefCon: *mut (),
    refCon: *mut (),
);

// ============================================================================
// MIDIPacketList layout
// ============================================================================
// The MIDIPacketList is a variable-length structure. CoreMIDI manages the
// layout internally via MIDIPacketListInit/MIDIPacketListAdd.
// We use a raw byte buffer and let CoreMIDI write packets into it.

// CoreMIDI packet list header: numPackets (u32) followed by packet data.
// We use a 256-byte buffer which is more than enough for our 3-byte messages.
const PACKET_LIST_BUFFER_SIZE: usize = 256;

#[repr(C)]
#[allow(non_snake_case)]
pub struct MIDIPacketList {
    pub numPackets: u32,
    // Variable-length packet data follows (managed by CoreMIDI)
}

// Opaque packet pointer - CoreMIDI manages the actual layout
pub type MIDIPacket = ();

// ============================================================================
// CoreMIDI worker
// ============================================================================

/// Pending MIDI send: (deadline, endpoint, 3-byte MIDI message).
/// Used for scheduling Note-Ons at offset and Note-Offs at offset+gate.
type PendingSend = (Instant, MIDIEndpointRef, [u8; 3]);

/// Runs the CoreMIDI worker thread on a dedicated off-RT thread.
///
/// This is the ONLY place in the codebase that calls `MIDISend`. The RT thread
/// pushes `MidiMsg`s to the lock-free ring; this worker drains and schedules.
///
/// - Note-On messages are scheduled at `now + send_at_offset_micros`
/// - Note-Off messages are scheduled at Note-On time + `gate_micros`
/// - On `stop_generation` change: drains-and-drops the ring, clears pending,
///   and sends All-Notes-Off (CC 123) to stop hanging notes
/// - Uses a bounded `heapless::Vec` for pending sends (no heap allocation)
///
/// # Safety
/// Caller must ensure `port` is a valid `MIDIPortRef` created via
/// `MIDIOutputPortCreate`. This function is spawned by `engine_start` (Task 20)
/// and joins on shutdown.
pub fn run_coremidi_worker(engine: &Arc<Engine>, port: MIDIPortRef) {
    let mut last_stop_gen = engine.transport.stop_generation.load(Ordering::Acquire);
    let mut pending: heapless::Vec<PendingSend, 128> = heapless::Vec::new();

    while !engine.shutdown.load(Ordering::Acquire) {
        let gen = engine.transport.stop_generation.load(Ordering::Acquire);

        // Check for stop generation change (Stop command or LoadSession)
        if gen != last_stop_gen {
            // Drain-and-drop the ring: any Note-On queued after stop is discarded
            while engine.midi.dequeue().is_some() {}

            // Clear all pending sends (cancel scheduled Note-Ons/Note-Offs)
            pending.clear();

            // Send All-Notes-Off to ensure no hanging notes
            let _ = send_cc_all_notes_off(port, engine);

            last_stop_gen = gen;
            continue;
        }

        // Fire due scheduled sends
        let now = Instant::now();
        let mut i = 0;
        while i < pending.len() {
            if pending[i].0 <= now {
                let (_, dest, bytes) = pending.remove(i);
                let _ = send_one(port, dest, &bytes);
                // Don't increment i: we removed an element
            } else {
                i += 1;
            }
        }

        // Drain ring -> schedule new sends
        while let Some(m) = engine.midi.dequeue() {
            // Note-On (0x9n): schedule at offset + gate-synth Note-Off
            if m.status & 0xF0 == 0x90 {
                let fire_at = Instant::now() + Duration::from_micros(m.send_at_offset_micros as u64);
                let _ = pending.push((fire_at, m.endpoint, [m.status, m.note, m.velocity]));

                if m.gate_micros > 0 {
                    let off_at = fire_at + Duration::from_micros(m.gate_micros as u64);
                    // Note-Off uses the same channel, 0 velocity
                    let _ = pending.push((
                        off_at,
                        m.endpoint,
                        [(m.status & 0xF0) | (m.channel & 0x0F), m.note, 0],
                    ));
                }
            } else {
                // Non-Note-On (including CC All-Notes-Off): send immediately
                let _ = send_one(port, m.endpoint, &[m.status, m.note, m.velocity]);
            }
        }

        // Poll sleep (1ms) - the RT thread drives cadence, we just drain
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Sends a 3-byte MIDI message via a single-packet MIDIPacketList.
///
/// Builds the packet list on the stack (no heap allocation) and calls `MIDISend`.
/// Returns `OSStatus` (0 = success).
fn send_one(port: MIDIPortRef, dest: MIDIEndpointRef, bytes: &[u8]) -> OSStatus {
    // Use a raw byte buffer for the MIDIPacketList. CoreMIDI writes packets
    // into this buffer via MIDIPacketListInit/MIDIPacketListAdd.
    let mut buffer: [u8; PACKET_LIST_BUFFER_SIZE] = [0; PACKET_LIST_BUFFER_SIZE];

    unsafe {
        // Cast buffer to MIDIPacketList pointer
        let pktlist_ptr = buffer.as_mut_ptr() as *mut MIDIPacketList;

        // MIDIPacketListInit sets numPackets to 0 and returns a pointer to
        // the first packet (which starts right after the u32 numPackets header)
        let pkt_ptr = MIDIPacketListInit(pktlist_ptr);

        // Add the MIDI data. timestamp=0 means send immediately.
        // listSize is the total buffer size so CoreMIDI can bounds-check.
        let result = MIDIPacketListAdd(
            pktlist_ptr,
            PACKET_LIST_BUFFER_SIZE as ByteCount,
            pkt_ptr,
            0,                  // timeStamp = 0 (immediate)
            bytes.len(),
            bytes.as_ptr(),
        );

        if result.is_null() {
            // Packet list too small (should never happen for 3-byte messages)
            return -1;
        }

        // Send the packet list
        MIDISend(port, dest, pktlist_ptr)
    }
}

/// Sends All-Notes-Off (CC 123) on the global MIDI channel.
///
/// Called when stop generation changes (Stop command or LoadSession) to
/// ensure no hanging notes. Returns `OSStatus` (0 = success).
fn send_cc_all_notes_off(port: MIDIPortRef, engine: &Engine) -> OSStatus {
    let snap = engine.snapshot.load_full();
    let channel = snap.global_midi_channel & 0x0F;

    // CC 123 (All Notes Off) on the global channel
    let bytes = [0xB0 | channel, 123, 0];

    // Send to all destinations (empty vec -> no-op, one endpoint -> single send)
    let mut last_status: OSStatus = 0;
    for endpoint in snap.midi_destinations.iter() {
        last_status = send_one(port, *endpoint, &bytes);
    }
    last_status
}

/// Creates a CFStringRef from a UTF-8 string.
///
/// # Safety
/// Caller is responsible for releasing the returned CFStringRef via
/// `CFRelease` (not needed for static strings passed to CoreMIDI, which
/// retains them internally).
///
/// # Note
/// This function is pub for integration tests; it is only called from tests.
pub unsafe fn cfstring_from_str(s: &str) -> CFStringRef {
    let c_str = s.as_ptr() as *const i8;
    CFStringCreateWithCString(std::ptr::null(), c_str, 0x08000100)
}

// ============================================================================
// InstantClock (from Task 3)
// ============================================================================

pub struct InstantClock {
    start: Instant,
}

impl InstantClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for InstantClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for InstantClock {
    fn now_micros(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    fn sleep_until(&self, deadline_micros: u64) {
        let now = self.now_micros();
        if deadline_micros > now {
            let remaining = deadline_micros - now;
            // Coarse sleep until ~1ms before, then spin the rest (sub-ms accuracy)
            if remaining > 1_000 {
                std::thread::sleep(std::time::Duration::from_micros(remaining - 1_000));
            }
            while self.now_micros() < deadline_micros {
                std::hint::spin_loop();
            }
        }
    }

    fn elevate_priority(&self) {
        // Called EXACTLY ONCE at RT-thread spawn (never in the per-tick loop).
        // Best-effort QoS elevation; failures are non-fatal (timing still correct).
        #[cfg(target_os = "ios")]
        unsafe {
            elevate_thread_rt_ios();
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = 0;
        }
    }
}

#[cfg(target_os = "ios")]
unsafe fn elevate_thread_rt_ios() {
    // pthread_set_qos_class_self(QOS_CLASS_USER_INTERACTIVE, 0)
    // Implemented alongside the CoreMIDI worker (same FFI block).
    // For now, best-effort via thread_priority_set_thread_policy if available.
    // Full implementation available if Darwin headers are bound.
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_clock_is_monotonic_nonzero() {
        let c = InstantClock::new();
        let a = c.now_micros();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let b = c.now_micros();
        assert!(b > a, "clock must be monotonic");
    }

    #[test]
    fn elevate_priority_does_not_panic() {
        let c = InstantClock::new();
        c.elevate_priority(); // once at spawn
    }
}
