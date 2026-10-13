# Progress
## What works - Task 4.3 complete (2026-10-13)
- Task 4.3: clips drawn (header strip with name + gain dB, body, offline red, <3 px = 1 px line, culled to view) in ui::timeline::clip_view; drag from bin via egui DragAndDrop payload (online rows only), ghost preview over lanes incl. dashed new-track row below the last track, Ctrl insert line, Esc cancel, drop -> editor::drop_media (PlaceClip overwrite/insert or PlaceClipOnNewTrack, label "Add Clip", one undo step). timeline::show returns Vec<String> surfaced through the app message modal. 1 new test (place_clip_on_new_track_roundtrip). cargo test 195 passed; clippy clean; fmt applied.
- Task 4.2: track headers (grip reorder with insertion line + Esc cancel + one MoveTrack on release, inline rename Enter/Esc, right-click Rename/Delete/Add Above/Below, 4 px bottom resize calling set_track_height), editor::set_track_height (40..=400 clamp, UI-only: no history, no dirty), lane.rs alternating shades + separators, headers+lanes scroll together (ruler fixed), toolbar row with Add Track button + Ctrl+Shift+T. Heights/order persist in project.json. 1 new test. cargo test 194 passed; clippy clean; fmt applied.
- Task 4.1: timeline frame - Geom/zoom_at/tick_step in ui::timeline, hand-drawn ruler (adaptive ticks, markers, playhead+handle), header column, lanes from track heights, hand-rolled h-scrollbar (10 percent padded extent), click ruler/lanes sets playhead, Ctrl+wheel zoom at mouse, Shift+wheel h-scroll, wheel v-scroll, +/- keyboard zoom, egui keyboard+Ctrl-wheel zoom disabled. editor view methods, project_length, wave_scale. Status bar playhead timecode.
- Task 3.6 complete (2026-10-13)
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