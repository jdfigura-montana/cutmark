# Active Context

## Current state
Task 3.1 DONE (2026-10-12). util::time (samples_to_timecode HH:MM:SS.mmm floor-ms, samples_to_seconds, seconds_to_samples round-nearest saturating), util::db (db_to_linear, linear_to_db -inf at 0), model::media Peaks/PeakLevel (plain, no serde), io::peaks build/write/read. IoError gained CorruptPeaks(String) variant in io/mod.rs. cargo test 147 passed; clippy clean; fmt applied.

## Key learnings
- PeakLevel has no channels field; channel count comes from Peaks.channels (io/peaks divides data.len() by channels).
- clippy 1.99: manual_checked_ops wants checked_div().unwrap_or(0); chunks_exact(2) wants as_chunks::<2>().0.
- Editor tool handles full-file/anchor edits on BOM .rs files fine now; only .md anchors were unreliable.
- PowerShell cargo invocations report exit code 1 whenever cargo writes to stderr even on success; judge by output text.
- cargo not on default PATH (append $env:USERPROFILE\.cargo\bin).

## Next steps
- Await next task prompt (phase 3 media import in progress; peaks infra now done).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
