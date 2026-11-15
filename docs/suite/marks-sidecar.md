# Marks Sidecar Schema: v1 Spec

Oct 28, 2026 · @JD

## Overview

The marks sidecar is one JSON file, `{name}.marks.json`, that sits next to a recording and lists every mark made during it. Every capture source writes it and both editors read it the same way, so an editor never needs to know whether a file came from the Recorder or from OBS.

**Goals**

- One format for every source: Recorder takes, OBS recordings through the Stream companion, and the Prompter's flub and cue marks.
- Times that line up with the media to within one frame.
- Safe to read years later: versioned, forgiving of unknown fields, and never half-written after a crash.
- A Rust `marks` crate that is the only code allowed to read or write the file.

**Non-goals**

- Editor state. Whether a cut was accepted, or what a user typed in the Import marks log, belongs to the editor's own project, never back into the sidecar.
- Script text and the script position stream. Those stay in `_script.md` and `_script.csv`; the sidecar only points at them.
- Chat logs and stream event streams. The Stream companion spec (S3) defines their file; the sidecar only lists it.

**Who touches it**

| App | Writes | Reads |
| --- | --- | --- |
| Recorder | Yes, one per take | Yes, to reopen a take |
| Stream companion (S3) | Yes, one per OBS recording | No |
| Prompter | No; sends marks to the Recorder, or keeps its own log beside an OBS recording | No |
| Audio editor | No | Yes |
| Video editor | No | Yes |

## Conventions

These rules apply to every field below, so the schema section does not repeat them.

1. **Encoding:** UTF-8 JSON, no byte order mark, `\n` line endings, pretty-printed with 2-space indentation so it diffs well.
2. **Time:** integer milliseconds. Never floating-point seconds, never strings.
3. **IDs:** UUIDv7 strings, lowercase, hyphenated. Time-sortable, so marks sort by creation without a counter.
4. **Enums:** lowercase snake\_case strings. Display names ("B-roll") are the UI's job.
5. **Paths:** relative to the sidecar's folder, forward slashes, no `..`. A sidecar stays valid when its folder is copied or moved.
6. **Timestamps:** UTC, ISO 8601 with milliseconds and a `Z`, for example `2026-10-28T14:03:11.250Z`.
7. **Versioning:** one integer, `schema_version`, starting at 1. See Reading rules.
8. **Immutability:** after a session ends, nobody edits the sidecar. Editors copy what they need into their own project.
9. **Single owner in code:** only the `marks` crate parses or writes the file. No app hand-rolls JSON for it.
10. **Limits:** `label` up to 200 characters, `notes` up to 4,000, `auto.data` up to 1 KB when serialized, 100,000 marks per file. A reader that meets a larger value truncates it and warns; it never refuses the file.

## File layout and naming

The sidecar sits next to the media file it describes, with the same base name. That is the whole discovery mechanism: an editor that opens `clip.mkv` looks for `clip.marks.json` in the same folder.

```
Sessions/
  2026-10-28_ranked-grind.mkv
  2026-10-28_ranked-grind.marks.json    <- the contract (this spec)
  2026-10-28_ranked-grind_marks.md      <- derived, human-readable
  2026-10-28_ranked-grind_marks.csv     <- derived, spreadsheet-friendly
```

1. **Sidecar name:** `{media_base_name}.marks.json`. The base name is the media file name without its final extension.
2. **Derived outputs** use `_marks.md` and `_marks.csv` (underscore, not dot) so they never collide with the sidecar glob `*.marks.json`.
3. **One sidecar per recording session.** If a session splits into several media files (file rollover), the sidecar lists them all in `media[]` and every mark points at one by index. See Schema.
4. **Source of truth:** the sidecar. The `.md` and `.csv` can be deleted and regenerated at any time. Nothing reads them back except the Video Editor's *Import marks log* path, which is a convenience and not part of this contract.
5. **Atomic writes:** the Recorder writes `{name}.marks.json.tmp` and renames it into place. A reader never sees a half-written file, and a leftover `.tmp` after a crash is ignored.
6. **Crash recovery:** while recording, the Recorder also appends each mark to `{name}.marks.journal` (one JSON object per line). On a clean stop it folds the journal into the sidecar and deletes it. On startup, a journal with no matching sidecar is recovered into one. The journal is private to the Recorder and is not a public format.
7. **Moves and renames:** because paths inside the file are relative, copying the folder is safe. Renaming the media file without renaming the sidecar breaks discovery; editors may offer a manual "locate marks file" fallback, but they do not guess.

**Audio-only sessions** (the podcast flow) get a sidecar by the same rules: `ep12.wav` pairs with `ep12.marks.json`, with one entry in `media[]`. `chapter` marks still produce the chapter list. Whether the audio format can embed it is the app's choice.

## Schema

A sidecar is one JSON object with five top-level keys. The mark object has its own section below.

| Key | Type | Required | Notes |
| --- | --- | --- | --- |
| `schema_version` | integer | yes | Starts at 1. See Reading rules for what readers do with a number they do not know. |
| `session` | object | yes | Who wrote this, and when. |
| `media` | array | yes | One entry per media file in the session. At least one. |
| `context` | object | no | Optional free-text hints such as game and title. Never required to open the file. |
| `marks` | array | yes | May be empty. Sorted by `session_ms` ascending, ties broken by `id`. |

**`session`**

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| `id` | UUIDv7 | yes | Identifies the session across all its files. |
| `started_at` | timestamp | yes | UTC wall clock when recording began. |
| `ended_at` | timestamp or null | yes | `null` only when the file was recovered from a crash journal. |
| `recovered` | boolean | yes | `true` if built from the journal after an unclean stop. |
| `writer` | object | yes | `{ "app": "cutmark-recorder", "version": "1.2.0" }`. Informational. |
| `clock` | object | no | How marks were timed: `{ "source": "capture" or "obs_output", "error_ms": integer or null }`. Missing means `capture` with `error_ms` 0. |

**`media[]` entry**

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| `index` | integer | yes | 0-based, equal to the array position. Marks refer to this. |
| `path` | string | yes | Relative path from the sidecar's folder. |
| `offset_ms` | integer | yes | Where this file starts on the session timeline. `0` for the first file. |
| `duration_ms` | integer | yes | Length of this file as recorded. |

**`context`** (all optional): `game` (string), `title` (string), `notes` (string, same limit as a mark's `notes`).

Minimal valid example:

```json
{
  "schema_version": 1,
  "session": {
    "id": "0196f3a2-7c10-7b3e-9a41-2d5e8f1c6b07",
    "started_at": "2026-10-28T14:00:02.118Z",
    "ended_at": "2026-10-28T15:01:40.902Z",
    "recovered": false,
    "writer": { "app": "cutmark-recorder", "version": "1.2.0" }
  },
  "media": [
    {
      "index": 0,
      "path": "2026-10-28_ranked-grind.mkv",
      "offset_ms": 0,
      "duration_ms": 3698784
    }
  ],
  "context": { "game": "Valorant", "title": "Ranked grind" },
  "marks": [
    {
      "id": "0196f3a2-9d44-7e02-8c15-6a0b3e7d2f91",
      "type": "highlight",
      "media": 0,
      "t_ms": 754300,
      "label": "Ace",
      "source": "hotkey",
      "created_at": "2026-10-28T14:12:36.418Z"
    }
  ]
}
```

## Mark object

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| `id` | UUIDv7 | yes | Unique within the file. |
| `type` | enum | yes | One of the seven types below. |
| `media` | integer | yes | Index into `media[]`. |
| `t_ms` | integer | yes | Milliseconds from the start of that media file. `>= 0`. |
| `end_ms` | integer | no | Same clock as `t_ms`, and `> t_ms`. Makes the mark a range. Absent means a point. |
| `label` | string | no | Short title. Shown on the timeline and used as the chapter name. |
| `notes` | string | no | Longer free text. |
| `source` | enum | yes | `hotkey`, `ui`, `api`, `guest`, or `auto`. `api` is the local control API (Stream Deck, other apps). `guest` is a remote guest. |
| `created_at` | timestamp | yes | UTC wall clock when the mark was made. For debugging and ordering ties, never for placement. |
| `script` | object | no | `{ "line": string, "position": integer }`. The script line being read, stored by the Recorder when the Prompter is linked. `line` up to 500 characters. |
| `auto` | object | no | `{ "kind": string, "data": object }`. Only when `source` is `auto`. |

**Mark types**

| `type` | UI name | Shape | Meaning |
| --- | --- | --- | --- |
| `highlight` | Highlight | point or range | Something worth keeping. Feeds clips and montage. |
| `cut` | Cut | point or range | Remove this. A range means "delete this span". A point is a review flag only: "look at this spot", with no automatic trim. |
| `retake` | Retake | point or range | The take failed and was redone. The editor can offer to drop the earlier attempt. |
| `chapter` | Chapter | point only | Section start. Exported as an MKV chapter. `label` is strongly recommended. |
| `note` | Note | point or range | Reminder for the editor. No automatic behavior. |
| `b_roll` | B-roll | point or range | Place to cover with other footage. |
| `system` | System | point only | Written by the app itself, not the user: recording start and stop, pause and resume, file rollover, dropped-frame warnings, device changes. |

Rules that keep marks unambiguous:

1. `end_ms` is allowed on `highlight`, `cut`, `retake`, `note`, and `b_roll`. It is not allowed on `chapter` or `system`.
2. A range stays inside one media file. If a user marks across a rollover, the Recorder writes two ranges, one per file.
3. `system` marks always have `source: "auto"` and an `auto.kind` beginning with `recorder.`, for example `recorder.pause`. The Stream companion uses an obs. prefix instead, for example obs.pause. The Recorder writes recorder.pause, recorder.resume, recorder.device\_lost, and recorder.encoder\_overload. Editors may hide system marks by default.
4. `auto.kind` is a dotted, lowercase namespace owned by the writer, for example `obs.scene_changed` from the Stream companion. `auto.data` is opaque to everything except the code that wrote it.
5. Colors, icons, and sort order are the editor's choice. They are not stored.

## Time model

Time is the part of this contract most likely to go wrong, so it gets its own rules.

1. **Media time:** `t_ms` and `end_ms` count from playback start of the referenced media file. Frame zero is `0`. They are not wall-clock times.
2. **Container start offset:** MKV streams can carry a non-zero start time. Media time is measured after that offset is removed. A reader using FFmpeg subtracts the container `start_time` before comparing. The Recorder writes files so the offset is zero, but readers must not assume it.
3. **Session time:** `session_ms = media[i].offset_ms + t_ms`. It is derived, never stored. It exists so that marks from several rollover files can sort and display on one timeline. The `marks` array is sorted by `session_ms`, ties broken by `id`.
4. **Rollover:** each file's `offset_ms` equals the previous file's `offset_ms + duration_ms`. Rollover is gapless, so there is no "missing time" case to handle.
5. **Same clock as the frames:** the Recorder stamps a mark from the monotonic clock that also stamps captured video frames (QPC on Windows), then converts to media time. Marks cannot drift against the picture over a long session.
6. **Press time, not intent time:** `t_ms` is when the hotkey fired. The sidecar never backdates a point mark. A retroactive mark ("highlight the last 30 seconds") is stored as a range that ends at the press time, because the user asked for that span. "Clip the 10 seconds before this Highlight" is an editor setting (pre-roll), applied at read time. That keeps the recorded data honest and lets users change their mind after the fact.
7. **Pause:** The Recorder already supports pause (default Ctrl+Alt+P), and OBS does too. Paused time is left out of the media file and does not advance media time, so the file stays continuous. The writer drops a `system` mark with `auto.kind` of `recorder.pause` and another of `recorder.resume`, both at the same `t_ms`.
8. **Frame snapping:** values are millisecond-precise. An editor that wants frame-accurate placement rounds to the nearest frame, half up. It does not write the rounded value back.
9. **Scope:** a sidecar describes the original file only. If an editor trims, re-encodes, or concatenates, it maps marks into its own project timeline and leaves the sidecar alone.

## Reading rules

Goal: a bad mark never costs the user the good ones, and a bad file never crashes an editor. The `marks` crate returns `(Sidecar, Vec<Warning>)` and apps show the warnings in a collapsed "Marks import notes" row.

**Versions**

1. Additive changes (new optional fields, new enum values) do **not** bump `schema_version`. Only breaking changes do.
2. `schema_version` lower than the reader's: migrate in memory. Never rewrite the file.
3. `schema_version` higher than the reader's: refuse to load, and show "This marks file was made by a newer Cutmark ({writer.version}). Update to open it." Do not guess at a newer shape.

**Tolerance**

4. Unknown keys: ignore them.
5. Unknown `type`: treat as `note`, keep the original string for display, and warn once.
6. Unknown `source`: treat as `auto`.
7. A UTF-8 byte order mark is accepted on read (Windows editors add them), and never written.

**Validation**

| Problem | Reader does |
| --- | --- |
| File is not JSON, or `session` or `media` is missing or empty | Fail with a clear error. The file is unusable. |
| A mark is missing a required field | Skip that mark, warn. |
| `media` index out of range | Skip that mark, warn. |
| `t_ms` negative | Skip that mark, warn. |
| `end_ms` is not greater than `t_ms`, or is on a `chapter` or `system` mark | Drop `end_ms`, treat as a point, warn. |
| `t_ms` or `end_ms` past the file's `duration_ms` | Clamp to `duration_ms`, warn. |
| Duplicate mark `id` | Keep the first, warn. |
| `marks` not sorted | Sort on load, no warning. |
| Over a length limit | Truncate, warn. |

**Files and state**

8. Open read-only and do not hold a lock. The Recorder may still be writing the journal for the same session.
9. A missing media file is not an error. The marks load, and the editor offers "locate media".
10. `recovered: true` means the session ended badly. Show a small "recovered after crash" badge. Trust a probe of the real file over the last `duration_ms`, which may be short.
11. A leftover `.tmp` file is ignored. A `.marks.journal` is the Recorder's private business and editors never open it.

## Writing rules

1. **One session, one writer.** If the Recorder is recording, the Stream companion does not write a sidecar for that session, and the reverse. Two writers on one file is a bug, not a feature.
2. **Through the crate only.** Writers use `marks::SessionWriter` (`begin`, `add_mark`, `finish`). No app builds the JSON by hand.
3. **Allowed sources.** Recorder: `hotkey`, `ui`, `a``pi, guest, auto. Stream companion: hotkey, ui, auto. Prompter: none (it does not write sidecars; the Recorder stores the script line it is reading`).
4. **IDs and `created_at` are set when the mark is made**, not when the file is saved.
5. **Journal first.** Each mark is appended to `{name}.marks.journal` and flushed to disk right away. Marks are rare, so there is no batching.
6. **Finish atomically.** On stop: build the sidecar, validate it with the same rules a reader applies, write `{name}.marks.json.tmp`, flush, rename over the final name, then delete the journal. If any step fails, keep the journal, tell the user, and retry on next launch.
7. **Open ranges close at the end.** A range still open when recording stops is closed at the end of the file.
8. **Key repeat is not a second mark.** A writer ignores a repeat of the same hotkey within 250 ms.
9. **Writers that do not own the capture clock** (the Stream companion, reading an OBS recording) take media time from the recording app's reported output duration, never from their own wall clock, and follow the Companion timing method below. They set session.clock so editors know the marks are approximate.
10. **Limits are enforced on write** with the same truncation the reader applies, so a file the crate wrote always reads back with zero warnings.
11. **Format on disk:** UTF-8, no BOM, 2-space indent, keys in the order shown in the Schema example, `\n` line endings.
12. **Never modify a finished sidecar.** Not to fix a typo, not to add a mark. If a user wants to annotate after the fact, the editor stores that in its own project file.

**Companion timing method (OBS)**

What the research found: `GetRecordStatus` returns `outputDuration` in milliseconds, worked out from the number of frames the output has produced times the frame time. It is not a wall clock. No source gives an accuracy figure. Known problems: older builds returned 0, and the value is wrong for Twitch enhanced-broadcast streams (a streaming-only bug, but it confirms the frame-count derivation). OBS leaves paused time out of its files. So the method below never trusts one reading, and the accuracy is measured in Phase 6 instead of assumed.

1. **Poll for anchors.** While recording, call `GetRecordStatus` about once a second, and again on every `RecordStateChanged` and `RecordFileChanged`. Keep each reply as an anchor: local QPC time at send, local QPC time at receive, and `outputDuration`. A reading's local time is the midpoint of send and receive.
2. **Stamp marks locally.** When the hotkey fires, read QPC and extrapolate from the nearest earlier anchor: `t_ms = anchor.duration + (press_qpc - anchor.midpoint) + comp_ms`. No network request sits on the critical path, so websocket jitter cannot move a mark.
3. **Prefer clean anchors.** Ignore an anchor whose round trip is more than twice the best recent one. Discard a reading of 0 while `outputActive` is true, and any reading that goes backwards.
4. **Calibration offset.** The frame count trails the live picture by the encoder and interleave delay, so readings run early. `comp_ms` is a user-visible setting, default 0, set from the Phase 6 timing test.
5. **Pause and split.** On pause, write an `obs.pause` system mark. On resume, re-anchor at once and write `obs.resume`. On `RecordFileChanged`, close the current `media[]` entry and open the next. Real durations come from probing the closed files, never from OBS readings.
6. **Fallback.** With no valid anchor, use the local clock since the started event and set `error_ms` to `null`.
7. **Record the outcome.** `session.clock` is `{ "source": "obs_output", "error_ms": N }`, where N is the worst-case error from the Phase 6 test, or `null` until that test has run. Editors show a "marks are approximate" notice for this source.

## Derived outputs

Three outputs come from a loaded sidecar. All are generated by the `marks` crate's `export` module, never by app code, and all are deterministic: the same sidecar always produces the same bytes, so tests can compare against golden files. Nothing in a derived output may carry information the sidecar does not.

**Shared rules**

1. **Timecode** is `HH:MM:SS.mmm`, with hours allowed to exceed 99.
2. **Order** is session time, ascending.
3. **Ranges** show both ends. In the Markdown file as `start to end`, in the CSV as separate columns.
4. **Regeneration overwrites.** The Recorder writes `_marks.md` and `_marks.csv` at stop. An editor may regenerate them on demand. Anything a person typed into them is lost, so the first line of the Markdown file says so.

**`{name}_marks.md`** (for people)

- A first-line HTML comment: `<!-- Generated by Cutmark from {name}.marks.json. Edits are overwritten. Copy this file before annotating. -->`
- A title (`context.title`, else the media base name), the date, and total duration.
- One table: `#`, `Time`, `Type`, `Label`, `Notes`. `system` marks are left out. A Script column is added when any mark carries a script line.
- If the session has `chapter` marks, a short "Chapters" list above the table.

**`{name}_marks.csv`** (for spreadsheets)

- One row per mark, `system` marks included, so nothing is hidden from a spreadsheet user. A final script column appears when any mark carries a script line.
- Columns, in order: `index`, `session_ms`, `media`, `t_ms`, `end_ms`, `timecode`, `end_timecode`, `type`, `label`, `notes`, `source`.
- RFC 4180 quoting and `\r\n` line endings.
- This is the one file written **with** a UTF-8 byte order mark, because Excel on Windows needs it to read non-ASCII labels correctly.

**MKV chapters** (for players)

- Built from `chapter` marks only. Each becomes one chapter: start is `t_ms`, title is `label` or `Chapter N` when the label is empty, end is the next chapter's start or the end of the file.
- Chapters are per media file, using that file's own `t_ms`. A rollover session gets chapters in each file for the chapter marks that point at it.
- The Recorder embeds them at stop. Editors that export MKV may embed them again from their own timeline.
- No implicit "Start" chapter is invented. If the first chapter mark is at 12 s, the first chapter starts at 12 s.

The role of `_marks.md` is output only. No editor parses it. The Video Editor spec's "Import marks log" should read the sidecar instead (see Conflicts).

## The marks crate

Lives at `cutmark/crates/marks`. It is the only code that knows the file format.

**Design constraints**

1. **Pure data crate.** Depends on `serde`, `serde_json`, `uuid` (v7), `and chrono`. No egui, no tokio, no FFmpeg, no Windows APIs. It must also build on macOS unchanged.
2. **No clock reads in the model.** Converting QPC ticks to media milliseconds happens in the Recorder. The crate receives finished millisecond values. A small `Clock` trait supplies `created_at` and UUIDs so tests can fix them.
3. **Errors are typed.** `LoadError` for fatal file problems, `Warning` for everything the Reading rules say to skip or clamp.
4. **Passes `cargo-deny`** with the suite's existing policy. No new licenses.

**Modules**

| Module | Responsibility |
| --- | --- |
| `model` | `Sidecar`, `Session`, `MediaEntry`, `Mark`, `MarkType`, `Source`, `AutoInfo`. Serde derives, nothing else. |
| `read` | `load`, `discover`, version migration, validation, `Warning`. |
| `write` | `SessionWriter`, journal, atomic finish, crash recovery. |
| `export` | Markdown, CSV, and chapter list generation. |
| `time` | `session_ms` math, `HH:MM:SS.mmm` formatting and parsing, frame snapping helper. |

**Public surface (v1)**

```rust
// read
pub fn discover(media: &Path) -> Option<PathBuf>;
pub fn load(sidecar: &Path) -> Result<(Sidecar, Vec<Warning>), LoadError>;

// write
pub struct SessionWriter { /* private */ }
impl SessionWriter {
    pub fn begin(dir: &Path, base_name: &str, writer: WriterInfo, first_media: &Path) -> Result<Self>;
    pub fn add_media(&mut self, path: &Path, offset_ms: u64) -> Result<usize>; // rollover
    pub fn add_mark(&mut self, mark: NewMark) -> Result<MarkId>;
    pub fn finish(self, durations_ms: &[u64]) -> Result<PathBuf>;
    pub fn recover(dir: &Path, base_name: &str) -> Result<Option<PathBuf>>;
}

// export
pub fn markdown(s: &Sidecar) -> String;
pub fn csv(s: &Sidecar) -> Vec<u8>;            // includes the BOM
pub fn chapters(s: &Sidecar, media: usize) -> Vec<Chapter>;

// time
pub fn session_ms(s: &Sidecar, m: &Mark) -> u64;
pub fn format_tc(ms: u64) -> String;
```

The signatures are a starting point for the Cline prompts. The behavior in the sections above is the contract. Names can change without a spec change.

## Test plan

All automated tests live in the crate and run with `cargo test -p marks`. They are fast and need no media files. CI stays manual, once per milestone, per the cheap-CI rules, and this crate just joins the existing run.

Fixtures live in `crates/marks/tests/fixtures/` and are shared: other crates' tests may load them, so every app is tested against the same contract.

| # | Test | Passes when |
| --- | --- | --- |
| 1 | Round trip | `SessionWriter` output loads with zero warnings and equals the input model. |
| 2 | Golden exports | The example sidecar produces byte-identical `_marks.md`, `_marks.csv`, and chapter list fixtures. |
| 3 | Reading rules | One fixture per row of the Validation table. Each yields the named `Warning`, and every other mark in the file survives. |
| 4 | Versions | `schema_version` 99 gives `LoadError::NewerVersion`. A lower version migrates in memory and the file on disk is untouched. |
| 5 | Tolerance | Unknown keys are ignored. An unknown `type` becomes `note` and keeps its original string. |
| 6 | Encoding | A BOM on read is accepted. Sidecar output has no BOM. CSV output has one. |
| 7 | Crash recovery | A journal cut off mid-line recovers every complete mark, sets `recovered: true` and `ended_at: null`, and drops only the partial line. |
| 8 | Atomic finish | A forced failure after the `.tmp` write leaves the journal in place and no partial sidecar. |
| 9 | Time math | `session_ms` is correct across a three-file rollover. `format_tc` round-trips and handles hours above 99. |
| 10 | Limits | Over-long `label`, `notes`, and `auto.data` truncate on write. The result reads back with zero warnings. |
| 11 | Property tests | Random valid mark lists survive write then load unchanged and come back sorted. `load` never panics on arbitrary bytes. |
| 12 | Rollover chapters | Chapter marks on two media files yield per-file chapter lists using each file's own `t_ms`. |

**Manual check at each milestone** (not automated, takes five minutes):

1. Record 60 seconds with the Recorder. Press the highlight, cut, and chapter hotkeys at least once each.
2. Confirm the sidecar, `_marks.md`, and `_marks.csv` exist, and that `ffprobe` shows the chapters.
3. Open the file in the Video Editor and the Audio editor. Marks should land on the right frames to within one frame.
4. Kill the Recorder process mid-recording, relaunch, and confirm the sidecar is recovered with the badge.

**Phase 6 only: Companion timing test** (one time, about 20 minutes):

1. In OBS, show an on-screen wall clock that reads to the millisecond.
2. Record 5 minutes through the Companion and press the mark hotkey about 30 times. Include one pause and one file split. Turn on the debug log so the Companion writes each press's wall-clock time.
3. For each mark, find the frame whose on-screen clock matches the logged press time. That frame's file time minus the mark's `t_ms` is the error.
4. Set `comp_ms` to the median error. Set `error_ms` to the worst case, rounded up to a whole frame.
5. Confirm `outputDuration` freezes during pause and never reads 0 while recording.

Results go into the Stream companion spec.

## Build phases

Each phase is one Cline prompt, leaves the workspace compiling, and ships on its own. Prompts get written only after this spec is approved.

| Phase | Delivers | Needs | Tests |
| --- | --- | --- | --- |
| 1. Model and reader | `model`, `read`, `Warning`, `LoadError`, fixtures, `discover` | S0 workspace scaffold | 3, 4, 5, 6 (read side) |
| 2. Time and exports | `time`, Markdown, CSV, chapter list | Phase 1 | 2, 9, 12 |
| 3. Writer | `SessionWriter`, journal, atomic finish, recovery, limits | Phase 1 | 1, 6 (write side), 7, 8, 10, 11 |
| 4. Recorder integration | Recorder uses the crate for every mark, embeds MKV chapters, handles rollover and recovery on launch | Phases 1 to 3, Recorder v1.2 spec update | Manual check |
| 5. Editors read | Video Editor and Audio editor load the sidecar through the crate and show marks on their timelines | Phase 1, Video Editor spec update | Manual check |
| 6. Stream companion writes | Companion writes sidecars for OBS recordings | Phases 1 to 3, Stream companion spec | Manual check |

Phases 1 to 3 touch only the `marks` crate and can start the day S0 lands. Phases 4 to 6 each wait on the spec update named in their row, so those specs go next in the doc queue.

## Conflicts, deferred items, and open questions

**Conflicts with existing docs** (this spec wins once approved, and each doc gets updated)

| Doc | Conflict | Resolution |
| --- | --- | --- |
| Video Editor UI v1 Spec | Lists marker types without `highlight` and `chapter`. | Add both. Map to the seven types in Mark object. |
| Video Editor UI v1 Spec | "Import marks log" reads a `_marks.md` the user filled out. | Replace with "Import marks", which reads the sidecar through the `marks` crate. `_marks.md` stays output only. |
| Recorder v1.1 | Marks were described as hotkey events with its own list of types. | v1.2 adopts this schema, the journal, and `SessionWriter`. |
| Recorder v1.1 | A setting that backdates a point mark conflicts with press-time storage. Retroactive range marks ("highlight the last 30 seconds") are fine. | Keep retroactive ranges in the Recorder. Point-mark backdating moves to the editors as a pre-roll setting. |
| Prompter spec | Does not write sidecars, and says nothing about marks. | No change. See Deferred for retake logging. |

**Deferred past v1** (the schema leaves room, nothing is built)

- Marks scoped to one audio track.
- Colors, tags, and categories on marks.
- Screenshots, thumbnails, or voice notes (the hold-a-key-and-speak idea) attached to marks.
- A sidecar signature or integrity hash.
- Merging sidecars from two sessions.
- Marks sent by the Prompter (for example "retake" when the user rereads a line) through the Recorder's local control API on port 4466. The Recorder would remain the writer.
- Step markers from TuteTool or FigNote. These get their own integration spec.

**Decided on Oct 28** (each lands in the doc named)

1. **Pause: yes.** The Recorder already supports it (default Ctrl+Alt+P) and removes paused time from the timecode, so the schema simply records that rule. Lands in the Recorder v1.2 update as a confirmation.
2. **A point `cut` is a review flag** for now. A range `cut` still means delete that span. Lands in the Video Editor spec update. Revisit after real use.
3. **Companion timing:** anchored OBS output duration plus a calibration offset, recorded in `session.clock`. Accuracy is unproven until the Phase 6 timing test. Lands in the Stream companion spec.
4. **Audio-only recordings get a sidecar:** same format, one audio file in `media[]`. Lands in the Audio editor podcast spec.
5. **The Recorder records audio-only sessions.** A podcast or voice session is a Recorder session with video off, so the Recorder is the writer. The Audio editor's own recording (voiceover straight onto a track) writes no sidecar in v1. That keeps recording in one place, and the Recorder already owns the global hotkeys, the journal, crash recovery, and the Prompter link. Lands in the Audio editor podcast spec.
6. **Minimum OBS is 28**, the first release with obs-websocket built in. The Companion calls `GetVersion` when it connects. File-split events (`RecordFileChanged`) need obs-websocket 5.5.0 or newer, and below that the Companion tells the user to turn splitting off. Which OBS release first bundles 5.5.0 is not confirmed. Lands in the Stream companion spec.
