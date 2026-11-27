# Recorder Phase 12: Replay buffer (spec)

Nov 26, 2026 · @JD · draft 1

Exported from the Claude Project "Creator Suite" ("Recorder Phase 12 Replay Buffer Spec"). The Project copy is the source. Built from item 2 of the Cutmark Suite Additions Plan.

## Overview

"Save the last N seconds" without recording the whole session. A new state, Buffering, runs capture and the encoders but writes nothing to disk. One hotkey saves the buffer as a normal take with its marks, and the buffer keeps running.

**Goals**

1. A saved replay is a normal take: same folder layout, same files, opens in Review and in any editor.
2. Saving never interrupts the buffer and never drops frames.
3. RAM use is predictable and shown before the user turns it on.

**Non-goals**

- No prepending the buffer to a full recording in v1 (Deferred).
- No instant replay playback inside the Recorder beyond Review.

## States

| From | Action | To |
| --- | --- | --- |
| Idle | Start replay buffer | Buffering |
| Buffering | Save replay | Buffering (a take is written in the background) |
| Buffering | Stop replay buffer | Idle (the buffer is dropped) |
| Buffering | Start recording | Recording (the buffer pauses: it neither fills nor saves) |
| Recording | Stop | Buffering again if it was Buffering before, else Idle |

Rules:

1. Pause is not available while Buffering.
2. Profile fields stay locked while Buffering, as while Recording, because the encoders are running.
3. Off by default. Settings > Recording has "Start the replay buffer when the Recorder opens" (off by default).

## The buffer

1. RAM rings of encoded packets for every stream the take would write: main video, camera ISO (when on), one AAC stream per track and the mixdown. Packets keep their timestamps and the keyframe flag.
2. When WAV sidecars are on, a PCM ring per track as well (48 kHz, the track's channel count).
3. Length setting: 30 s, 60 s (default), 2 min, 5 min. The ring keeps at least that much, trimmed only at keyframes, so the oldest packet in the video ring is always a keyframe.
4. Marks made while Buffering (any source: hotkeys, API, Prompter) are kept with their times; ones older than the ring are dropped.
5. Memory estimate shown next to the length setting: video bitrate times length (max bitrate for constant quality modes), plus camera ISO, plus about 1.5 MB per minute per AAC stream, plus about 23 MB per minute per track for WAV rings. Example: 60 s at 40 Mbps max is about 300 MB.
6. If the estimate is more than 25% of free RAM when Buffering starts, a warning toast shows; it still starts.

## Save

1. Save replay copies the rings (cheap: reference-counted packets) and hands the copy to a writer thread; capture and encoding go on untouched.
2. The clip starts at the newest video keyframe at or before (now minus the length), so a clip can be up to one keyframe interval longer than the setting. Audio and camera ISO are cut to the same start time. The clip ends at the moment Save was pressed.
3. The take folder, file names and take number follow the naming template like any take. session.json gets `"capture": "replay"` and the replay length; the composite block is written as usual.
4. Marks: every kept mark in the clip's span, plus a Highlight range covering the whole clip, labelled "Replay". The full marks sidecar, md, csv and MKV chapters are written as for a normal take.
5. Saving while a save is still writing queues the second save (both clips are written). Two saves less than 2 s apart are treated as one.
6. A toast "Replay saved: takeNN" with a Review button. A failed save shows an error toast and a System mark is not possible (no take), so the error goes to the log and the banner strip.

## Hotkey and controls

1. Ctrl+F9 (today's Highlight last 30 seconds) is Save replay while Buffering, and stays Highlight last while Recording. One hotkey, two states.
2. New suite hotkey (owner recorder): Start or stop replay buffer, default Ctrl+Alt+B.
3. Capture menu: Start or stop replay buffer, Save replay.
4. Deck (Setup and Record modes): a Replay toggle button; while Buffering, a "Save replay" capsule in sem.record tint next to Record, and the caption "Replay 60 s".
5. Status bar state: "Buffering · 60 s · ~300 MB". Tray icon: amber while Buffering.
6. Control API: commands `replay.start`, `replay.stop`, `replay.save`; event `replay.saved` with the take path; status gains `replay` (off, buffering, paused) and the length.

## Steps (one prompt each)

| Step | Work | Done when |
| --- | --- | --- |
| B1 | Engine: packet rings with keyframe-aware trimming, PCM rings, mark ring, and a writer that turns a ring snapshot into a take through the existing muxer, marks and session code. No UI, no state change yet | Unit tests: trim keeps a keyframe first; snapshot start picks the right keyframe; a written clip from a synthetic ring opens with ffprobe-like checks in tests |
| B2 | Controller: Buffering phase, transitions from the table, encoders kept running, save queue, Ctrl+F9 dual use, new hotkey, session.json capture field, toasts | A 10 minute Buffering session with five saves gives five playable takes; no dropped frames in the stats |
| B3 | UI and API: Settings length and start-on-open, memory estimate, deck controls, menu items, status bar, tray state, Control API commands and event | JD saves replays from the hotkey, the deck and the API |

## Acceptance

1. Five saves in a 10 minute Buffering session give five takes that play in Review and VLC, each about the set length (up to one keyframe interval longer).
2. Audio and camera ISO line up with video within one frame in every saved clip.
3. Marks made during Buffering appear in the right clip at the right time; every clip has its Replay Highlight range.
4. RAM stays flat over a 30 minute Buffering session (no growth beyond the estimate plus 10%).
5. Ctrl+F9 saves while Buffering and marks while Recording.

## Deferred

- Prepend the buffer to a full recording when one starts from Buffering.
- Replay length per profile.
