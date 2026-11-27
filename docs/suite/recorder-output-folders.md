# Recorder: output folders and Replays (spec)

Nov 27, 2026 · @JD · approved (option C for replays)

Exported from the Claude Project "Creator Suite" ("Recorder Output Folders Spec"). The Project copy is the source.

## Goals

1. A user finds where recordings go, and changes it, without searching.
2. Replays are never mixed with normal takes.

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| Save recordings to | `%USERPROFILE%\Videos\CutMark` | Root for normal takes: `{root}\{project}\{take folder}` (unchanged layout) |
| Save replays to | Empty, meaning "inside each project" | Empty: `{root}\{project}\Replays\{take folder}`. Set: `{replays root}\{project}\{take folder}` |

Rules:

1. Both are absolute folder paths saved in settings.json. Old settings without them load with the defaults; an existing root value is kept.
2. Replays use the same naming template, but their take numbers count only replays (numbering is per folder, so it follows from the separate folder).
3. Both folders are created on demand.

## UI

1. **Settings > Recording, top two rows:** "Save recordings to" and "Save replays to". Each shows the full path (middle-ellipsis if long, full path as tooltip), the free space on that drive, and buttons Browse and Open. "Save replays to" also has "Use project folder" to reset it to the default, and shows "Inside each project: <root>\<project>\Replays" while empty.
2. **File menu:** "Change recordings folder…" (opens the Browse dialog for the recordings folder directly) and "Open recordings folder".
3. **First launch** (no settings file yet, or the recordings folder was never confirmed): a small dialog "Where should CutMark save your recordings?" with the default path, Browse, and Continue. Saves a `folders_confirmed` flag.
4. **Preview toolbar:** the "Next take" tooltip shows the full folder path. While the replay buffer is on, a second line shows where the next replay goes.
5. **Status bar:** disk free is for the recordings folder's drive. While the replay buffer is on and replays go to a different drive, it also shows that drive's free space.
6. **Review:** a segmented control Takes | Replays at the top of the Takes panel. Each lists its own folder for the current project, grouped by day as today. The panel header's folder button opens the folder for the current view. The "Replay saved" toast's Review button opens Review on Replays with that take selected.

## Checks

1. A recordings folder that is missing and cannot be created, or is not writable, blocks starting a take: banner "Can't save to <path>: <reason>" with a "Choose folder" button. Same for the replays folder when the buffer turns on (the buffer does not start).
2. Pre-flight checks both folders (writable, free space) and reports each.
3. Low-disk warnings and auto-stop use the drive of the folder being written.
