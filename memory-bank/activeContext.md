## Current state
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