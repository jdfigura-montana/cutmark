# Active Context

## Current state
Task 3.2 DONE (2026-10-12). Added symphonia 0.6.1 (features wav, mp3, pcm) and hound 3.5.1 via cargo add. io::decode: Decoded { samples, channels, rate }, decode_file(path, progress) using symphonia probe (gapless on by default via AudioDecoderOptions), converts any sample format to interleaved f32 via GenericAudioBufferRef::copy_to_vec_interleaved::<f32>, rejects >2 channels with the exact required message, progress fires only when track.num_frames is known; is_supported (wav/mp3 case-insensitive). io::wav: write_f32 (32-bit float, .wav.tmp then rename), read_f32. IoError gained Decode(String) + UnsupportedAudio(String). Tests: 16-bit stereo 1000 frames, 24-bit mono, 4-channel rejection, is_supported, float round-trip, MP3 fixture skipped-if-missing. cargo test 153 passed; clippy clean; fmt applied.

## Key learnings
- PeakLevel has no channels field; channel count comes from Peaks.channels (io/peaks divides data.len() by channels).
- Symphonia 0.6 is a major API rewrite vs 0.5: Probe::probe(hint, mss, fmt_opts, meta_opts) (no get_format), next_packet() -> Result<Option<Packet>> (EOF = Ok(None) or IoError UnexpectedEof), gapless moved from FormatOptions to AudioDecoderOptions { gapless: true } (default true), Track.codec_params is Option<CodecParameters> enum with .audio() accessor, total frames is field track.num_frames (no total_frame_count()), channels is a Channels bitset with .count(), MediaSourceStream has a lifetime param, f32 conversion via buf.copy_to_vec_interleaved::<f32>() (overwrites dst, so extend_from_slice from a scratch vec to append). Verify 0.6 APIs by reading ~/.cargo/registry/src/.../symphonia-core-0.6.1 source; docs.rs fetch often 404s.
- hound WavWriter::finalize takes self by value (use a closure returning Result and call finalize inside).
- clippy 1.99: manual_checked_ops wants checked_div().unwrap_or(0); chunks_exact(2) wants as_chunks::<2>().0; collapsible if with let-chains (edition 2024).
- Editor tool handles full-file/anchor edits on BOM .rs files fine now; only .md anchors were unreliable.
- PowerShell cargo invocations report exit code 1 whenever cargo writes to stderr even on success; judge by output text.
- cargo not on default PATH (append $env:USERPROFILE\.cargo\bin).

## Next steps
- Await next task prompt (phase 3 media import: copy to media/, resample, cache, bin panel remain).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
