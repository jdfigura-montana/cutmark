# Recorder: Stream Deck (spec)

Nov 27, 2026 · @JD · approved

Exported from the Claude Project "Creator Suite" ("Recorder Stream Deck Spec"). The Project copy is the source. Builds on docs/suite/Remote-Control.md (cutmark-remote, protocol v1, the suite plugin) from AE 1.3.

## Decisions

1. The Recorder runs a `cutmark-remote` server next to its existing Control API. The API on 4466 stays for the Prompter, scripts and other tools. Both feed the same command bus, so a key press does exactly what the hotkey does.
2. Recorder actions join the existing suite plugin (`com.cutmark.suite`) in a new category "CutMark Recorder". Discovery app id `recorder`, exe `cutmark-recorder.exe`.
3. AE 1.3 (branch `ae-1.3`) merges into `main` first.

## Commands (protocol v1 `command` ids)

| id | args | Does |
| --- | --- | --- |
| `start_stop` | none | Start or stop a take |
| `pause_resume` | none | Pause or resume |
| `mark` | `category` (note, retake, cut, broll, chapter; default note) | Silent mark of that category |
| `mark_label` | none | Mark plus the label popup |
| `highlight_last` | none | Highlight the last 30 s |
| `mute_mic` | none | Mute or unmute track 0 |
| `replay_toggle` | none | Replay buffer on or off |
| `replay_save` | none | Save replay; `blocked` while the buffer is off |
| `track_mute` | `track` (1-based) | Toggle that track's mute |
| `track_gain` | `track`, `delta_db` | Change that track's gain, clamped to the track's range |

Guards return `blocked` exactly as the Recorder's own buttons would refuse (for example a bad recordings folder, or Save replay with the buffer off).

## State (published with `publish`)

```json
{
  "phase": "idle | countdown | recording | paused",
  "elapsed_ms": 0,
  "replay_on": false,
  "mic_muted": false,
  "marks": 0,
  "tracks": [ { "index": 1, "name": "Mic", "muted": false, "gain_db": 0.0, "peak_dbfs": -60.0 } ]
}
```

`elapsed_ms` and `peak_dbfs` change every frame; the crate's 20 messages per second cap keeps traffic low.

## Recorder app

1. Settings > Control API gains a "Stream Deck" row: on/off switch (default on) and status "Stream Deck connected" / "Waiting for Stream Deck" / "Off".
2. Single instance: only the first running Recorder starts the server (the crate already refuses a second).
3. The `apps.json` entry points at the installed exe, so the plugin can launch the Recorder.

## Plugin: Recorder keys

| Key | Command | Display |
| --- | --- | --- |
| Record / Stop | start_stop | Idle: record icon. Recording: red, title is the elapsed time (mm:ss). Countdown: the number |
| Pause / Resume | pause_resume | Lit while paused; dimmed while idle |
| Mark | mark + category setting | Category letter on the icon; flashes on press |
| Mark with label | mark_label | Flashes on press |
| Highlight last 30 s | highlight_last | Flashes; dimmed while idle |
| Mute mic | mute_mic | Lit (muted icon) while muted |
| Replay buffer | replay_toggle | Lit while on |
| Save replay | replay_save | Dimmed while the buffer is off; flashes on save |
| Track mute | track_mute + track setting | Title is the track name; lit while muted; "No track N" if missing |

Not running: keys dimmed with title "Open"; pressing launches the Recorder (as the audio actions do).

## Plugin: Stream Deck+ dial

| Dial | Turn | Press | Touch strip |
| --- | --- | --- | --- |
| Track gain (track setting) | track_gain, 0.5 dB per tick | track_mute | Track name, gain in dB, live level bar; muted shows "Muted" |

## Icons

SVG, per the current design language (E3) and the Recorder's app color for "on" states; red only for recording. Two states per toggle key.

## Steps (one prompt each)

| Step | Work |
| --- | --- |
| M0 | Merge `ae-1.3` into `main`, resolve conflicts, all tests and the plugin build pass |
| SD1 | Recorder: cutmark-remote server, command map, state publish, Settings row |
| SD2 | Plugin: Recorder keys and category |
| SD3 | Plugin: dial, icons, plugin version bump, pack |

## Acceptance

1. Every key works on the device with the Recorder in the background (a game focused).
2. Record key shows the running time; Replay and Mute keys light correctly; dial changes gain and the Recorder's meter shows it.
3. With the Recorder closed, a key press opens it.
4. The audio editor's actions still work.
