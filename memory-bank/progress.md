# Progress

## What works - Task 3.1 complete (2026-10-12)
- Task 3.1: util::time + util::db helpers, model::media Peaks/PeakLevel, io::peaks build (levels 256/1024/4096/16384, level 0 from samples, higher levels merge 4 blocks of the previous) and LE binary write/read (magic PEAK, version 1, temp-then-rename). IoError::CorruptPeaks for bad magic/version/truncation/trailing bytes. 11 new tests. cargo test 147 passed; clippy clean; fmt applied.
- Phase 2 complete: model/undo core, editor ops, io (paths, project_file, recent), start screen, menus, dialogs (1.1-2.8).

## What is left (SPEC "Build phases")
3. Media import (remaining: copy to media/, decode, resample, cache, bin panel). 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub UI. 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH; append $env:USERPROFILE\.cargo\bin each session.
- No unwrap()/expect() outside tests and main.rs (unwrap_or is fine).
- Rewrite BOM .md files via Set-Content.

## Evolution of decisions
- 2026-10-12: peaks file format fixed at version 1: magic PEAK, u32 version, u16 channels, u64 frames, u32 level count, then per level u32 block_size + u64 block count + f32 min/max pairs. read rejects trailing bytes too.
