# Rust Audio Editor Spec

Oct 12, 2026 · @JD

## Overview

A personal, Windows-only, multitrack audio editor in Rust that works like a video editor: import media into a project bin, place clips on tracks, edit non-destructively, and export a single mixed WAV or MP3.

Guiding principles:

1. **Simplicity first.** Every feature must earn its place. When in doubt, leave it out.
2. **Non-destructive.** Original files are never modified. Clips are references to source media plus in/out points.
3. **Undo-first architecture.** Every change to the project goes through one edit path that records undo history. Nothing mutates project state any other way.
4. **Sample-accurate.** All timeline positions and lengths are stored as integer sample counts at the project sample rate.
5. **Testable core.** The project model and edit operations are pure Rust with no UI or audio dependencies, covered by unit tests.

## Platform and stack

Target: Windows 10 and 11, x64, single executable. Rust stable, edition 2021. Use the latest stable version of each crate at build time.

| Job | Crate | Notes |
| --- | --- | --- |
| GUI | `eframe` / `egui` | Immediate mode. Timeline and waveforms drawn with the egui painter. |
| Audio output | `cpal` | WASAPI backend, default output device. |
| Decode WAV and MP3 | `symphonia` | Features: `wav`, `mp3`, `pcm`. |
| Write WAV | `hound` | 16-bit, 24-bit int, 32-bit float. |
| Encode MP3 | `mp3lame-encoder` | Wraps LAME. CBR only. |
| Resample | `rubato` | High-quality sinc resampler for import conform. |
| Project file | `serde`, `serde_json` | Human-readable JSON. |
| IDs | `uuid` | v4 for media, tracks, clips, markers. |
| File dialogs | `rfd` | Native Windows open, save and folder pickers. |
| App config path | `directories` | Recent projects list in `%APPDATA%`. |
| Threading | `crossbeam-channel`, `arc-swap` | UI to audio commands, lock-free state handoff. |
| Errors | `anyhow`, `thiserror` | `thiserror` in the core, `anyhow` at the app edge. |

## Projects and project management

A project is a self-contained folder. Copy the folder anywhere and it still works.

```
MyProject/
  project.json        project file (the source of truth)
  media/              copies of imported originals, never modified
  cache/              conformed audio + waveform peaks (safe to delete, rebuilt on open)
  autosave/           autosave snapshots
  exports/            default export location
```

**project.json** holds:

- `format_version` (integer, starts at 1)
- Project name, sample rate, default export bit depth
- Media bin entries
- Tracks, clips, markers
- View state: zoom, scroll position, playhead, snapping and ripple toggles

Undo history is not saved. It lives for the session only.

**Start screen** (shown on launch and after closing a project):

1. **New Project:** name, parent folder, sample rate (44.1, 48, 88.2 or 96 kHz, default 48), default export bit depth (16, 24 or 32-bit float, default 24). Sample rate is fixed after creation.
2. **Open Project:** pick a `project.json` or a project folder.
3. **Recent Projects:** last 10, newest first, stored in `%APPDATA%`. Missing ones show greyed out with a Remove option.

**Project menu:** New, Open, Save (Ctrl+S), Save As (copies the whole folder to a new location and name), Close, Project Settings (name and default export bit depth only).

One project open at a time. Closing or quitting with unsaved changes prompts Save, Don't Save, or Cancel.

**Autosave and recovery:**

1. Every 2 minutes, if there are unsaved changes, write `autosave/project.autosave.json`. Never touch `project.json`.
2. Write to a temp file, then rename, so a crash mid-write cannot corrupt anything. Same rule for manual Save.
3. On open, if the autosave is newer than `project.json`, ask: Recover autosave, or Open last saved.
4. A successful manual Save deletes the autosave.

## Media bin

The bin is the project's file manager. Every clip on the timeline points at one bin item.

**Import** (button, menu, or drag files from Explorer onto the bin):

1. Accept `.wav` and `.mp3`. Reject anything else with a message.
2. Copy the original into `media/`. On a name clash, add a suffix: `boom.wav`, `boom (2).wav`.
3. Decode to 32-bit float. If the file's sample rate differs from the project's, resample with `rubato`.
4. Write the result to `cache/<media-id>.f32.wav` (the conformed audio).
5. Build waveform peaks (see Waveform display) and write `cache/<media-id>.peaks`.
6. Run steps 3 to 5 on a background thread with a progress bar per item. The UI stays responsive.
7. Mono stays mono, stereo stays stereo. Files with more than 2 channels are rejected in v1.

**Each bin item shows:** display name, duration, original sample rate, channels, original format, and a usage count (how many clips use it).

**Bin actions:**

- Rename (display name only; the file in `media/` keeps its name)
- Preview: play the item on its own without touching the timeline
- Drag to the timeline to create a clip
- Reveal in Explorer
- Remove from project: if any clips use it, warn with the count; confirming removes those clips too (undoable). The file in `media/` is deleted on the next Save, not immediately, so undo still works.
- Sort by name or date added

**Missing media:** if a file in `media/` is gone on open, its clips draw red with "Media offline" and play silence. Offer Relink, which copies the chosen replacement into `media/`.

**Cache:** if `cache/` is missing or stale, rebuild it from `media/` on open with a progress bar.

## Timeline model

The timeline is plain data: tracks hold clips, clips point at media. No audio samples live in the model, so cloning it is cheap.

```rust
struct Project {
    name: String,
    sample_rate: u32,          // fixed at creation
    export_bit_depth: BitDepth,
    master_volume_db: f32,     // -60.0 to +12.0, default 0.0
    media: Vec<MediaItem>,
    tracks: Vec<Track>,        // top to bottom
    markers: Vec<Marker>,      // sorted by position
}

struct MediaItem {
    id: Uuid,
    display_name: String,
    file_name: String,         // name inside media/
    original_rate: u32,
    channels: u16,             // 1 or 2
    length: u64,               // samples at project rate
}

struct Track {
    id: Uuid,
    name: String,
    volume_db: f32,            // -60.0 to +12.0, default 0.0
    pan: f32,                  // -1.0 (L) to +1.0 (R), default 0.0
    mute: bool,
    solo: bool,
    height: f32,               // UI only
    clips: Vec<Clip>,          // sorted by start, never overlapping
}

struct Clip {
    id: Uuid,
    media_id: Uuid,
    start: u64,                // timeline position, samples
    src_in: u64,               // offset into conformed media, samples
    src_out: u64,              // exclusive; length = src_out - src_in
    gain_db: f32,              // -60.0 to +24.0, default 0.0
    fade_in: u64,              // samples
    fade_out: u64,             // samples
}

struct Marker {
    id: Uuid,
    position: u64,             // samples
}
```

**Rules the model enforces (unit-tested):**

1. Clips on one track never overlap. Placing a clip over another one overwrites it: the clip underneath is trimmed or split around the new one, like an NLE overwrite edit.
2. `src_in < src_out <= media.length`.
3. `fade_in + fade_out <= clip length`. Shortening a clip shrinks its fades if needed.
4. Gaps between clips play as silence.
5. Project length = end of the last clip on any track.

**Selection** lives in editor state, not in `Project`, and is not undoable:

- **Range selection:** a start and end in samples, plus a set of track IDs. Can span any tracks, contiguous or not.
- **Clip selection:** a set of clip IDs, used for move, delete, and clip properties.
- Only one kind is active at a time. Making one clears the other.

**Mono and stereo:** every track outputs stereo. A mono clip feeds both channels equally before track pan.

## Waveform display and timeline UI

**Window layout:**

- Top: menu bar, then a toolbar (transport buttons, Ripple toggle, Snap toggle, Add Track).
- Left: media bin panel, resizable.
- Center: timeline. Ruler across the top, track headers on the left, track lanes to the right.
- Right: master strip with the master fader, master meter, clip light and peak readout.
- Bottom: status bar showing playhead time, selection start / end / length, project sample rate.

**Track header:** name (double-click to rename), volume slider in dB, pan knob or slider, M and S buttons, a stereo peak and RMS meter with clip light, and a drag handle to reorder tracks. Right-click: Rename, Delete Track, Add Track Above / Below.

**Ruler:** timecode as `HH:MM:SS.mmm`, tick spacing adapts to zoom. Markers draw on the ruler as small triangles with a faint line down through all tracks.

**Waveform drawing:**

1. Peaks are min/max pairs per block of samples, stored at 4 levels: 256, 1,024, 4,096 and 16,384 samples per block.
2. At each zoom, draw from the coarsest level that still gives at least one block per pixel.
3. Below 256 samples per pixel, read the conformed samples directly.
4. Stereo clips draw two lanes (L over R) inside the clip. Mono draws one.
5. Waveforms reflect clip gain and fades so you see what you'll hear. Track volume does not change the drawing.

**Zoom and scroll:**

- Horizontal zoom: Ctrl + mouse wheel (centered on the mouse), or `+` / `-`. Zoom to fit: `\`. Zoom to selection: `Ctrl+\`.
- Vertical waveform scaling: Alt + mouse wheel, or a slider in the toolbar. 1x to 16x, display only, never changes the audio.
- Track height: drag the bottom edge of a track header.
- Mouse wheel scrolls tracks vertically; Shift + wheel scrolls the timeline horizontally.
- During playback, the view pages to follow the playhead.

**Clip anatomy and mouse actions:**

| Where on the clip | Click | Drag |
| --- | --- | --- |
| Header strip (top 18 px, shows name and gain) | Select clip (Ctrl+click adds) | Move clip in time and between tracks |
| Body | Set playhead | Range select |
| Left or right edge (6 px) |  | Trim clip in or out |
| Fade handle (small square at each top corner) |  | Set fade length |
| Right-click anywhere | Context menu: Split, Delete, Clip Gain..., Reset Fades, Reveal in Bin |  |

On empty lane space: click sets the playhead and clears selection; drag makes a range selection. On the ruler: click or drag scrubs (see Transport).

**Range selection:**

- Dragging down across lanes adds those tracks to the selection.
- Ctrl+click a track header toggles that track in or out of the current range.
- Shift+click extends the range to the click point.
- Selected range draws as a tinted overlay on selected tracks only, with solid vertical boundary lines at start and end running full height.
- Boundary lines are draggable to adjust the range.

**Snapping** (toolbar toggle, `N` key):

- Snaps to clip edges, markers, and the playhead within 8 px.
- Applies to clip moves, trims, fade handles, range edges, and playhead clicks.
- A highlighted vertical line shows the snap point.
- Holding Alt while dragging bypasses snapping for that drag.

## Transport and scrubbing

| Action | Key | Behavior |
| --- | --- | --- |
| Play / Stop | Space | Plays from the playhead. Stop returns the playhead to where playback started. |
| Pause | P | Freezes the playhead where it is. Play resumes from there. |
| Stop | Stop button | Same as Space while playing. |
| Play selection | Shift+Space | Plays the range selection once, then stops and returns to the range start. All unmuted tracks play; the range sets the time span. |
| Jump to start | Home | Playhead to 0. |
| Jump to end | End | Playhead to project end. |
| Previous / next marker | , and . | Playhead jumps to the marker. |

Play from cursor is the default Play: wherever the playhead sits is where playback starts.

**During playback:**

- Mute, solo, volume, pan and clip gain changes are heard immediately.
- Edits made while playing take effect immediately; playback does not stop.
- Audio buffer target: about 20 ms.

**Audible scrubbing** (drag on the ruler, or drag the playhead handle):

1. Normal playback stops when a scrub starts.
2. Audio plays at a speed that follows mouse movement, forward or backward, with pitch shifting like tape.
3. Speed is clamped to 4x in either direction. Holding still is silent.
4. Reading uses linear interpolation between samples, with a short smoothing ramp so speed changes don't click.
5. On release, the playhead stays where you let go.

## Core editing

All edits change clip references only. Audio data is never rewritten. Every operation below is one undo step.

**Terms used below:** `S` and `E` are the range start and end. "Selected tracks" are the tracks in the range selection. "Target track" is the topmost selected track, or, with no selection, the last track clicked.

**Range operations:**

| Operation | Key | Behavior |
| --- | --- | --- |
| Copy | Ctrl+C | Copies the clip fragments inside `S` to `E` on selected tracks, keeping their positions relative to `S` and their track order. Gaps are copied as gaps. Clipboard length = `E - S`. |
| Cut | Ctrl+X | Copy, then Delete. |
| Delete | Delete / Backspace | Removes content from `S` to `E` on selected tracks. Ripple on: later clips on those tracks shift left by `E - S`. Ripple off: leaves a gap. |
| Paste, with a range selected | Ctrl+V | **Replace.** Removes `S` to `E` on the receiving tracks and puts the clipboard at `S`. Ripple on: later clips shift by the length difference. Ripple off: clipboard overwrites from `S`; if shorter than the range, the rest stays silent; if longer, it overwrites past `E`. |
| Paste, nothing selected | Ctrl+V | **Insert** at the playhead. Clips under the playhead split; later clips on receiving tracks shift right by the clipboard length. Insert always shifts, whatever the Ripple toggle says. |
| Trim to selection | Ctrl+T | Removes everything outside `S` to `E` on selected tracks. Nothing moves. |
| Select All | Ctrl+A | Range from 0 to project end, all tracks. |
| Split | S | Splits clips under the playhead on selected tracks, or all tracks if nothing is selected. |

**Receiving tracks for paste:** clipboard track 1 lands on the target track, clipboard track 2 on the next track down, and so on. If there aren't enough tracks, new ones are created.

**Clip operations** (with clips selected via their header strip):

- **Move:** drag the header. Clips can move in time and across tracks. Multiple selected clips move together, keeping their spacing and track offsets. Dropping onto other clips overwrites them (Timeline model, rule 1).
- **Trim:** drag a clip edge. Can't extend past the start or end of the source media.
- **Delete:** Delete key removes selected clips. Ripple on: each track's later clips close the gap.
- **From the bin:** dragging a bin item onto a track places it as an overwrite. Holding Ctrl while dropping inserts it instead (later clips shift right).

**Ripple toggle:** toolbar button and `R` key. State is saved with the project. It affects Delete, Cut, clip Delete, and Paste-replace length changes. Markers never move with ripple.

## Amplitude processing and levels

All processing is non-destructive. It sets clip gain and clip fades; it never rewrites samples.

**Range processing** (applies to the range on selected tracks; if clips are selected instead, applies to those whole clips):

| Operation | Key | Behavior |
| --- | --- | --- |
| Gain | Ctrl+G | Dialog, -24 to +24 dB. Splits clips at `S` and `E`, then adds the dB value to each fragment's clip gain. |
| Normalize | Ctrl+Shift+N | Dialog with target peak, default -1.0 dBFS. Splits at `S` and `E`, measures the highest peak across all affected fragments (after their clip gain and fades), then sets one gain change so that peak hits the target. Checkbox: "Normalize each track separately" (off by default). |
| Fade in | Ctrl+Shift+I | Splits at `S` and `E`, then sets the fragment starting at `S` to fade in over `E - S`. |
| Fade out | Ctrl+Shift+O | Splits at `S` and `E`, then sets the fragment ending at `E` to fade out over `E - S`. |
| Silence | Ctrl+L | Removes clip content from `S` to `E` on selected tracks. Nothing moves, whatever the Ripple toggle says. |

Fade in and fade out need the range to sit inside one clip per track. If a track's range spans more than one clip, show: "Fade range must be inside a single clip on each track" and do nothing.

**Clip-level controls:**

- **Clip gain line:** a thin horizontal line across the clip body at its gain level. Hover changes the cursor; drag up or down to set gain. The value shows in the header strip (for example `+3.0 dB`). Double-click the value to type one. Range: -60 to +24 dB.
- **Fade handles:** squares at the top-left and top-right corners. Drag inward to set fade length. The fade curve draws over the waveform.
- **Fade curve:** linear gain in v1.

**Track controls:**

- **Volume:** -60 to +12 dB. Double-click resets to 0 dB.
- **Pan:** full left to full right. Double-click resets to center. Constant-power pan law (-3 dB at center).
- **Mute:** silences the track.
- **Solo:** if any track is soloed, only soloed tracks are heard. Ctrl+click Solo makes it exclusive (unsolos the rest).
- Track control changes are undoable. A slider drag counts as one undo step.

**Master fader:**

- Lives in a master strip on the right edge of the timeline, full height.
- -60 to +12 dB. Double-click resets to 0 dB.
- Applied after all tracks are summed, for playback, bin preview and export.
- Saved in the project. Undoable; a drag counts as one undo step.

**Meters:**

| Meter | Where | Measures |
| --- | --- | --- |
| Track | Track header, stereo L/R bars | After track volume, pan, mute and solo |
| Master | Master strip, tall L/R bars | After the master fader (exactly what export writes) |

1. Scale -60 to 0 dBFS, ticks at 0, -6, -12, -18, -24, -36, -48 and -60.
2. RMS is the solid bar, averaged over 300 ms.
3. Peak is a thin line over the bar: rises instantly, falls at 20 dB per second.
4. Peak hold: a marker at the highest recent peak, held for 2 seconds.
5. Clip light: turns red when any sample reaches 0 dBFS and stays lit until clicked. Clicking the master clip light clears every clip light.
6. Master strip shows the highest peak since the last reset as a number in dBFS. Click it to reset.
7. Colors: green below -18 dBFS, yellow from -18 to -6, red above -6.
8. Meters run during playback, play selection, scrubbing and bin preview (master only). They fall to empty when stopped.

**Meter plumbing:**

- The audio thread computes each block's peak and mean square per channel, per track and for the master, and stores them as `f32` bits in `AtomicU32`s.
- The atomics live in a `MeterBank` inside the `MixPlan`. The Editor allocates a new bank when tracks are added or removed, so the audio thread never allocates.
- The UI reads meter values through the Editor every frame and applies the RMS window, peak falloff and hold itself.

**Signal path:** source sample, then clip fade, then clip gain, then mono to stereo (if mono), then track volume, then track pan, then mute and solo, then the track meter, then summed into the stereo mix, then the master fader, then the master meter, then out to the speakers or the export file.

## Undo and redo

Undo is built on whole-project snapshots. The `Project` struct holds no audio, so a snapshot is a cheap clone, and every operation gets undo for free with no per-operation inverse code.

**The one rule:** the UI never mutates `Project` directly. It sends an `Action` to the `Editor`, and the `Editor` is the only code that changes project state.

```rust
struct Editor {
    project: Arc<Project>,      // current state, shared read-only with audio thread
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    gesture_base: Option<Arc<Project>>,
    dirty: bool,
    // plus selection, view, playhead (not undoable)
}

struct HistoryEntry {
    label: &'static str,        // "Move Clip", "Delete", "Normalize"
    before: Arc<Project>,
}

enum Action {
    MoveClips { .. }, TrimClip { .. }, SplitAt { .. },
    DeleteRange { .. }, PasteReplace { .. }, PasteInsert { .. },
    ApplyGain { .. }, Normalize { .. }, FadeIn { .. }, FadeOut { .. },
    SetTrackVolume { .. }, AddMarker { .. }, ImportMedia { .. },
    // one variant per user-facing edit
}
```

**Applying an action:**

1. Clone the current project.
2. Run the matching pure function from `editor::ops` on the clone.
3. Check model invariants (Timeline model rules). On failure, discard the clone, show the error, change nothing.
4. Push the old project onto `undo` with a label. Clear `redo`.
5. Swap in the new project, set `dirty`, and publish the new snapshot to the audio thread.

**Undo** (Ctrl+Z): move current to `redo`, restore the top of `undo`. **Redo** (Ctrl+Y or Ctrl+Shift+Z): the reverse. Menu items show the label: "Undo Move Clip".

**Continuous gestures** (dragging clips, trims, fade handles, sliders, gain line):

1. On mouse down, store `gesture_base`.
2. While dragging, update a live preview without touching history.
3. On mouse up, push one history entry using `gesture_base`.
4. Esc during a drag restores `gesture_base` and records nothing.

**Limits and scope:**

- History is capped at 1,000 steps. Oldest entries drop off.
- Saving does not clear history. Closing the project does.
- Not undoable: selection, zoom, scroll, playhead, Ripple and Snap toggles, bin preview, Save.
- Undoing an import removes the bin item. Media files no longer referenced by the bin or history are deleted from `media/` and `cache/` on Save.

**Testing:** every function in `editor::ops` has unit tests that apply it, check invariants, undo, and confirm the project equals the original.

## Mixdown and export

Export renders all audible tracks into one file. It uses the exact same mixer function as playback, so what you hear is what you get, mute, solo and master fader included.

**Export dialog** (File > Export, Ctrl+E):

| Setting | Options | Default |
| --- | --- | --- |
| Scope | Whole project, or Current selection (disabled when there is no range) | Whole project |
| Selected tracks only | Checkbox, only with Current selection | Off |
| Format | WAV, MP3 | WAV |
| WAV bit depth | 16-bit, 24-bit, 32-bit float | Project default |
| MP3 bitrate | 128, 192, 256, 320 kbps CBR | 192 |
| Channels | Stereo, Mono | Stereo |
| File | Save dialog, starts in `exports/` | `<project name>.wav` or `.mp3` |

**Sample rate:** WAV always exports at the project rate. MP3 exports at the project rate when it is 44.1 or 48 kHz; 88.2 kHz projects export at 44.1, and 96 kHz projects at 48.

**Render steps:**

1. Pass 1: render the mix in memory-sized blocks and track the peak sample after the master fader. Show a progress bar with Cancel.
2. If the peak is above 0 dBFS, show: "The mix peaks at +X.X dBFS and will clip." Buttons: Export Anyway, Cancel. Shown for every format, 32-bit float included.
3. Pass 2: render again and write the file. 16-bit WAV gets TPDF dither. Mono export sums L and R at half gain each.
4. Write to a temp file in the target folder, then rename. Confirm before overwriting an existing file.
5. Scope end: whole project ends at the last clip; selection ends at `E`. No silence is added.

Both passes run on a background thread. The UI stays usable but editing is locked until export finishes or is cancelled.

## Code architecture

One Cargo binary crate split into five sealed layers. Each layer is a top-level module, and calls only flow downward.

&#91;embedded content: layers · 5 sealed modules\]

The Editor is the only gateway: the UI asks it to edit, play, import or export, and it drives the Audio Engine and File I/O.

```
src/
  main.rs                 launch eframe
  app.rs                  eframe::App; start screen vs editor
  model/                  plain data, serde; read by all, written only by editor
    project.rs  media.rs  track.rs  clip.rs  marker.rs  invariants.rs
  editor/                 the only code that changes a Project
    editor.rs  action.rs  history.rs  selection.rs  clipboard.rs  snap.rs
    transport.rs          forwards play / pause / stop / seek / scrub to audio
    jobs.rs               runs import and export on worker threads
    ops/  range.rs  paste.rs  clip.rs  amplitude.rs  track.rs  marker.rs  media.rs
  audio/                  no file access
    engine.rs             cpal stream, owns the callback
    mix_plan.rs           Project + media buffers, handed over by the editor
    mixer.rs              render(plan, start, frames) -> stereo f32; also used by export
    scrub.rs              variable-speed reader
    meters.rs             per-track and master peak / RMS, written to atomics
  io/                     no playback
    decode.rs  resample.rs  peaks.rs  import.rs    media in
    wav.rs  mp3.rs                                 audio out
    project_file.rs  autosave.rs  recent.rs  paths.rs
  ui/                     talks only to editor
    start_screen.rs  menu.rs  toolbar.rs  status_bar.rs  bin_panel.rs  master_strip.rs
    timeline/  ruler.rs  track_header.rs  lane.rs  clip_view.rs  waveform.rs  interaction.rs
    widgets/  meter.rs  fader.rs
    dialogs/  new_project.rs  export.rs  gain.rs  normalize.rs  confirm.rs
  util/
    time.rs               samples <-> timecode
    db.rs                 dB <-> linear
```

**Layer rules:**

1. `ui` calls only `editor`. Play, import, export and meter readings all go through the `Editor`. The UI may read `model` to draw it.
2. `editor` is the only code that writes a `Project`. It calls `audio` and `io`.
3. `audio` never touches files. `io` never plays audio. They never call each other.
4. `model` depends on nothing. `util` holds shared helpers only.
5. Only `ui` and `app` import `egui`. Only `audio` imports `cpal`.

**Threads:**

- **UI thread:** runs the `ui` layer and owns the `Editor`. The Editor builds a new `MixPlan` after every change and publishes it with `ArcSwap`.
- **Audio thread** (the `cpal` callback): reads the latest `MixPlan`, calls `mixer::render`, writes the playhead to an `AtomicU64` and meter values to atomics.
- **Worker threads:** started by `editor::jobs` for import, conform, peaks and export. Report progress and results back to the Editor over `crossbeam-channel`.
- Editor to audio commands (play, pause, stop, seek, scrub target) go over a bounded `crossbeam-channel`.

&#91;embedded content: thread model · 3 threads, 4 handoffs\]

The audio thread never waits on the UI. It reads whatever MixPlan is current and reports the playhead back.

**Audio thread rules (no exceptions):**

1. No memory allocation or freeing.
2. No locks, no file I/O, no logging.
3. No `unwrap()` or anything that can panic.
4. The UI thread keeps replaced `MixPlan`s alive for 2 seconds before dropping them, so the last reference is never released on the audio thread.

**Media in memory:** conformed audio for every bin item is loaded into RAM as `Arc<[f32]>` (interleaved) when the project opens. One hour of stereo at 48 kHz is about 1.4 GB, which sets the practical project size for v1.

## Keyboard shortcuts

Shortcuts only fire when no text field has focus.

| Group | Key | Action |
| --- | --- | --- |
| Project | Ctrl+N / Ctrl+O / Ctrl+S | New / Open / Save project |
| Project | Ctrl+Shift+S | Save As |
| Project | Ctrl+I | Import media |
| Project | Ctrl+E | Export |
| Transport | Space | Play / Stop |
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
| View | \\ / Ctrl+\\ | Zoom to fit / zoom to selection |
| View | Ctrl+wheel / Alt+wheel | Horizontal zoom / vertical waveform scale |
| Track | Ctrl+Shift+T | Add track |

## Build phases

Each phase ends with something you can run and check. Don't start a phase until the previous one passes its check. Each phase becomes one or more Cline prompts.

1. **Skeleton.** Cargo project, module folders, eframe window with empty panels, start screen, New / Open / Save of an empty project folder, recent projects list.
   - Check: create a project, close, reopen it from Recent.
2. **Model and undo core.** `model/` structs, invariants, `Editor`, `Action`, history, and the `editor::ops` functions for clips, ranges, paste, markers. No UI wiring yet.
   - Check: `cargo test` passes, including apply / undo / compare tests for every op.
3. **Media import.** Copy to `media/`, decode, resample, cache, peaks, bin panel with all bin actions except Preview.
   - Check: import a 44.1 kHz MP3 into a 48 kHz project; it shows the right duration in the bin.
4. **Timeline view.** Ruler, track headers, lanes, clips drawn from the model, waveforms, zoom, vertical scaling, scroll, drag from bin to track.
   - Check: drop three clips on two tracks and zoom from the whole project down to single samples.
5. **Playback.** Audio engine, `MixPlan`, mixer, transport keys, playhead following, bin Preview.
   - Check: plays all tracks in sync with no clicks; Space and Shift+Space behave as specced.
6. **Editing.** Range selection across tracks, clip selection, move, trim, split, cut / copy / paste (insert and replace), delete, trim to selection, select all, ripple toggle, snapping, Esc cancel.
   - Check: every edit undoes and redoes cleanly while playing.
7. **Levels and processing.** Track volume, pan, mute, solo; master fader; track and master peak plus RMS meters; clip gain line and fade handles; Gain, Normalize, Fade in / out, Silence.
   - Check: normalize a quiet clip to -1 dBFS and confirm the peak by ear and on the waveform.
8. **Markers and scrubbing.** Add, move, delete, jump; audible scrubbing on the ruler and playhead.
   - Check: scrub backward over speech and hear it reversed.
9. **Export.** Export dialog, two-pass render, clipping warning, WAV and MP3 writers, mono option.
   - Check: export, re-import the result into a new track, and confirm it lines up and sounds the same.
10. **Safety net and polish.** Autosave and recovery, missing media and relink, unsaved-changes prompts, unused media cleanup on Save.
    - Check: kill the app mid-edit; reopening offers the autosave.

## Out of scope and defaults to review

**Not in v1:**

- File formats other than WAV and MP3 (no AAC, FLAC, OGG); files with more than 2 channels
- Recording
- Effects, plugins, real-time FX
- Volume or pan automation envelopes
- Crossfades between overlapping clips (overlaps overwrite instead)
- Named markers
- Spectral view, time stretch, pitch shift
- Batch processing
- More than one project open at a time
- macOS and Linux

**Defaults I picked that you haven't confirmed.** Change any of these before we write prompts.

| Area | Default |
| --- | --- |
| Stop vs Pause | Stop returns the playhead to where playback began; Pause leaves it in place |
| Paste with no selection | Always inserts and shifts later clips, even with Ripple off |
| Overlapping clips | New clip overwrites the one underneath |
| Ripple and markers | Markers never move |
| Fade curve | Linear |
| Normalize | Target -1.0 dBFS, all affected tracks treated as one |
| New project | 48 kHz, 24-bit export default |
| MP3 export | 192 kbps CBR default |
| Autosave | Every 2 minutes |
| Undo history | 1,000 steps |
| Snapping | 8 px, Alt bypasses during a drag |
| Recent projects | Last 10 |
| Media size | All conformed media held in RAM, about 1 hour of stereo per project |

**Markers** (position only, no names):

- Add at playhead: `M`. Works during playback.
- Move: drag the triangle on the ruler. Snaps when snapping is on.
- Delete: click a marker to select it, then Delete. Or right-click it > Delete.
- Jump: `,` previous, `.` next.
- Adding, moving and deleting markers are undoable.
