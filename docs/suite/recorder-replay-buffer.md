# Recorder Phase 12: Replay buffer (spec)

Nov 26, 2026 · @JD · approved (simplified)

Exported from the Claude Project "Creator Suite" ("Recorder Phase 12 Replay Buffer Spec"). The Project copy is the source.

## What the user sees

1. **Replay buffer on/off.** Capture menu, a toggle on the deck, hotkey Ctrl+Alt+B.
2. **Save replay.** A button on the deck (only while the buffer is on), Capture menu, hotkey Ctrl+Alt+S. Saves the last N seconds as a normal take.
3. **Length.** Settings > Recording: 30, 60, 90 or 120 seconds. Default 60.

That is all. Off by default. Ctrl+F9 stays "Highlight last 30 seconds" during a recording, unchanged.

## How it behaves

1. While the buffer is on, capture and the encoders run even when not recording; nothing is written to disk until Save.
2. The buffer keeps running during a recording. Save works whether or not a take is recording.
3. Save never interrupts the buffer or a running take, and never drops frames.
4. A saved replay is a normal take: same folder layout, files, take number and naming template. It opens in Review and in any editor.
5. Toast after saving: "Replay saved: takeNN" with a Review button. A failed save shows an error toast.
6. Profile fields are locked while the buffer is on (the encoders are running).
7. If the buffer would use more than 25% of free RAM when turned on, a warning toast shows; it still turns on.

## Under the hood

1. RAM rings of encoded packets for every stream a take writes: main video, camera ISO when on, one AAC stream per track and the mixdown. PCM rings per track only when WAV sidecars are on.
2. Rings keep at least the set length and trim only at video keyframes, so a saved clip starts on a keyframe. A clip can be up to one keyframe interval longer than the setting. Audio and camera ISO are cut to the same start.
3. Marks made while the buffer is on are kept for the same span and saved with the clip, plus a Highlight range over the whole clip labelled "Replay".
4. Save copies the rings (reference-counted packets) and a writer thread writes the take through the existing muxer, marks and session code. session.json gets `"capture": "replay"`. Saves made while one is still writing queue up; two saves under 2 s apart count as one.
5. Status bar shows "Replay 60 s" while the buffer is on. Tray icon shows a small amber dot while it is on and nothing is recording.
6. Control API: commands `replay.start`, `replay.stop`, `replay.save`; event `replay.saved` with the take path; status gains `replay` (on or off) and the length.

## Steps (one prompt each)

| Step | Work | Done when |
| --- | --- | --- |
| B1 | Engine: packet, PCM and mark rings with keyframe trimming, and a writer that turns a ring snapshot into a take through the existing muxer, marks and session code. No UI, no state change | Unit tests: trim keeps a keyframe first; snapshot start picks the right keyframe; a clip written from a synthetic ring reads back with the right streams and duration |
| B2 | Controller: buffer on/off independent of recording, encoders run while it is on, rings fed from the same packets a take writes, save queue, hotkeys Ctrl+Alt+B and Ctrl+Alt+S, profile lock, RAM warning | A 10 minute session with five saves, two of them during a recording, gives five playable takes; no dropped frames |
| B3 | UI and API: deck toggle and Save button, Capture menu items, Settings length, status bar, tray dot, toast, Control API commands and event | JD saves replays from the hotkey, the deck and the API |

## Acceptance

1. Five saves give five takes that play in Review and VLC, each about the set length.
2. Audio and camera ISO line up with video within one frame in every clip.
3. Marks made while buffering appear in the right clip; every clip has its Replay Highlight range.
4. RAM stays flat over a 30 minute session with the buffer on.
5. Saving during a recording does not affect the recording.

## Deferred

- Adding the buffered seconds to the front of a recording when it starts.
