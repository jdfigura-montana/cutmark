# Recorder Phase 10: Per-app audio (spec)

Nov 24, 2026 · @JD · draft 1

Exported from the Claude Project "Creator Suite" ("Recorder Phase 10 Per-app Audio Spec"). The Project copy is the source. Built from item 1 of the Cutmark Suite Additions Plan.

## Overview

A track's source becomes Device or App. Game, Discord and music land on separate tracks with no virtual mixer. It uses WASAPI process loopback, built into Windows 11, the same API behind OBS's Application Audio Capture.

**Goals**

1. Any app making sound can be its own track, saved by exe name so it survives restarts.
2. An app that is not running yet is attached as soon as it opens, during Idle or a take.
3. App tracks behave like device tracks everywhere else: gain, mute, offset, mixdown, WAV sidecar, meters, AAC stream.

**Non-goals**

- No "desktop audio except an app" mode in v1 (Deferred).
- No per-window capture; the unit is the app (its process tree).

## Capture (`capture` crate)

1. New `AppLoopback` next to the device loopback in `audio.rs`, built on `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` and `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`.
2. Format requested: 48 kHz, stereo, 32-bit float. Packets use the same `AudioPacket` type and QPC timestamps as device capture, so the rest of the pipeline is unchanged.
3. One exe can have several top-level processes (two instances, launcher plus game). `AppLoopback` captures every top-level process of that exe (a process whose parent is not the same exe) and mixes them; child processes are covered by the process-tree mode.
4. `list_audio_apps()` returns running apps that own an audio session on any render device: exe name, display name (file description, else exe), process ids, and whether the session is active (making sound now). Apps that are only running, with no audio session, are listed when the caller asks for all apps.
5. All Windows code stays behind `cfg(windows)` in `capture`, as today.

## Tracks (Recorder)

1. `TrackKind` gains `App`. An app track saves `exe` (lowercase file name, for example `discord.exe`) and `app_name` for display. Old settings files load unchanged.
2. Attach and detach:
   - The Recorder checks for the track's exe once a second while the app is open (Idle, Buffering later, Recording).
   - Found: start `AppLoopback`. Lost (all processes exited): stop it; the track writes silence, as a lost device does today.
   - During a take, each attach and detach adds a System mark: "App attached: Discord" and "App detached: Discord".
   - A take may start with the app not running; the track records silence until it attaches.
3. Gain, mute, offset, in mixdown, WAV sidecar, meters and the AAC stream work exactly as for device tracks.
4. The mute hotkey keeps muting track 0.

## UI (Phase 13 layout)

- **Audio tool, track group:** a new first row "Source" with a segmented control Device | App.
  - Device: today's device dropdown.
  - App: a dropdown of apps, ones making sound first (with a small live dot), then a "Show all running apps" entry that expands the list. Each entry shows the display name and exe. Below the row, a caption with the state: "Attached", "Waiting for Discord to open", or "Discord closed".
- **Sources panel, Audio group:** app tracks show the app name, a meter, and "waiting" in text.tertiary when not attached.
- **Add track:** a split choice, "Add device track" or "Add app track".

## Elsewhere

- **Pre-flight:** an app track that is not attached is Info ("Discord is not running; the track will attach when it opens"), never a failure.
- **session.json:** each track records `kind`, `exe` and `app_name` for app tracks, plus the attach and detach times in ms on the take clock.
- **Control API:** the status event's tracks include `kind`, `exe` and `attached`.
- **Marks sidecar:** attach and detach marks are System marks with source `auto`.

## Steps (one prompt each)

| Step | Work | Done when |
| --- | --- | --- |
| A1 | `capture`: `AppLoopback` (process loopback, tree mode, multi-instance mix) and `list_audio_apps()` | A test harness captures 10 s from a browser playing audio; level is non-zero; packets carry QPC times |
| A2 | Recorder: `TrackKind::App`, settings migration, attach and detach polling, silence while detached, System marks, session.json and status fields | A take started with Discord closed records silence, then Discord's audio after it opens, with the System mark at the right time |
| A3 | UI: Source row, app dropdown with "Show all", state caption, Sources panel row, Add track choice, pre-flight Info | JD records a game, Discord and the mic on three tracks |

## Acceptance

1. Game, Discord and mic on three tracks in one take; each WAV sidecar holds only its source.
2. App tracks line up with video within one frame after 30 minutes (same test as phase 3).
3. Closing and reopening the app mid-take: silence in the gap, audio again after, two System marks.
4. Settings with app tracks survive a restart and re-attach by exe name.

## Deferred

- "Desktop audio except an app" (process loopback exclude mode).
- Picking one window of a multi-window app.
