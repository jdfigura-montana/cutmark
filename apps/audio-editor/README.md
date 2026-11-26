# CutMark Audio Editor

Version 1.2.0

A small, keyboard-driven multitrack audio editor for Windows built in Rust with egui. It opens
project folders (media, cache and project.json live together), imports WAV and MP3 files, and lets
you arrange clips on any number of tracks with waveform drawing, zooming, scrubbing and markers.
Playback runs on a real-time cpal engine with sample-accurate mixing, per-track gain/pan/mute/solo,
a master fader with peak and RMS meters, and clip-level gain lines and fades. Non-destructive edits
(split, trim, move, cut/copy/paste, gain, normalize, fades, silence) are all undoable, and projects
autosave with recovery on reopen. Finished audio can be exported to WAV or MP3 with a clipping
warning pass.

Recording: arm a track, press Ctrl+R and record from the input device straight onto the timeline;
each take lands in the bin and on the armed track as one undo step.

## Build

```
cargo build --release -p cutmark-audio
```

The executable ends up at `target\release\cutmark-audio.exe` in the workspace root (two folders
above this one). Release builds are windowed — no
console window opens, and the C runtime is linked statically (no VC++ redistributable needed).

## Build the installer

```
powershell -ExecutionPolicy Bypass -File apps\audio-editor\scripts\build-installer.ps1
```

Requires [Inno Setup 6](https://jrsoftware.org/isinfo.php) (ISCC.exe) to be installed. The script
refuses to run while CutMark is open, builds the release exe, runs Inno Setup with the version from
`apps\audio-editor\Cargo.toml` and writes `dist\CutMark-Setup-<version>.exe`. The installer bundles the exe plus the FFmpeg DLLs
(beside the exe) and a `licenses\` folder (third-party licences for the Instrument Sans and Martian Mono fonts, the Phosphor icons and the FFmpeg
DLLs). The
icons can be regenerated with `python scripts\make_icon.py`.

## Keyboard shortcuts

Shortcuts only fire when no text field has focus.

| Group | Key | Action |
| --- | --- | --- |
| Project | Ctrl+N / Ctrl+O / Ctrl+S | New / Open / Save project |
| Project | Ctrl+Shift+S | Save As |
| Project | Ctrl+I | Import media |
| Project | Ctrl+E | Export |
| Transport | Space | Play / Stop |
| Transport | Ctrl+R | Record onto the armed track (Space keeps the take, Esc discards it) |
| Transport | Shift+Space | Play selection |
| Transport | P | Pause |
| Transport | Home / End | Jump to start / end |
| Markers | M | Add marker at playhead |
| Markers | , / . | Previous / next marker |
| Edit | Ctrl+Z / Ctrl+Y | Undo / Redo (Ctrl+Shift+Z also redoes) |
| Edit | Ctrl+X / Ctrl+C / Ctrl+V | Cut / Copy / Paste |
| Edit | Delete / Backspace | Delete |
| Edit | Ctrl+T | Trim to selection |
| Edit | Ctrl+A | Select All |
| Edit | S | Split at playhead |
| Edit | Esc | Clear selection, or cancel a drag |
| Process | Ctrl+G | Gain |
| Process | Ctrl+Shift+N | Normalize |
| Process | Ctrl+Shift+I / Ctrl+Shift+O | Fade in / Fade out |
| Process | Ctrl+L | Silence |
| Toggles | R | Ripple on / off |
| Toggles | N | Snap on / off |
| View | + / - | Zoom in / out |
| View | \ / Ctrl+\ | Zoom to fit / zoom to selection |
| View | Ctrl+wheel / Alt+wheel | Horizontal zoom / vertical waveform scale |
| Track | Ctrl+Shift+T | Add track |
