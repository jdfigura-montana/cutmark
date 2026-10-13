## Current state
Task 3.6 DONE (2026-10-13). ui::bin_panel: BinPanel{selected,renaming,rename_text,rename_focused_once,sort,confirm_remove} with show(ui, editor) -> Vec<String> messages; header (Import... button via pick_and_import -> rfd filter wav/mp3 -> editor.import_files; Sort combo DateAdded/Name via sorted_items), two-line rows (name + right-aligned usage count; grey small details line: HH:MM:SS.mmm at project rate, kHz, Mono/Stereo, WAV/MP3 by extension), offline items red + " (offline)", pending_jobs progress bars below list, click select (accent bar + strong name), double-click name -> inline rename TextEdit (Enter applies Action::RenameMedia, Esc/blur cancels, focus requested once), row context_menu (Rename / Reveal in Explorer / Remove from Project), Remove with usage>0 opens confirm window (Remove/Cancel), usage==0 removes directly. app.rs: App owns BinPanel + messages Option<ConfirmDialog>; poll_jobs + request_repaint while has_pending_jobs; dropped files -> editor.import_files; Ctrl+I / Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y shortcuts behind egui_wants_keyboard_input; msgs (bin messages + import rejections + take_job_errors) joined into one ConfirmDialog. main.rs: ViewportBuilder::with_drag_and_drop(true). menu.rs: ProjectMenuAction::Import, Project menu "Import Media... (Ctrl+I)", new Edit menu with Undo/Redo <label> greyed when empty. egui 0.36 gotchas: dropped_files entries are Arc<dyn DroppedFile> with path() -> &Path (NOT Option; no .map); Ui has .close() not close_menu; ComboBox::from_id_salt; Window .open(&mut open) borrows open for the whole show() so Cancel must set a separate flag. cargo test 181 passed; clippy clean; fmt applied.
## Key learnings
- PeakLevel has no channels field; channel count comes from Peaks.channels (io/peaks divides data.len() by channels). Peaks is NOT Default.
- Symphonia 0.6 is a major API rewrite vs 0.5: Probe::probe(hint, mss, fmt_opts, meta_opts), next_packet() -> Result<Option<Packet>>, gapless in AudioDecoderOptions (default true), Track.codec_params Option<CodecParameters> with .audio(), track.num_frames, Channels bitset .count(), copy_to_vec_interleaved::<f32> overwrites dst. Verify by reading ~/.cargo/registry source; docs.rs fetch often 404s.
- Rubato 5.0.1 is a full API rewrite vs 0.15: audioadapter InterleavedSlice::new/new_mut; Async::<f32>::new_sinc(...); process_into_buffer / output_delay; Indexing accumulates. Verify in registry source.
- hound WavWriter::finalize takes self by value.
- clippy 1.99: chunks_exact(2) wants as_chunks::<2>().0; % != 0 wants is_multiple_of(); no From<usize> for u128.
- A `use` at module top that is only consumed inside `#[cfg(test)] mod tests` (via `use super::*`) counts as UNUSED in non-test builds -> move such imports into the tests module.
- rejected-file names from import_files are file names, not full display paths.
- Editor tool with insert_line behaves as INSERT-only here (repeated misfires this session); PowerShell here-strings passed through the terminal mangled Rust files containing backticks. Reliable pattern: [IO.File]::ReadAllText (normalize CRLF->LF) + string-array (-join "`n") Contains/Replace + [IO.File]::WriteAllText; delete + recreate whole files for big edits.
- PowerShell cargo invocations report exit code 1 whenever cargo writes to stderr even on success; judge by output text.
- cargo not on default PATH (append $env:USERPROFILE\.cargo\bin).

## Next steps
- Await next task prompt (phase 4: timeline view per SPEC).

## Active considerations
- Do NOT build ahead into later phases (clinerules).