## Current state
Task 5.2 DONE (2026-10-13). Transport state machine + device-rate resampler, pure code (no cpal).
- audio::player: Command (Play{from}/PlayRange{start,end}/Pause/Stop/Seek/Preview/StopPreview), State (Stopped/Playing/Paused/Previewing), Player{ramp_len=rate/200, position, return_to, range_end, preview_pos, ramp_pos/total/from/dir, pending}. handle() defers changes behind a 5 ms ramp-down (Pending enum; Pause captures `at` at command time so position freezes there); natural/range end starts the down ramp exactly ramp_len frames early so gain hits 0 at the end; process() renders via mixer::render in pieces split at ramp boundaries, silence when Stopped/Paused; no allocation.
- audio::resampler: DeviceResampler (ratio=project/device, fractional pos, scratch Vec of (16384+8)*2 f32, linear FIFO buf with base/len, 3 zero pre-roll frames at base=-3). fill(out, pull): drops consumed frames below floor(pos)-1, pulls up to floor(pos+(out-1)*ratio)+2, renders Catmull-Rom per frame; capacity overflow halves eff and zero-fills; no allocation. Hermite c3 = 1.5*(p1-p2)+0.5*(p3-p0) (sign fix found by test).
- 13 new tests (9 player, 4 resampler). cargo test 236 passed; clippy clean; fmt applied.
- Tasks 5.1 and 4.1-4.5 still true. Next: engine/cpal (5.3?), scrub, meters per phase 5.
## Key learnings
- Task 5.2: `gen` is a reserved keyword in edition 2024 (renamed to `source`). Resampler pre-roll: 3 zero history frames cause a 2-frame transient at stream start (tests skip 4 samples). Keep-last-3 continuity implemented as a residual FIFO (frames below floor(pos)-1 dropped, stencil floor(pos)-1..+2 always covered).
- Task 5.1 gotcha: clip-local offset for src/fade must be clip.start.max(start) - clip.start, NOT the block-local frame index (mixer read the wrong buffer offset when clip.start > block start).
- LNK1104/LNK1201 (locked exe/pdb) after panicking test runs: kill stale audio_editor processes or delete target debug deps audio_editor-*.pdb/exe, then rebuild.
- egui 0.36 DragAndDrop::set_payload/payload::<T>()->Option<Arc<T>>/clear_payload (auto-cleans on Esc/release too; payload() is Some on the release frame). Response::drag_started() exists.
- egui 0.36 Color32::from_rgba_unmultiplied is NOT a const fn — use helper fns, not consts.
- Painter::rect_stroke/rect take (rect, impl Into<CornerRadius>, stroke, StrokeKind).
- The editor tool can insert a stray BOM mid-file on large inserts (hit again); PowerShell fix: [IO.File]::ReadAllText / .Replace([string][char]0xFEFF + text, text) / WriteAllText UTF8-BOM. Mid-file EF BB BF decodes as U+FEFF char.
- egui 0.36 InputState: Ctrl(zoom_modifier)+wheel goes ONLY into zoom_delta(); SHIFT converts wheel delta to horizontal; ALT is the vertical_scroll_modifier. InputState is in egui-0.36.2/src/input_state/mod.rs.
- egui 0.36: Context has no .style(); use ui.visuals(); CursorIcon has ResizeVertical/ResizeHorizontal (NO ResizeNorthSouth); TextEdit::frame takes egui::Frame; Memory::data_mut -> get_temp_mut_or_default/insert_temp.
- PeakLevel has no channels field; Peaks is NOT Default. Symphonia 0.6 / rubato 5.0.1 API rewrites; verify in registry source.
- hound WavWriter::finalize takes self by value.
- clippy 1.99: as_chunks, is_multiple_of; let-chains available and used.
- PowerShell cargo exit code 1 whenever stderr is written; judge by output text.
- cargo not on default PATH (prepend $env:USERPROFILE\.cargo\bin).
## Next steps
- Await next task prompt (5.2 next: engine/cpal per phase 5).
## Active considerations
- Do NOT build ahead into later phases (clinerules).
