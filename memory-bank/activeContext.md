## Current state
Task 3.3 DONE (2026-10-12). Added rubato 5.0.1 via cargo add. io::resample::resample(samples, channels, from, to, progress) -> Result<Vec<f32>, IoError>: same-rate copy; otherwise rubato Async::new_sinc (sinc_len 256, BlackmanHarris2, oversampling 256, Cubic), FixedAsync::Input, chunk 1024; manual chunk loop with Indexing offsets + final partial_len flush (Some(0) is valid); trims output_delay() startup silence, then trims/zero-pads to exactly round(in*to/from) in u64 integer math; progress per chunk. Tests: same-rate identity, 44.1->48 exact 48000 stereo frames, 1000 frames 48->44.1 = 919, impulse alignment within 2 frames, 1 kHz sine peak 0.45-0.55. cargo test 158 passed; clippy clean; fmt applied.

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
- Await next task prompt (phase 3 media import: copy to media/, cache, bin panel remain).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
