# Cutmark Recorder: v1.2 Spec

Oct 26, 2026 · @JD

**Changes from v1.1** (Nov 14, after S0 and the Marks Sidecar spec):

1. Windows 11 only, like the rest of the suite.
2. The UI runs on eframe with the wgpu renderer, the one renderer for the whole suite. The compositor stays on D3D11. The preview reaches egui as a low-resolution copy at 15 fps.
3. FFmpeg is the pinned LGPL build from S0 (9.0.2). On Nov 14 it was confirmed to carry `av1_amf`, `hevc_amf`, `h264_amf`, the NVENC and QSV encoders, `libsvtav1` and `aac`, and `av1_amf` encoded 1440p60 at 3.6x realtime on the RX 7700 XT. No source build is needed for v1.
4. Marks follow the Marks Sidecar Schema v1 spec: `marks::SessionWriter`, a journal flushed per mark, recovery on launch, `recorder.*` system marks, and retroactive ranges. Point marks are never backdated.
5. Global hotkeys go through `cutmark-hotkeys` and the suite hotkey map.
6. Settings live in `%APPDATA%\Cutmark\recorder\`. The API token lives in Windows Credential Manager.
7. Prompter link: `countdownStarted` and `countdownCancelled` events, a hide-countdown setting, the `scriptPosition` command, script take files, and a Script column in the marks log.
8. One file per take. No rollover.
9. The build follows the S0 workspace rules (CLAUDE.md, strict lints in new crates, `scripts\check.ps1`, static CRT, Inno Setup installer).

**Changes from v1** (to match the Cutmark Creator Suite overview):

1. Hardware encoding on AMD, Nvidia and Intel, with SVT-AV1 as the software fallback.
2. Vendor-neutral rate control and profiles.
3. UI moves from Tauri and React to egui, sharing the suite's `ui-kit`.
4. Code lives in the suite workspace: capture and encoding in shared crates, the app in `apps/recorder`.
5. Marks write the suite sidecar, `{name}.marks.json`, and gain Highlight and Chapter types.
6. Built for commercial release: LGPL FFmpeg only, no OBS plugin.

## Overview

The Cutmark Creator Suite's Windows desktop recorder: record only, no streaming, no scenes, no plugins. It records the screen with hardware AV1 on AMD, Nvidia or Intel, a chroma-keyed webcam, multiple audio tracks, and marks that drive editing.

**Goals**

- Hardware AV1, HEVC and H.264 on AMD, Nvidia and Intel, with SVT-AV1 software fallback and full rate control.
- Webcam with chroma key and virtual background, using a physical green screen.
- Any audio device on any track, including Wave Link 3 mixes.
- Hotkey marks written to the suite sidecar, plus a log you fill out before editing.
- Files that drop straight into the suite's editors, LUXEA Pro 8, and Premiere (with a Premiere-safe profile).
- Every action reachable by hotkey and by a local API.

**Non-goals**

- Streaming, scenes, plugins, AI background removal.
- Auto-remuxing. The file stays in the container you chose.
- A native OBS plugin. OBS users get the Stream companion (suite milestone S3), which uses obs-websocket.
- macOS or Linux.

## Platform and tech stack

Rust on Windows 11, inside the Cutmark workspace. Capture and encoding live in shared crates so TuteTool can reuse them; the app never touches them except through its engine module.

- **Target:** Windows 11 with any D3D11 GPU. Developed on an RX 7700 XT; tested on Nvidia and Intel per the suite test rig.
- **Code layout:**
  - `crates/capture`: D3D11 device, Windows Graphics Capture, WASAPI, Media Foundation webcam. No UI.
  - `crates/media-io`: FFmpeg wrapper, `Encoder` trait and its four backends, muxers. No UI.
  - `crates/marks`: sidecar schema, md and csv writers.
  - `crates/ui-kit`: the suite's dark theme, meters and faders.
  - `apps/recorder`: the egui app, plus an `engine` module (compositor, pacer, command bus, take folder, API server) with no egui imports.
- **Key crates:** `windows` (D3D11, WASAPI, Media Foundation, Win32, Credential Manager), `windows-capture`, `wasapi`, `ffmpeg-next` 9 against the pinned LGPL DLLs in `third_party/ffmpeg`, `rubato`, `hound`, `eframe` (wgpu renderer) and `egui`, `cutmark-hotkeys` (wraps `global-hotkey`), `marks`, `tray-icon`, `tokio-tungstenite`, `serde`.

&#91;embedded content: recorder pipeline · 4 inputs, 1 take folder\]

Screen and webcam meet in the compositor; the raw webcam and the audio also branch straight to their own files in the take folder.

**Workspace rules (from S0).** New crates (`capture`, `media-io`, `cutmark-hotkeys`, the app) use the strict lint set. `build.rs` copies the FFmpeg DLLs next to the exe, as the audio editor does. Done means `scripts\check.ps1` passes plus a release build. The installer is Inno Setup and ships the DLLs, the LGPL and GPL texts and `BUILDINFO.txt`.

## Screen capture

Screen frames come from Windows Graphics Capture as D3D11 textures and never leave the GPU.

- **API:** Windows Graphics Capture via the `windows-capture` crate.
- **Sources:** full monitor, single window, or custom region. Region is drawn with a drag overlay that snaps to window edges and remembers the last region.
- **Cursor:** show or hide. Capture border: off.
- **Frame rate:** 24, 30, 60, or custom up to 120.
- **Output scaling:** capture resolution to output resolution on the GPU (for example 2160p to 1080p), bilinear or Lanczos.
- **Self-exclusion:** every app window, popup, and overlay calls `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`, so none of them appear in recordings.
- **Constant frame rate:** enforced by the pacer (see Sync and reliability rules).

## Audio capture and tracks

Every track picks any Windows audio device, so Wave Link 3 mixes and channels show up like any other source.

**Tracks**

- 2 tracks by default, up to 6.
- Defaults: Track 1 = an output device via loopback (for example a Wave Link mix), Track 2 = mic.
- Each track can use any capture device (mics, Wave Link virtual inputs) or any render device through WASAPI loopback.
- Devices are saved by Windows endpoint ID, with the device name as fallback. A Refresh button re-lists devices.

**Per-track controls**

- Name, gain (dB), mute, and an optional mute hotkey.
- Live peak meter with a clip indicator.
- Sync offset from -500 to +500 ms.

**Format**

- All tracks resampled to 48 kHz stereo. Mono sources become dual mono.
- In the video file: AAC-LC 320 kbps per track. PCM 24-bit is an option for MKV and MOV. Never Opus.
- **Mixdown track:** an optional first audio stream that sums all tracks, on by default. Editors that play one stream at a time get the full mix.
- **WAV sidecars:** one 48 kHz 24-bit WAV per track, on by default. The header is refreshed every 5 seconds so a crash leaves a usable file.

## Webcam, chroma key, compositor

The webcam is keyed live on the GPU and composited into the main video, while the raw camera is always saved to its own file for re-keying in post.

**Camera**

- Capture via Media Foundation. Pick device, resolution, frame rate, and format (MJPEG or NV12).
- **Locked camera settings:** exposure, white balance, focus, gain, brightness. Set manually in the app, with a "Lock current values" button. Saved per profile.

**Chroma key (HLSL shader)**

- Key color picked with an eyedropper on the preview.
- Similarity, smoothness, and spill suppression sliders.
- Matte view toggle (black and white) for tuning.
- **Garbage matte:** crop left, right, top, bottom, plus an optional mask rectangle to cut stands and screen edges.

**Virtual background**

- None (keyed cam floats over the screen), image (PNG or JPG), looping video (MP4 or MKV), or solid color.

**Layouts**

- **PiP:** cam over screen. Anchor corner, X and Y offset, scale %, crop, optional border and rounded corners.
- **Cam only:** keyed cam over the virtual background, full frame.
- **Screen only.**

**Outputs**

- Composite goes into the main video.
- Raw, unkeyed webcam ISO file is written whenever the webcam is on, using the same codec family at the camera's native resolution.

**Compositor**

- Direct3D 11 with HLSL shaders, on the same D3D11 device as capture and the encoder, so frames never copy to the CPU. This replaces the earlier `wgpu` idea.
- Does keying, layout, output scaling, and conversion to NV12 for the encoder.
- Preview gets a low-res copy at 15 fps.

## Encoding and rate control

All video encoding goes through the `Encoder` trait in `media-io`, AV1 by default, on whichever vendor's hardware the machine has.

The DLLs are the pinned S0 build (FFmpeg 9.0.2, LGPL, fetched by `third_party/ffmpeg/get-ffmpeg.ps1`). It already includes the AMF, NVENC, QSV and SVT-AV1 encoders and FFmpeg's native AAC encoder, so the Recorder needs no build of its own.

- **Library:** FFmpeg via `ffmpeg-next`, an LGPL build with AMF, NVENC, QSV (oneVPL) and SVT-AV1 enabled.
- **Backends and codecs:**
  - AMD: `av1_amf`, `hevc_amf`, `h264_amf`
  - Nvidia: `av1_nvenc`, `hevc_nvenc`, `h264_nvenc`
  - Intel: `av1_qsv`, `hevc_qsv`, `h264_qsv`
  - Software: `libsvtav1`, AV1 only
- **Probe:** at startup, open each encoder at the profile's resolution and frame rate; hide the ones that fail. Option names and ranges are read from the encoder, never hardcoded.
- **Adapter:** the encoder runs on the same GPU as the D3D11 device. On machines with two GPUs, the device is created on the adapter driving the captured monitor, and only that vendor's encoders are offered.
- **Default:** the first working AV1 encoder on that adapter; if none, SVT-AV1 with a banner saying it uses the CPU.
- **Frames:** D3D11 hardware frames on the shared device, 8-bit NV12. QSV maps them through FFmpeg's D3D11-to-QSV derivation. SVT-AV1 is the only path that copies frames to the CPU.
- **Presets:** speed, balanced, quality, mapped to each backend's own preset names.
- **Keyframe interval:** seconds, default 2.
- **Webcam ISO:** its own encoder session on the same backend, with its own rate control settings.

| UI mode | Settings exposed | AMD | Nvidia | Intel | Software |
| --- | --- | --- | --- | --- | --- |
| Constant quality | Quality 1 to 100, max bitrate (Mbps) | QVBR | VBR with a CQ target | ICQ (max bitrate ignored) | CRF |
| Constant QP | QP for I and P frames | CQP | Constant QP | CQP | Fixed QP |
| CBR | Bitrate (Mbps) | CBR | CBR | CBR | Hidden |
| VBR with peak | Target and max bitrate (Mbps) | Peak-constrained VBR | VBR with max bitrate | VBR | Capped CRF |

Profiles store the codec, the UI mode and vendor-neutral values. Each backend maps the 1 to 100 quality value to its native scale and shows the native number beside the slider, so a profile made on AMD opens on Nvidia or Intel unchanged.

Vendor high-quality variants (AMF HQ VBR and HQ CBR, NVENC multipass) appear as a High quality toggle when the encoder exposes them.

## Containers and output files

The file is written in the container the profile chose and is never remuxed.

- **MKV:** any codec. Gets chapters from marks.
- **MP4:** written fragmented, so a crash leaves a playable file.
- **MOV:** written fragmented. HEVC and H.264 only.

Each take gets its own folder:

```
{root}/{project}/{date}_{time}_take{nn}/
  {name}.mkv               main video (one file per take, no rollover)
  {name}_cam.mkv           raw webcam ISO
  {name}_t1_{track}.wav    one per audio track
  {name}.marks.json        suite sidecar, written by marks::SessionWriter on stop
  {name}.marks.journal     live journal while recording; removed after a clean stop
  {name}_marks.md          marks log to fill out
  {name}_marks.csv         same marks, spreadsheet-ready
  {name}_script.md         Prompter link only: the script at the first scriptPosition
  {name}_script.csv        Prompter link only: ms, line, token, text
  {name}_session.json      settings, devices, encoder stats
```

`session.json` is written at start and updated on stop with duration, frame counts, duplicated and dropped frames, and average bitrate.

## session.json composite block

Every take's `session.json` has a `composite` object, written at start, that says what is in the main video and how an editor rebuilds the Baked look: canvas, main file, camera layer and background. The schema and field rules are in [recorder-composite-mode.md](recorder-composite-mode.md), section "The `composite` block in session.json".

- File names are relative to the take folder. A Baked camera layer with no camera file has `"file": null`.
- `layers` is empty in the Screen only layout, when the webcam is off or absent at start, and in a Clean take whose camera file could not start.
- In Cam only, `main.source` stays `screen`, the camera layer is baked, and its crop is 0 because that layout shows the whole camera frame.

## Marks system

A hotkey stamps the exact recording time, and every mark reaches disk the moment it is made. The format and the time rules are the Marks Sidecar Schema v1 spec; the Recorder writes only through \`marks::SessionWriter\` and never builds the JSON itself.

**Hotkeys**

- **F9:** silent mark, category Note.
- **Shift+F9:** mark plus a quick popup. The time is captured at the keypress, not when the popup closes.
- **Ctrl+F9:** retroactive Highlight, stored as a range that ends at the keypress and covers the last 30 seconds (length configurable). Point marks are never backdated; pre-roll is an editor setting.

**Popup**

- Small, always on top, excluded from capture. Recording keeps running.
- Keys 1 to 6 pick a category: Highlight, Cut, Retake, Chapter, Note, B-roll.
- Type a label. Enter saves, Esc saves with no label.

**Timecode**

- Recording time with paused time removed.
- Stored in milliseconds, shown as `HH:MM:SS:FF` at the recording frame rate.

**Automatic marks** (type `system`): `auto.kind` is `recorder.pause`, `recorder.resume`, `recorder.device_lost` or `recorder.encoder_overload`. Pause and resume land on the same `t_ms`, because paused time is not in the file.

**Outputs**

- `{name}.marks.json` through `marks::SessionWriter`: `begin` at start, `add_mark` per mark into `{name}.marks.journal` (flushed per mark), `finish` on stop writes the sidecar atomically. On launch, a leftover journal is recovered into a sidecar marked `recovered: true`.
- `_marks.md` and `_marks.csv` from the `marks` crate's export module, appended and flushed per mark. With a Prompter linked, both gain a Script column (line number and the first 60 characters of the line).
- MKV chapters from the `marks` crate's chapter list, written on stop, named `Category: label`.

```markdown
# Take 03, 2026-10-25, Tutorial AV1 1440p

| # | Timecode    | Category | Label           | Notes |
|---|-------------|----------|-----------------|-------|
| 1 | 00:01:12:24 | Note     | intro done      |       |
| 2 | 00:04:03:10 | Retake   | flubbed command |       |
| 3 | 00:06:30:00 | System   | paused          |       |
```

CSV columns: `index, timecode, ms, category, label, notes`, plus `script` when a Prompter is linked.

## Profiles, naming, settings

A profile holds every capture, encode, audio, webcam, and layout setting, and switches only while idle.

**Shipped profiles**

| Profile | Codec | Output | Container | Audio in file |
| --- | --- | --- | --- | --- |
| Tutorial AV1 1440p60 | AV1, constant quality | 2560x1440, 60 fps | MKV | AAC + mixdown |
| Premiere HEVC 1080p60 | HEVC, VBR with peak | 1920x1080, 60 fps | MP4 | AAC + mixdown |
| Cam only AV1 | AV1, constant quality | 1920x1080, 30 fps | MKV | AAC + mixdown |

**Naming template**

- Tokens: `{project}`, `{date}`, `{time}`, `{take}`, `{profile}`.
- Take number auto-increments per project per day.
- Root folder picked in settings.

**Storage**

- App settings: `%APPDATA%\Cutmark\recorder\settings.json`, located through the `directories` crate.
- One JSON file per profile in `%APPDATA%\Cutmark\recorder\profiles\`, easy to back up or copy. Hotkeys live in the suite file, `%APPDATA%\Cutmark\hotkeys.json`.

## UI, hotkeys, monitoring

One egui window on eframe's wgpu renderer, in the suite's dark theme from `ui-kit`, with the preview, meters and controls, plus a tray icon (`tray-icon`) while recording. Global hotkeys go through `cutmark-hotkeys`; the window handle gets `WDA_EXCLUDEFROMCAPTURE` at creation.

**Main window**

- Source picker, low-res preview, per-track meters, layout controls, record controls, status bar.
- Minimizes to tray during recording. Tray icon shows recording state.
- All app windows are excluded from capture.

**Default hotkeys** (all configurable, global, owned by the Recorder in the suite hotkey map)

| Action | Default |
| --- | --- |
| Start / stop | Ctrl+Alt+R |
| Pause / resume | Ctrl+Alt+P |
| Silent mark | F9 |
| Mark with label | Shift+F9 |
| Highlight the last 30 seconds | Ctrl+F9 |
| Mute mic track | Ctrl+Alt+M |

**Status bar while recording**

- Elapsed time, file size, current bitrate.
- Disk free and time remaining at the current data rate.
- Duplicated and dropped frame counters.
- Encoder queue depth.

**Warnings**

- Disk under 10 GB or under 15 minutes left: banner and sound.
- Disk under 2 GB: clean auto-stop.
- Dropped frames over 0.5% across 10 seconds: banner plus a System mark.

**Countdown**

- Off, 3, or 5 seconds. Overlay excluded from capture. Esc cancels.

**Pre-flight test**

- One button records 5 seconds with the current profile to a temp folder.
- Reports dropped frames, average encode time against the frame budget, and peak dBFS per track, with a pass or fail.
- Also shows a reminder to turn on Do Not Disturb (Win+N) before recording, because Windows has no public API that reports it; the Recorder never toggles the setting.
- Temp files deleted unless you choose to keep them.

## Local control API

Every hotkey, button, and API call goes through one internal command bus, and a local WebSocket server exposes that bus.

- **Bind:** 127.0.0.1 only, port 4466 (configurable).
- **Auth:** a token generated on first run and stored in Windows Credential Manager, shown and regenerable in settings, sent in the first message.
- **Off switch:** API can be disabled in settings.
- **Clients:** Stream Deck (any generic WebSocket plugin), the teleprompter app, scripts.

**Commands:** `start`, `stop`, `pause`, `resume`, `mark`, `mute`, `setProfile`, `getStatus`, `scriptPosition`.

**Events:** `countdownStarted { seconds }`, `countdownCancelled`, `recordingStarted`, `recordingStopped`, `paused`, `resumed`, `markAdded`, `warning`, plus `status` once per second while recording.

**Prompter link** (the Prompter v1 spec's required Recorder changes)

- Setting "Hide my countdown when a prompter is linked", default on, so only one countdown shows.
- `scriptSnapshot { scriptId, title, markdown }` (markdown up to 2 MB) sends the script once; the Recorder keeps the latest per `scriptId` and writes it into `{name}_script.md` at the first `scriptPosition`.
- `scriptPosition { scriptId, title, token, line, text }` arrives at most 10 times a second. The Recorder stamps it with its own clock.
- The first `scriptPosition` of a take writes `{name}_script.md`, a snapshot of the script. Every one appends a row to `{name}_script.csv` (`ms, line, token, text`), flushed per row.
- Every mark made while a script is linked carries the current line in the Script column of `_marks.md` and `_marks.csv`.
- A `mark` sent by the Prompter (flub hotkey, `[[mark: ...]]` cue) is stored with source `api`.

```json
{ "op": "mark", "id": 7, "data": { "category": "Retake", "label": "redo intro" } }
{ "event": "markAdded", "replyTo": 7, "data": { "index": 4, "timecode": "00:02:41:12" } }
```

## Sync and reliability rules

One clock (QPC) timestamps everything, and the video is always written at an exact constant frame rate.

**Clock and frame rate**

1. Every video frame and audio buffer is stamped with QPC on arrival. Recording time 0 = the moment Start fires.
2. The pacer ticks at the exact profile frame rate from time 0.
3. Each tick sends the newest composited frame to the encoder. No new frame: the last one is repeated and counted as duplicated.
4. Encoder queue full: the frame is dropped and counted.

**Audio**

- Each device is resampled to 48 kHz with small ratio adjustments to stay locked to QPC (drift correction).
- Per-track offsets shift audio by the set ms. Webcam offset delays camera frames through a small buffer.

**Pause**

- Pause stops feeding the pacer and audio writers. Resume continues timestamps from where they stopped, with no gap.

**Crash safety**

- MKV, fragmented MP4, and fragmented MOV stay playable up to the last written fragment.
- Marks flushed per mark. WAV headers refreshed every 5 seconds. `session.json` written at start.

**Device loss**

- Audio device gone: the track writes silence, adds a System mark, and reconnects if the device returns.
- Webcam gone: the last frame holds and a System mark is added.

**Threads**

- Capture, compositor and pacer, one encoder per video stream, one per audio device, muxer, file writers.
- Bounded queues between every stage.

## Build phases

Nine phases, each ending in a test you can run yourself; each phase becomes one batch of Cline prompts, sized per the prompt sizing rules. S0 is done, so phase 1 can start now. Phase 5 needs Marks Sidecar phases 1 to 3 (the \`marks\` crate's model, exports and writer) to land first.

| # | Phase | Delivers | Done when |
| --- | --- | --- | --- |
| 1 | Skeleton | `apps/recorder` egui shell on wgpu with `ui-kit` theme, `cutmark-hotkeys` crate and the suite hotkey map, settings and profile JSON in `%APPDATA%\Cutmark\recorder\`, command bus, tray, capture exclusion | Hotkeys log to the console; app windows are invisible in a test capture |
| 2 | Screen to AV1 | `capture` crate: D3D11 device, WGC capture. `media-io` crate: `Encoder` trait with the AMF backend, MKV writer. NV12 shader, CFR pacer, FFmpeg DLLs copied by `build.rs` | 10 min at 1440p60 plays in VLC and LUXEA; frame count = seconds x fps; 0 dropped |
| 3 | Audio | Device list, loopback, 48 kHz resample, AAC tracks, mixdown, WAV sidecars, meters, gain, offsets | 2 tracks + mixdown in the file; WAVs line up with video within 1 frame after 30 min |
| 4 | Vendors, rate control, output | NVENC, QSV and SVT-AV1 backends, adapter selection, codec probe, vendor-neutral RC modes, MP4 and MOV fragmented, naming template, take folders, `session.json` | ffprobe confirms each RC mode on AMD, Nvidia and Intel; killing the app mid-take leaves a playable file |
| 5 | Marks | `marks::SessionWriter` with the journal, hotkeys, popup, retroactive Highlight ranges, md and csv exports, MKV chapters, pause-aware time, `recorder.*` system marks, journal recovery on launch | Marks land within 1 frame of the keypress; after a forced kill the next launch recovers the journal into a sidecar with `recovered: true`; the sidecar reads back through the `marks` crate with zero warnings |
| 6 | Control API | WebSocket server, token in Credential Manager, commands, events, Prompter link (countdown events, hide-countdown setting, `scriptPosition`, script take files, Script column) | A test script starts, marks, pauses, and stops a take; a fake Prompter script fills `_script.csv` and the Script column |
| 7 | Monitoring | Status bar, disk meter, warnings, auto-stop, counters, countdown, pre-flight | Pre-flight reports correctly; auto-stop fires on a nearly full test drive |
| 8 | Webcam ISO | MF capture, camera control lock, own encoder session, cam offset | Cam file and main video align within 1 frame |
| 9 | Compositor and key | Chroma key shader, matte view, garbage matte, backgrounds, PiP and cam-only layouts, output scaling | 1440p60 with keyed PiP cam records 30 min with 0 dropped frames on AMD, and 10 min on Nvidia and Intel |

## Deferred

- Premiere marker export (XML), if you switch editors.
- Per-application audio capture.
- 10-bit AV1 and HDR.
- Recording two monitors at once.
- Click highlights and keystroke overlays.
- Stream companion mode (suite S3) and remote podcast sessions (suite S6) get their own specs.
- File rollover (splitting a long take into several files). The marks schema already supports it through `media[]` offsets, so it can come later without a format change.

## Open questions

- [ ] App name inside the suite ("Cutmark Recorder" for now).
- [ ] Confirm LUXEA Pro 8 plays the mixdown stream first and reads MKV chapters.
- [ ] Confirm the D3D11-to-QSV frame path on real Intel hardware during phase 4.
