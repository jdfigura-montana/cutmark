## Current state
Task 3.5 DONE (2026-10-12). editor::media_store (HashMap<Uuid, Loaded> + offline HashSet, entries never removed in a session); editor::jobs::JobMessage {Progress, ImportDone, LoadDone} + spawn_import/spawn_load (one std::thread per job) + JobChannels (Editor-owned crossbeam pair with spawn helpers); editor::ops::media import_media/remove_media/rename_media; Action::{ImportMedia, RemoveMedia, RenameMedia} with labels Import / Remove Media / Rename Media. Editor gained media_store, pending (BTreeMap job->(label,frac)), next_job, job_errors, jobs fields; import_files (rejects unsupported ext by file name, one job per supported file), poll_jobs (non-blocking drain; ImportDone Ok inserts into store then applies Action::ImportMedia; LoadDone Err MediaOffline -> set_offline, other Err -> set_offline + record message), pending_jobs/has_pending_jobs/take_job_errors, media/is_media_offline/media_usage/reveal_media, Editor::open spawns one load job per media item (with_project does NOT spawn). cargo test 177 passed; clippy clean; fmt applied.

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
- Await next task prompt (phase 3 remaining per SPEC: bin panel UI with bin actions except Preview).

## Active considerations
- Do NOT build ahead into later phases (clinerules).