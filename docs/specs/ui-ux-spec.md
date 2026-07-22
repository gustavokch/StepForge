# iOS MIDI Drum Sequencer — UI/UX Design Specification

## 1. Application Modes
The app features two primary layout modes, toggled via a corner icon. Switching layouts never interrupts playback or resets state. Mode switches are purely a UI concern — the Rust engine is unaware of which mode is active.

### 1.1 Editing Mode (Default)
Optimized for step-sequencing and pattern programming.
- **Top Bar (Row 1 - Transport)**: Play/Stop (large, leftmost), BPM display (tap to type/tap-tempo), Sync Source badge (Free/MIDI Clock/Link), 8/16 Zoom Toggle (top right).
- **Top Bar (Row 2 - Feel)**: Global Swing %, Humanize button (expands popover for Timing Jitter & Velocity Randomization sliders), Pattern Management button, Quantize Grain selector.
- **Track Area**: 4-8 track rows vertically scrollable. Track headers pinned horizontally on the left. Step grid to the right.

### 1.2 Performance Mode
Optimized for live jamming and pattern triggering.
- **Top Bar**: Play/Stop (enlarged), Patterns button (enlarged, shows live loop-progress ring on the button itself), Quantize Grain selector (enlarged).
- **Hidden/Collapsed Elements**: BPM (passive readout), Sync badge (icon only), Swing/Humanize (hidden).
- **Track Area**: Simplified track rows showing Track Name, Mute toggle, and Activity indicator. Full step grid is hidden by default to maximize space, but can be expanded if the user wants to tweak mid-set.

---

## 2. Grid & Step Gestures
The core composition area is a 4x16 (or 4x8) grid of steps. All gestures are interpreted by SwiftUI and forwarded to the Rust engine as commands. The UI never mutates step state directly — it mirrors the engine's response.

### 2.1 Grid Display Rules
- **8/16 Toggle**: Global switch to display 8 steps or 16 steps on screen. Pinch-to-zoom is supported as a shortcut.
- **Horizontal Scroll**: Users can pan tracks horizontally. Scroll position persists per track when paging vertically.
- **Track Length Window**: Track length (1-16) is a playback window. Steps beyond the set length are dimmed but visible, ensuring non-destructive editing.

### 2.2 Step Gestures
- **Tap (Empty Step)**: Places a hit at Mid velocity instantly. No delay.
- **Tap (Filled Step)**: No-op (prevents accidental deletion during fast programming).
- **Drag Up (Filled Step)**: Snaps velocity to Accent zone.
- **Drag Down (Filled Step)**: Snaps velocity to Low zone.
- **Double Tap (Filled Step)**: Deletes the step. (Debounced ~150ms after a placement to prevent misfires).
- **Long Press (~450ms)**: Opens Ratchet Popover (Off, 2x, 3x, 4x).

### 2.3 Velocity Visuals
Velocity is displayed using discrete color hues (not just opacity) for sunlight legibility. Haptic feedback (light tap) fires when crossing velocity zones during a drag.

### 2.4 Playhead
The engine pushes a playhead position (track index + step index) to the UI at audio-rate intervals. The UI coalesces these to display refresh rate (~60 Hz). The playhead is a read-only visual — the user cannot drag it.

---

## 3. Track Headers & Action Drawer
Track headers are pinned to the left edge so they remain visible during horizontal scrolling.

### 3.1 Header Contents
- Mute/On-Off toggle
- Note Name (Tap to open Note Picker)
- Speed Ratio Badge (e.g., 1x, 2x, 1/2x)
- `...` button (Tap to open Action Drawer)

### 3.2 Action Drawer
A horizontal slide-out strip overlaying the leftmost steps of the selected track. Does not interrupt playback.
- **Roll**: Single-tap. Applies randomized presence/position/value based on a strength dial. Provides a brief "✕ revert / ✓ keep" affordance. The revert button triggers an engine undo for that track.
- **Vary**: Single-tap. Perturbs non-accent steps while locking accents. Provides "✕ revert / ✓ keep" affordance. The revert button triggers an engine undo for that track.
- **Cut / Copy / Paste / Trash**: Standard clipboard actions. Trash clears step content but leaves track structure (length, note, speed) intact.

### 3.3 Track Management
- **`+` (Green) / `-` (Red) Buttons**: Located above the tracks.
- **Add (`+`)**: Instantly adds a new track (up to 8 max). Auto-scrolls to it.
- **Remove (`-`)**: Quick tap shows a tooltip warning ("Hold to remove track"). Long-press (~500ms) deletes the bottom-most track (down to 4 min). Fires haptic on deletion.

---

## 4. Pattern Management & Jamming
Accessed via the Patterns button in the top bar.

### 4.1 Pattern Picker (3x3 Grid)
- A non-modal popover displaying 9 pattern slots.
- **States**: Empty (dim), Filled (solid), Currently Playing (highlighted + progress ring), Queued (pulsing). State is derived from engine events, not UI-local timers.
- **Tap Filled**: Queues pattern to play at the next quantize boundary.
- **Tap Active**: Retriggers pattern from step 1 at the next quantize boundary.
- **Long Press Active**: Retriggers at 1/16th note quantization (ignores global grain).
- **Long Press Filled**: Opens options (Duplicate, Clear, Copy, Paste, Follow Actions).

### 4.2 Quantize Grain Control
A tap-to-cycle button next to the Patterns button.
- Options: Next Step, Next Beat, Next Bar, End of Pattern.
- Defaults to "End of Pattern" in Arrangement Mode, "Next Beat" in Jam Mode.

### 4.3 Modes (Arrangement vs. Jam)
- **Arrangement Mode**: Follow-actions drive the sequence. Manual taps are exceptions.
- **Jam Mode**: Manual taps expected constantly. Any manual queue resets the current pattern's loop counter, pausing follow-actions until the user stops interacting.

### 4.4 Follow Actions
Evaluated after N loops of the current pattern.
- Actions: None (default), Play Next, Play Specific, Play Previous, Stop, Play Random.
- Visual: Small badge on the grid cell (e.g., "→3" meaning advance after 3 loops).

---

## 5. MIDI Routing & Note Picker UI

### 5.1 MIDI Devices Screen
- Accessed via a MIDI icon. Lists available USB and Network MIDI destinations. Device discovery and connection lifecycle are managed by Swift; dispatch is handled by the Rust engine.
- Multi-destination supported (toggle multiple endpoints On).
- Global MIDI Channel selector (defaults to Channel 10).

### 5.2 Note Picker (Hybrid UI)
Triggered by tapping the Note Name in the Track Header.
- **View 1 (Drum Names)**: Default. Scrollable grid of GM drum names (Kick, Snare, etc.). Tapping assigns the note.
- **View 2 (Piano Roll)**: Toggle to view a mini 2-octave keyboard for custom note mapping.
- **Header Display**: Shows Drum Name (e.g., "Kick") or Note Name (e.g., "C2").

---

## 6. UI/Engine Contract
The UI layer (SwiftUI) and the sequencer core (Rust) are decoupled. The following contract governs their interaction:

### 6.1 UI Responsibilities
- Render all state from a lightweight mirror updated by engine events.
- Translate gestures into engine commands (tap, drag, long-press → `SetStep`, `DeleteStep`, `SetRatchet`, etc.).
- Never read or write engine state directly — always through the command/event channel.
- Coalesce playhead events to display refresh rate.
- Own transient UI state (scroll position, popover visibility, drag-in-progress coordinates) that has no musical meaning.

### 6.2 Engine Responsibilities
- Accept commands asynchronously (lock-free queue). The UI thread never blocks on the engine.
- Emit events for any state change that affects the UI (step toggled, velocity changed, pattern switched, playhead advanced, follow-action fired, undo snapshot pushed).
- Emit a full state snapshot on initial load and on explicit request.
- Guarantee that commands are applied in submission order.

### 6.3 Responsiveness Targets
- **Step tap → visual confirmation**: ≤ 16 ms (one display frame). The UI optimistically renders the expected state; if the engine rejects the command (e.g., race condition), a correction event follows within 1 tick.
- **Pattern switch queue → UI feedback**: Immediate (pulsing state on the cell).
- **App backgrounding → state serialized**: ≤ 100 ms from `applicationWillResignActivity`.
