## Current state
Task 4.3 DONE (2026-10-13). Clips drawn + drag from bin.
- ops::clip::place_clip_on_new_track(project, media_id, start): appends Track::new("Track N") with one full-length clip (src_in 0, src_out = media length); Action::PlaceClipOnNewTrack, label "Add Clip", ONE undo step (test place_clip_on_new_track_roundtrip in editor.rs).
- editor::drop_media(media_id, track_index: Option<usize>, start, insert): Some -> Action::PlaceClip (src_in 0, src_out = media len); None -> PlaceClipOnNewTrack. Errors leave the project unchanged (NotFound media / Overflow bad index).
- ui::timeline::clip_view::draw(painter, clip, name, offline, g, track_rect, visuals): clip = lane minus 2 px top/bottom, culled to visible_range, <3 px clips draw as 1 px line; 18 px header (darker, darken() 0.7 of body) with name + gain "+3.0 dB"/"-6.0 dB" (only when != 0), text clipped via painter.with_clip_rect; body visuals.widgets.hovered.bg_fill; offline: red fill + "Media offline"; rect_stroke(..., StrokeKind::Inside) border.
- bin_panel: online rows are click_and_drag; drag_started sets egui::DragAndDrop::set_payload(MediaDrag{media_id}) (payload type lives in ui::timeline::interaction). Offline rows click only.
- ui::timeline::interaction: handle_bin_drag (Esc clears payload; on any_released: over a lane -> drop Some(index), below last track inside lanes -> drop_media None, anywhere else nothing; returns error messages). draw_drop_ghost: translucent ghost at x_to_sample(ptr), media-length wide on target track; below last track -> dashed new-track row (hand-rolled dashed_rect/dashed_line, 6/4 dash/gap); Ctrl -> bright vertical insert line at start.
- timeline::show now returns Vec<String>; app.rs msgs.extend(timeline::show(...)) -> modal. Clips drawn per lane in draw(); ghost drawn after lanes, before ruler/track headers.
- Tasks 4.1/4.2 still true (timeline frame; track headers + lanes + toolbar).
- cargo test 195 passed; clippy -D warnings clean; fmt applied.
## Key learnings
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
- Await next task prompt (4.4: waveforms in clips per SPEC).
## Active considerations
- Do NOT build ahead into later phases (clinerules).
