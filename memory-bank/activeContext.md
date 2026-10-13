## Current state
Task 3.4 DONE (2026-10-12). io::paths gained unique_media_name (x.wav -> "x (2).wav" style), conformed_path, peaks_path (cache/<id>.f32.wav / .peaks) and reveal_in_explorer (explorer.exe /select via raw_arg, errors ignored). New io::import: Loaded { samples: Arc<[f32]>, peaks: Arc<Peaks> }, import_file (reject unsupported ext, decode 0-0.6, copy to media/ with unique name, resample 0.6-0.85, wav::write_f32 to cache 0-0.95, peaks build+write to 1.0; on failure after the copy deletes media copy + cache files) and load_or_rebuild (media missing -> MediaOffline(id); cache valid when wav has item channels/length and peaks reads -> load; else rebuild from media/). 6 new tests (44.1->48 length 48000, duplicate naming, .txt rejected with empty dirs, cache reuse preserves mtime, cache rebuild, media missing -> MediaOffline). Let chains (`if c && let Ok(x) = y`) work and keep clippy happy even in edition 2021. cargo test 166 passed; clippy clean; fmt applied.

## Key learnings
- PeakLevel has no channels field; channel count comes from Peaks.channels (io/peaks divides data.len() by channels).
- Symphonia 0.6 is a major API rewrite vs 0.5: Probe::probe(hint, mss, fmt_opts, meta_opts), next_packet() -> Result<Option<Packet>>, gapless in AudioDecoderOptions (default true), Track.codec_params Option<CodecParameters> with .audio(), track.num_frames, Channels bitset .count(), copy_to_vec_interleaved::<f32> overwrites dst. Verify by reading ~/.cargo/registry source; docs.rs fetch often 404s.
- Rubato 5.0.1 is a full API rewrite vs 0.15: data goes through audioadapter Adapter/AdapterMut objects (rubato re-exports audioadapter_buffers; use rubato::audioadapter_buffers::direct::InterleavedSlice::new/new_mut(&slice, channels, frames)); sinc resampler is Async::<f32>::new_sinc(ratio, max_rel_ratio, &params, chunk, channels, FixedAsync::Input) with params = SincInterpolationParameters::new(sinc_len, window).oversampling_factor(n).interpolation(SincInterpolationType::Cubic); Resampler::process_into_buffer(in, out, Option<&Indexing>) -> (in_used, out_written), output_delay(), input/output_frames_next; process_all chunks and trims delay but has no progress hooks. Indexing { input_offset, output_offset, partial_len } accumulates across calls; partial_len Some(0) is legal (pure silence flush).
- hound WavWriter::finalize takes self by value (use a closure returning Result and call finalize inside).
- clippy 1.99: chunks_exact(2) wants as_chunks::<2>().0; % != 0 wants is_multiple_of(); no From<usize> for u128 (use u64).
- Editor tool handles full-file/anchor edits on BOM .rs files fine now; only .md anchors were unreliable.
- PowerShell cargo invocations report exit code 1 whenever cargo writes to stderr even on success; judge by output text.
- cargo not on default PATH (append $env:USERPROFILE\.cargo\bin).

## Next steps
- Await next task prompt (phase 3 media import remaining: bin panel with all bin actions except Preview).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
