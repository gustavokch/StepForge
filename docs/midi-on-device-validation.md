# On-device CoreMIDI `MIDISend` validation

The engine's CoreMIDI worker (`engine/crates/ffi/src/coremidi.rs::run_coremidi_worker`)
calls real `MIDISend`, but **real `MIDISend` delivery cannot be validated in CI**.
This document is the checklist for validating it on a device / real loopback.

## Why CI can't cover it

`cargo test` runs on the macOS host. The host tests that would prove
end-to-end `MIDISend` delivery (`engine/crates/ffi/tests/coremidi_host.rs`) are
`#[ignore]`'d because **CoreMIDI does not reliably route `MIDISend` to a virtual
endpoint created in the same process** — virtual destinations receive from
*external* sources, and same-process `MIDIReceived` → virtual-destination routing
is environment-dependent. This is a CoreMIDI platform behavior, not an engine bug.

What CI *does* cover (the 5 un-ignored worker-logic tests in `coremidi_host.rs`):
`MIDIPacketList` construction, FFI bindings are callable, Note-On/Note-Off
scheduling + ordering, stop `stop_generation` drain-drop, and all-notes-off (CC 123)
format. So the *logic* that decides what/when to send is tested; only the actual
wire delivery to a real endpoint is not.

Production connects to **external** endpoints (hardware synths, other apps), where
`MIDISend` works correctly.

## Validation options (pick one)

### Option A — macOS IAC Driver (loopback, no hardware)

The IAC Driver is a built-in macOS virtual MIDI bus that routes between apps —
`MIDISend` to an IAC destination *does* deliver (it's inter-app, not same-process).

1. Open **Audio MIDI Setup → Window → Show MIDI Studio**.
2. Double-click **IAC Driver** → check **Device is online** → add a port (e.g. `bus1`).
3. Run a MIDI monitor that listens on the IAC bus (e.g. `midi monitor` from the
   App Store, or `pbpaste`-class tools, or a tiny receiver app).
4. Point the engine at the IAC endpoint: submit `SetMidiDestinations { endpoints:
   [<IAC endpoint ref as u32>] }` (Swift discovers the IAC endpoint via
   `MIDIDestinationGetNumberOfSources`/`MIDIGetDestination` and passes its
   `MIDIEndpointRef as UInt32`).
5. `Play` a pattern with a hit; observe the **Note-On** then the gate-delayed
   **Note-Off** in the monitor.

### Option B — Physical interface + synth

1. Connect a class-compliant USB-MIDI interface + synth.
2. Swift discovers the endpoint and passes its integer ID via `SetMidiDestinations`.
3. `Play`; the synth voices the hits.

### Option C — Separate helper process (CI-able, no hardware)

Because CoreMIDI routes *inter-process*, a **second process** sending to / receiving
from a virtual endpoint works where same-process does not:

1. Helper process A creates a virtual **destination** (read proc records received bytes).
2. The engine (process B) `MIDISend`s to A's destination endpoint (discovered via
   `MIDIGetNumberOfDestinations`/`MIDIGetDestination`).
3. Assert A received Note-On then Note-Off in order.

This is the only option that could run unattended; it's a future CI enhancement,
not required for the host-testable-scope milestone.

## How to run the existing host test against a real endpoint

The ignored tests in `coremidi_host.rs` currently create a same-process virtual
endpoint. To validate against a real endpoint, adapt them:

1. Remove `#[ignore]` from a delivery test (or gate it behind an env var, e.g.
   `STEPFORGE_MIDI_ENDPOINT=<endpoint ref>`; skip if unset).
2. Replace the same-process virtual-destination setup with the discovered real
   endpoint (IAC / physical) — Swift side passes it via `SetMidiDestinations`.
3. Run: `cargo test -p sequencer_engine_ffi --test coremidi_host -- --ignored`
   (or with the env var set).

## What to assert (acceptance)

- A Note-On (`0x90 | channel`, `note`, `velocity`) arrives.
- A Note-Off (`0x80 | channel`, `note`, `0`) arrives **after** the Note-On, at
  approximately Note-On-time + `DEFAULT_GATE_MICROS` (50 ms).
- On `Stop`: no further Note-Ons; an all-notes-off (CC 123) arrives.
- Under the E9 worst-case budget (300 BPM × ratchet X4 × 8 tracks → ~1920
  Note-Ons/sec), no drops observed over a sustained run (ring depth 128 +
  ~120 Hz-equivalent worker drain should absorb it; confirm empirically).

This closes the loop the live-playback-to-physical-device follow-up depends on.
