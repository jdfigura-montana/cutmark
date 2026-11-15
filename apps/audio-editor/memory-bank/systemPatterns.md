# System Patterns

## Layered architecture (SPEC: "Code architecture")
Five sealed top-level modules; calls flow downward only:

```
src/
  main.rs, app.rs            eframe launch + App (start screen vs editor)
  model/    project, media, track, clip, marker, invariants   (plain data + serde)
  editor/   editor, action, history, selection, clipboard, snap, transport, jobs
            ops/ range, paste, clip, amplitude, track, marker, media
  audio/    engine (cpal), mix_plan, mixer, scrub, meters
  io/       decode, resample, peaks, import, wav, mp3, project_file, autosave, recent, paths
  ui/       start_screen, menu, toolbar, status_bar, bin_panel, master_strip
            timeline/ ruler, track_header, lane, clip_view, waveform, interaction
            widgets/  meter, fader
            dialogs/  new_project, export, gain, normalize, confirm
  util/     time, db
```

## Layer rules
1. `ui` calls only `editor`; it may read `model` to draw it.
2. `editor` is the only code that writes a `Project`; it calls `audio` and `io`.
3. `audio` never touches files; `io` never plays audio; they never call each other.
4. `model` depends only on std, serde, uuid; `util` holds shared helpers.
5. Only `ui`, `app.rs`, `main.rs` import egui/eframe; only `audio` imports cpal.

## Threading model (SPEC: "Code architecture" → Threads)
- UI thread: runs `ui` + `Editor`; publishes `MixPlan` via ArcSwap after every change.
- Audio thread (cpal callback): reads latest MixPlan, writes playhead to AtomicU64, meters to atomics. NO allocation, NO locks, NO file I/O, NO logging, NO panics. UI keeps replaced MixPlans alive 2 s so the last drop never happens on the audio thread.
- Worker threads (`editor::jobs`): import/conform/peaks/export; report via bounded crossbeam-channel.
- Editor→audio commands (play/pause/stop/seek/scrub) via bounded crossbeam-channel.

## Key patterns
- Action/Command for undo (`editor::action` + `editor::history`); selection is editor state, not Project data, not undoable; only one kind active (range vs clip).
- Model invariants unit-tested in `model/invariants.rs` (no overlap, src_in < src_out <= length, fade sum <= clip length, gaps = silence, length = end of last clip).
- Atomic writes: temp file in same folder, then rename (project.json, autosave, exports).
- Waveform peaks at 4 block sizes (256/1k/4k/16k); pick coarsest level with >=1 block per pixel; below 256 spp read samples directly.

## egui 0.36.2 API notes (breaking release — verified against docs.rs for the locked version)
- `eframe::App` requires `fn ui(&mut self, ui: &mut Ui, frame: &mut Frame)` — `update` is gone.
- Panels: `egui::Panel::left/right/top/bottom(id)` + `.resizable(bool)`, `.exact_size(f32)`, `.default_size(f32)`, `.show(ui, ...)` (takes `&mut Ui`, not ctx). `CentralPanel::default().show(ui, ...)`. Always add CentralPanel last. Resizable panels need `ui.take_available_space()` inside.
- Menus: `ui.menu_button("Project", ...)`.