## Current state
Task 5.1 DONE (2026-10-13). MixPlan + mixer, pure code (no cpal).
- audio::mix_plan: ClipPlan (start/len/src_in/channels/gain/fades/samples: Option<Arc<[f32]>>), TrackPlan (audible/gain_l/gain_r/clips), MixPlan (rate/length/master_gain/tracks). MixPlan::empty(rate); build(project, &dyn Fn(Uuid)->Option<Arc<[f32]>>) resolves mute/solo (solo exclusive), constant-power pan (angle=(pan+1)*PI/4, center -3 dB/side), clip gain db_to_linear, samples None when offline, channels from MediaItem, length = max clip end; preview(media_len, channels, samples, rate, master_volume_db) = one unity track/clip.
- audio::mixer: render(plan, start, out interleaved stereo). partition_point for first overlapping clip; mix_clip uses base_local = clip.start.max(start) - clip.start for src/fade and t for output index; fade = min(applicable) of local/fade_in and (len-local)/fade_out; read_sample uses .get() (0.0 past end); master gain applied at end. No alloc/locks/panics.
- 17 new tests (12 mixer, 5 mix_plan incl. build mute/solo/pan/offline + no-solo audibility, preview, block independence 0..300 vs 0..77/77..160/160..300). cargo test 223 passed; clippy clean; fmt applied.
- arc-swap added (1.9.2), not yet used (engine task).
- Tasks 4.1-4.5 still true.
## Key learnings
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
