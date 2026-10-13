# Technology Context

## Environment
- Windows 10/11 x64; terminal is PowerShell (run one command at a time, no `&&` chains).
- Working dir: `c:\Users\JD_Admin\Documents\Programming_Projects\Audio_Editor`
- cargo is NOT on PATH by default; prepend: `$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path`

## Toolchain (installed 2026-10-12)
- rustup + Rust stable 1.99.0 (x86_64-pc-windows-msvc)
- Visual Studio 2022 Build Tools with VC++ workload (MSVC linker) â€” installed via winget after link.exe was missing

## Dependencies (Cargo.toml; add only via `cargo add`, never hand-type versions)
- eframe 0.36.2 / egui 0.36.2 â€” GUI (breaking-API release; see systemPatterns; always check docs.rs for the locked version before using crate APIs)
- symphonia 0.6.1 (features wav, mp3, pcm) â€” decode WAV and MP3 (0.6 API differs a lot from 0.5; see activeContext learnings)
- hound 3.5.1 â€” write/read WAV (16-bit, 24-bit int, 32-bit float)
- serde 1.0 (derive), serde_json 1.0 â€” project file
- uuid 1.28 (v4, serde) â€” IDs
- rfd 0.17 â€” native file dialogs
- directories 6.0 â€” %APPDATA% config path
- anyhow (app edge), thiserror (core) â€” errors
- tempfile (dev) â€” atomic write tests
- rubato 5.0.1 — resample imported media to the project rate (v5 API is a full rewrite; see activeContext learnings)
- NOT YET ADDED (later phases): cpal, mp3lame-encoder, crossbeam-channel, arc-swap

## Commands
- Validate per task, in order: `cargo fmt`; `cargo clippy --all-targets -- -D warnings`; `cargo test`
- Run: `cargo run` (debug build ~8 s after first full build)

## Constraints from clinerules
- Errors: thiserror enums in model/editor/audio/io; anyhow only in app.rs and main.rs.
- No unwrap()/expect() outside tests and main.rs.
- Timeline positions/lengths are u64 sample counts; never store seconds as floats in the model.
- Small files, one concern each; every pub item gets a one-line doc comment.
- Unit tests in same file under `#[cfg(test)] mod tests`.
- Files that matter are written to a temp file in the same folder, then renamed over the target.
- Do only what the current task asks; no building ahead.
