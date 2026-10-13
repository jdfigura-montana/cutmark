## Current state
Task 4.2 DONE (2026-10-13). Track headers + lanes + toolbar.
- editor::set_track_height(id, h): clamps 40..=400, swaps a changed Arc<Project>, NO history entry, NO dirty (test set_track_height_clamps_without_history_or_dirty).
- ui::timeline::track_header::show(ui, editor, header_rect, lane_rect, scroll_y): one HEADER_W-wide header per track offset by scroll_y, culled offscreen; 10 px grip (drag reorders: insertion line drawn across header+lanes, Esc cancels, on release ONE MoveTrack - target boundary j from pointer content-y, new_index = j>i ? j-1 : j); double-click name -> inline TextEdit (state.renaming), Enter applies RenameTrack (non-empty + changed), Esc/click-away cancels; 4 px bottom resize strip (CursorIcon::ResizeVertical) calls set_track_height per frame (snapshot height + drag_delta); right-click menu Rename / Delete Track / Add Track Above (index=i) / Below (i+1) attached to host+grip+resize responses. State{renaming, reorder} carried via ctx data_mut get_temp_mut_or_default/insert_temp keyed "track_header_state" (clone each frame, write back at end).
- lane::draw(painter, rect, is_even, even_bg, odd_bg, separator): alternating shade + 1 px bottom separator; timeline mod.rs delegates per track.
- timeline draw() now takes &mut Ui and calls track_header::show last (insertion line above lanes). Ruler fixed; headers+lanes share scroll_y. Toolbar panel in app.rs now calls ui::toolbar::show (140 px transport-reserved space + Add Track append); Ctrl+Shift+T added in the app.rs shortcut chain. Empty space below last track already set the playhead in 4.1 (lane_rect click). Heights/order persist via Track serde in project.json.
- Task 4.1 (still true): timeline frame per SPEC - Geom/zoom/ticks, hand-drawn ruler, playhead, h-scrollbar, wheel/zoom input, wave_scale, status bar.
cargo test 194 passed; clippy -D warnings clean; fmt applied.
## Key learnings
- egui 0.36 InputState: Ctrl(zoom_modifier)+wheel goes ONLY into zoom_delta(); SHIFT converts wheel delta to horizontal (smooth_scroll_delta().x = x+y); ALT is the vertical_scroll_modifier. InputState is in egui-0.36.2/src/input_state/mod.rs (not data/input).
- egui 0.36: Context has no .style(); use ui.visuals(); ctx.options_mut works.
- egui 0.36 gotchas hit this task: CursorIcon has ResizeVertical/ResizeHorizontal (NO ResizeNorthSouth); Response::on_hover_cursor takes self (rebind result); TextEdit::frame takes egui::Frame (use Frame::NONE), no desired_size (use desired_rows/desired_width); Memory::data_mut -> get_temp_mut_or_default::<T>/insert_temp works for arbitrary Clone+Default state.
- editor tool (replace mode + insert_line) worked fine this session on src files; verify each file by reading back as usual (old INSERT-mode corruption warning may be stale).
- PeakLevel has no channels field; Peaks is NOT Default. Symphonia 0.6 / rubato 5.0.1 API rewrites; verify in registry source.
- hound WavWriter::finalize takes self by value.
- clippy 1.99: as_chunks, is_multiple_of; let-chains available and used.
- PowerShell cargo exit code 1 whenever stderr is written; judge by output text.
- cargo not on default PATH (prepend $env:USERPROFILE\.cargo\bin).
## Next steps
- Await next task prompt (4.3+: clips drawing / drag from bin per SPEC).
## Active considerations
- Do NOT build ahead into later phases (clinerules).
