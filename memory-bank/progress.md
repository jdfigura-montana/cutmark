# Progress
## What works - Task 3.6 complete (2026-10-13)
- Task 3.6: bin panel UI (sort combo, two-line rows with usage count and details, offline styling, click select, inline rename, context menu, remove confirm, per-job progress bars), viewport drag-and-drop import, Ctrl+I + Project>Import Media..., Edit menu Undo/Redo, message modal for rejections and job errors. 4 new tests.
- Task 3.5: editor::media_store (session-long Loaded audio + offline set), editor::jobs (JobMessage, per-job std threads, Editor-owned JobChannels), ops::media import/remove/rename + matching Action variants, Editor job plumbing (import_files, poll_jobs, pending_jobs, take_job_errors), media accessors + reveal_media, Editor::open starts load jobs. 11 new tests. cargo test 177 passed; clippy clean; fmt applied.
- Task 3.4: io::paths unique_media_name + cache path helpers + reveal_in_explorer; io::import Loaded / import_file / load_or_rebuild. 8 new tests.
- Task 3.3: io::resample (rubato 5.0.1 sinc, delay compensation, progress).
- Task 3.2: io::decode (WAV/MP3 to interleaved f32) and io::wav. IoError + Decode/UnsupportedAudio.
- Task 3.1: util::time + util::db, model::media Peaks/PeakLevel, io::peaks.
- Phase 2 complete: model/undo core, editor ops, io (paths, project_file, recent), start screen, menus, dialogs (1.1-2.8).

## What is left (SPEC Build phases)
3. Media import complete. 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub UI. 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH; append $env:USERPROFILE\.cargo\bin each session.
- No unwrap()/expect() outside tests and main.rs (unwrap_or is fine).
- See activeContext for the reliable PowerShell file-editing pattern.

## Evolution of decisions
- 2026-10-12: peaks file format fixed at version 1 (magic PEAK, u32 version, u16 channels, u64 frames, u32 level count, per level u32 block_size + u64 count + f32 min/max pairs); read rejects trailing bytes.
- 2026-10-12: MediaStore entries persist for the session (undo/redo can re-add bin items); insert clears the offline mark.