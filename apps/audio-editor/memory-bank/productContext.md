# Product Context

## Why
CutMark Audio Editor is a personal audio editor with an NLE (video-editor) workflow: media bin, tracks, non-destructive clips, final mixdown export. Simplicity first — every feature must earn its place (SPEC: "Overview").

## Problems it solves
- Multitrack editing on Windows with familiar NLE interactions (drag from bin, trim/split, ripple, snapping).
- Non-destructive edits with full undo/redo (1,000 steps, session only).
- Sample-accurate positioning (u64 sample counts, never float seconds).
- Export to WAV or MP3 from one mixed render.

## How it should work (UX shape — SPEC: "Waveform display and timeline UI" → "Window layout")
- Top: menu bar (Project menu) + toolbar (transport, Ripple/Snap toggles, Add Track).
- Left: resizable Media Bin. Center: timeline (ruler, track headers, lanes). Right: fixed 90 px Master strip. Bottom: status bar (playhead time, selection, sample rate).
- Start screen on launch / after close: New Project, Open Project, Recent Projects (last 10 in %APPDATA%).
- Space = play/stop; overwrite-style clip placement; 8 px snapping (Alt bypasses); Ctrl+wheel zoom; audible 4x-max scrubbing; M adds a marker.

## Experience goals
- Snappy immediate-mode UI (egui/eframe).
- Every edit undoable; edits while playing apply immediately.
- Conformed media held in RAM (~1.4 GB per hour of stereo) — practical project size is about an hour.