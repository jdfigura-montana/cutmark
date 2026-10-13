# Progress
## What works - Task 3.4 complete (2026-10-12)
- Task 3.4: io::paths unique_media_name + cache path helpers + reveal_in_explorer; io::import Loaded / import_file / load_or_rebuild with cache validation, rebuild and MediaOffline. 8 new tests. cargo test 166 passed; clippy clean; fmt applied.
- Task 3.3: io::resample (rubato 5.0.1 sinc resampler, delay compensation, exact round(in*to/from) output length, progress, 5 tests).
- Task 3.2: io::decode (symphonia 0.6.1 WAV/MP3 decode to interleaved f32, gapless, progress, >2-channel rejection, is_supported) and io::wav (32-bit float write with temp-then-rename, read). IoError + Decode/UnsupportedAudio. 6 new tests.
- Task 3.1: util::time + util::db helpers, model::media Peaks/PeakLevel, io::peaks build and LE binary write/read.
- Phase 2 complete: model/undo core, editor ops, io (paths, project_file, recent), start screen, menus, dialogs (1.1-2.8).

## What is left (SPEC Build phases)
3. Media import (remaining: copy to media/, cache, bin panel). 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub UI. 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH; append $env:USERPROFILE\.cargo\bin each session.
- No unwrap()/expect() outside tests and main.rs (unwrap_or is fine).
- Rewrite BOM .md files via Set-Content.

## Evolution of decisions
- 2026-10-12: peaks file format fixed at version 1: magic PEAK, u32 version, u16 channels, u64 frames, u32 level count, then per level u32 block_size + u64 block count + f32 min/max pairs. read rejects trailing bytes too.
