## Current state
Task 5.3 DONE (2026-10-13). cpal engine + Editor publishing MixPlans.
- audio::engine: Shared (playhead/preview_pos/state AtomicU8: 0 stopped,1 playing,2 paused,3 previewing), Engine {stream, cmd_tx bounded(64), timeline/preview Arc<ArcSwap<MixPlan>>, shared, device_info, errored}. Engine::start: default device, f32-only configs; prefers 2ch at project rate, else any ch at project rate, else device default rate + DeviceResampler; buffer 20 ms fixed when SupportedBufferSize::Range allows; preallocated 8192-frame stereo scratch; callback = assert_no_alloc{ drain rx -> player.handle, load both plans with Guard (not load_full), chunked render (scratch cap, resampler pull cap MAX_PULL_FRAMES/ratio), write_channels (mono avg, >2ch zero tail), store 3 atomics }; error callback only sets an AtomicBool. send drops on full queue; publish_timeline/publish_preview swap and return old plan; Drop of Engine (Editor) stops the stream.
- editor: audio/retired((Instant, Arc<MixPlan>))/audio_error fields; start_audio (idempotent, stores error string, NOT called by with_project); publish() builds MixPlan from project+MediaStore (no-op without audio); hooked into commit, undo, redo, preview, end_gesture, cancel_gesture, LoadDone Ok + MediaOffline; tick() drops retired >2 s, called from app every frame; audio_status() = device_info / "No audio: <err>" / "No audio". app calls start_audio after open/create. status bar right side shows audio_status(). main.rs: debug-only #[global_allocator] assert_no_alloc::AllocDisabler (feature warn_debug).
- cargo test 237 passed; clippy clean; fmt applied. cpal 0.18.2 (see learnings).
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