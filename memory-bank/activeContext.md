## Current state
Task 6.4 DONE (2026-10-13). Edit commands + Edit menu.
- clipboard::copy_clips(project, clip_ids): clipboard tracks span the topmost..bottommost project tracks holding a selected clip (empty in-between tracks kept as empty Vecs, so track offsets survive paste); whole clips, new ids, starts relative to the earliest selected start; length = latest end - earliest start; empty Clipboard when no clips.
- editor: copy_selection (Range->copy, Clips->copy_clips, None->nothing); cut_selection (Range: cut(ripple) then clear_selection + playhead=start; Clips: copy_clips + apply_labeled DeleteClips "Cut"); paste (empty clipboard -> Ok(()); Range -> paste_replace_range at target_track with view.ripple then clear + playhead=start; else paste_insert_at(playhead, target_track())); split (SplitAt playhead, track_ids Some(range tracks) else None); trim_to_selection (Range only, keeps selection).
- app.rs keys (no text focus): S split (plain, !ctrl), Ctrl+C copy_selection, Ctrl+X cut_selection (gesture-guarded), Ctrl+V paste, Ctrl+T trim_to_selection.
- menu.rs Edit menu order: Undo <label>, Redo <label>, sep, Cut, Copy, Paste, Delete, sep, Split, Trim to Selection, sep, Select All, Clear Selection, sep, Ripple/Snap as egui::Checkbox items (changed() -> toggle). Greyed when: no selection (Cut/Copy/Delete/Clear), clipboard empty (Paste), playhead not strictly inside a clip on the relevant tracks (Split), no Range (Trim), empty project (Select All). Delete inline duplicates app.rs delete_selection logic (App's helper is private).
- egui 0.36 gotcha: menu.button() returns Response; a match arm mixing it with a statement block fails E0308 — wrap the disabled arm in braces with a semicolon.
- 8 new tests (2 clipboard, 6 editor). cargo test 271 passed; clippy clean; fmt applied.
Task 6.3 DONE (2026-10-13). Clip selection, moving, trimming, deleting, clip right-click menu.
- clip_view: Zone enum (Header/Body/LeftEdge/RightEdge), clip_rect() helper (shared by draw + hit-test), hit_test (edges outer 6 px full height win; header top 18 px; clips < 18 px wide have no edge zones); draw takes `selected: bool` (bright border + lighten()ed header). 2 tests.
- interaction: ClipState temp "timeline_clip_state" (press/ClipPress{clip_id,track_index,pos,zone}, drag/ClipDrag::Move{ids,base_start,base_end,press_track}|Trim{clip_id,edge}, snap_line, menu/ClipMenu{clip_id,track_id,pos}); handle_clip_input runs BEFORE handle_lane_input, returns true when it consumed the press/drag (mod.rs gates lane input; Body presses fall through). Click header = select alone, Ctrl = toggle_clip; drag: select alone if unselected, begin_gesture, per-frame delta = px*spp with snap of pressed clip's start/end (min |adjust|, moving ids excluded, Alt bypass), delta_tracks = track under pointer (clamp=false, None keeps last good) - press track; preview(MoveClips), Err keeps last good; release end_gesture. Edge drags preview TrimClip with snapped pointer (pressed clip excluded). Esc mid-drag: cancel_gesture + consume_key(Escape) so app.rs Esc doesn't clear selection.
- draw_clip_menu: manual egui Area (Order Foreground, Frame::popup) at stored pos; Split (enabled when playhead strictly inside, SplitAt on this track only), Delete (selected set or just this clip, editor.view.ripple), Reset Fades (SetClipFades 0,0), Reveal in Bin (bin_panel::request_reveal -> temp "bin_reveal_request", BinPanel selects + scroll_to_cursor Center). Closes on Esc/outside click/after action.
- draw_clip_snap_line draws the drag snap line (called after timeline draw()).
- app.rs: Delete/Backspace -> Self::delete_selection (Clips: DeleteClips ripple; Range: DeleteRange, clear_selection, playhead=start); Ctrl+Z/Y guarded by gesture_base.is_none().
- egui 0.36: Frame::popup(&Style), Area::show -> InnerResponse, Memory::remove_temp::<T>, ui.scroll_to_cursor(Some(Align)).
- cargo test 263 passed; clippy clean; fmt applied.
Task 6.2 DONE (2026-10-13). Range selection mouse interaction, toggles, Shift+Space, zoom to selection.
- interaction.rs: LaneState (egui temp "timeline_lane_state": press/boundary/range_drag/snap_line); handle_lane_input (press -> set_last_clicked_track; click <3 px -> snapped playhead + clear_selection, seek while Playing; drag >=3 px -> set_range over contiguous track span, both edges snap_pos, Alt bypass; Shift+click extends nearer edge or starts range at playhead; boundary-line drag within 4 px, swaps via set_range ordering with stored tracks). draw_range: tint on selected tracks only, full-height boundary lines, bright snap line while dragging. Ctrl+press on lanes ignored (reserved for 6.3 clip select).
- egui 0.36 gotchas: Pos2-Pos2 no longer has .length() (compute via x/y sqrt); Color32::from_rgba_unmultiplied not const (use fns); get_temp_mut_or_default needs turbofish.
- timeline::mod: ruler-only playhead click (lanes handled by handle_lane_input); ViewMenuAction::ZoomToSelection; zoom_to_selection(start,end,lane_width) (spp=len/width*1.1 clamped, scroll=start-5% visible, >=0); Ctrl+\ and View menu wired; draw_range called after drop ghost.
- app.rs keys: Esc clear_selection, Ctrl+A select_all, R toggle_ripple, N toggle_snap (no text focus); Shift+Space plays selection via play_range (phase 6 comment removed).
- toolbar: Ripple/Snap selectable_label toggles (R/N). status_bar: "Sel HH:MM:SS.mmm to ... (...)". menu.rs: Edit Select All/Clear Selection (Esc, disabled when None)/Ripple/Snap; View Zoom to Selection (Ctrl+\, disabled unless Range). track_header: Ctrl+click host toggles range track.
- 5 new zoom_to_selection tests. cargo test 261 passed; clippy clean; fmt applied.
Task 6.1 DONE (2026-10-13). Selection state and snapping.
- editor::selection: Selection enum (None / Range{start,end,tracks} / Clips) + tidy(project). Range always start<end, tracks in project order, no dups.
- Editor fields `selection`, `last_clicked_track` (both private; accessors selection()/last_clicked_track()); methods set_range/toggle_range_track/select_clips/toggle_clip/clear_selection/select_all/target_track/set_last_clicked_track/toggle_snap/toggle_ripple; private tidy_selection() called from commit, undo, redo. Selection is not undoable, never dirty.
- editor::snap: free fn snap(pos, candidates, spp, threshold_px) (partition_point nearest); Editor::snap_candidates(exclude) (0, view.playhead, marker.position, clip start+end, sorted dedup); Editor::snap_pos(pos, exclude, bypass) (no-op when view.snap off or bypass; 8 px).
- No UI wiring yet. cargo test 256 passed; clippy clean; fmt applied.
Task 5.5 DONE (2026-10-13). Bin Preview.
- Editor: `preview_item: Option<Uuid>` field (3 constructors); preview_media(id) (no-op without audio/unloaded item; builds MixPlan::preview(item.length, item.channels, samples, project rate, master_volume_db), publish_preview + retire, Command::Preview), stop_preview(), previewing() (Some only while state Previewing), preview_progress() (shared.preview_pos / item.length, clamped). Test in transport.rs tests.
- bin_panel: ▶/⏹ small button left of online rows (toggles preview via editor), 2 px progress bar under the previewing row (row height 34→38, allocate_exact_size returns (Rect, Response)), context menu Preview/Stop Preview on online rows.
- Space already stops preview via toggle_play; master fader is in the preview plan.
- cargo test 242 passed; clippy clean; fmt applied.
Task 5.4 DONE (2026-10-13). Transport keys, toolbar, playhead following.
- editor::transport: TransportState enum (Stopped/Playing/Paused/Previewing, own type — ui must not see audio::player types); Editor methods transport_state/toggle_play/toggle_pause/stop/play_range/seek/jump_start/jump_end/poll_transport, all no-ops without audio. poll_transport copies Shared.playhead into view.playhead while Playing and on Playing→Stopped (return point); uses new Editor field transport_prev: Option<TransportState> (initialized in all 3 constructors).
- app.rs: poll_transport() every frame after tick(); repaints while Playing/Previewing. Keys (when egui_wants_keyboard_input false, modifiers.is_none()): Space toggle_play (consume_key(Modifiers::NONE, Key) so a focused button does not activate), P toggle_pause, Home/End jumps; `// phase 6: play range selection (Shift+Space)` comment left in the input_mut closure.
- toolbar: left buttons |◀ / ▶|■ toggle (glyph follows state) / ‖ / ■ / ▶| via text glyphs; hover texts with keys.
- timeline: ruler/empty-lane click calls editor.seek() while Playing, set_playhead otherwise; follow-paging while Playing only: when playhead < scroll_x or > visible_end, scroll_x = playhead - 5% of lane width (clamped by clamp_scroll_x), placed in show() after handle_input before Geom build.
- cargo test 241 passed; clippy clean; fmt applied.
- Task 5.3 DONE. cpal engine + Editor publishing MixPlans: audio::engine (Shared atomics, bounded(64) command queue, ArcSwap MixPlan timeline+preview, 20 ms buffer, preallocated scratch, assert_no_alloc callback), editor start_audio/publish/tick/audio_status wired into commit/undo/redo/gestures/loads; retired plans kept 2 s.
## Key learnings
- cpal 0.18.2 is an API rewrite vs 0.15: SampleRate/FrameCount are plain u32/u16 aliases (no tuple struct); supported_output_configs() yields SupportedStreamConfigRange with contains_rate/try_with_sample_rate; build_output_stream takes StreamConfig BY VALUE plus (data_cb, err_cb, Option<Duration>); Device::name() gone — use Display (device.to_string()); SampleRate(project_rate) is invalid syntax.
- assert_no_alloc 1.1.2: allocator type AllocDisabler; warn_debug feature prints a warning instead of aborting (disable_release default keeps release builds untouched); gate with #[cfg(debug_assertions)].
- app.rs Screen enum needed Editing(Box<Editor>) after Editor grew (clippy large_enum_variant); Callback::new dropped for a struct literal (clippy too_many_arguments).
- Editor tool on this machine repeatedly corrupted files via unintended insert_line (joined lines mid-token). Recovery pattern: git checkout -- <file>, then apply edits with a Python script (utf-8-sig read/write, assert count==1 per replacement). Prefer that for multi-edit changes.
- Task 5.2: `gen` is a reserved keyword in edition 2024 (renamed to `source`). Resampler pre-roll: 3 zero history frames cause a 2-frame transient at stream start (tests skip 4 samples). Keep-last-3 continuity implemented as a residual FIFO.
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
- Await next task prompt (phase 5 continues: transport keys/playhead following, bin Preview, scrub, meters).
## Active considerations
- Do NOT build ahead into later phases (clinerules).
- Editor::with_project keeps audio None (tests stay silent); status bar shows No audio there.