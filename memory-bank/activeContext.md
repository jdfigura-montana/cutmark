## Current state
Task 4.4 DONE (2026-10-13). Waveforms in clips.
- ui::timeline::waveform: pure fns choose_level (None iff spp<256; coarsest level with block_size<=spp), level_minmax (blocks covering frames from..to, (0,0) on empty/past end/bad channel), samples_minmax (raw interleaved), envelope (gain_db * min(fade_in, fade_out), saturating len-fade_out). draw(painter, clip, &Loaded, g, body_rect, wave_scale): stereo = 2 lanes L/R, mono 1, faint white center line each; spp>=1 -> one vertical line per pixel column (x_to_sample(x)..(x+1), +src_in-start mapping, env at column center sample * wave_scale, clamped to lane); spp<1 -> polyline per lane, dots when spp<0.125; whole clip added as ONE Shape::Vec.
- clip_view::draw gained media: Option<&Loaded> + wave_scale params (#[allow(too_many_arguments)]); waveform drawn after header fill when !offline && Some. timeline mod.rs passes editor.media(id) (None when offline) and editor.wave_scale().
- track volume does not affect drawing; offline media never gets a waveform.
- 8 new tests in waveform.rs (choose_level 100/256/1000/1024/20000 -> None/0/0/1/3, level_minmax ramp + zero cases, samples_minmax channel pick, envelope 1.0 / 2.0 @ +6.0206 dB / 0.5 fades). cargo test 203 passed; clippy -D warnings clean; fmt applied.
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
- Await next task prompt (4.5 per SPEC build phase 4 is done through waveforms; check next task number when it arrives).
## Active considerations
- Do NOT build ahead into later phases (clinerules).
