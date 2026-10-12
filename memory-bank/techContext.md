# Technology Context

## Environment
- Windows 10/11 x64; terminal is PowerShell (run one command at a time, no `&&` chains).
- Working dir: `c:\Users\JD_Admin\Documents\Programming_Projects\Audio_Editor`
- cargo is NOT on PATH by default; prepend: `$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path`

## Toolchain (installed 2026-10-12)
- rustup + Rust stable 1.99.0 (x86_64-pc-windows-msvc)
- Visual Studio 2022 Build Tools with VC++ workload (MSVC linker) — installed via winget after link.exe was missing

## Dependencies (Cargo.toml; add only via `cargo add`, never hand-type versions)
- eframe 0.36.2 / egui 0.36.2 — GUI (breaking-API release; see systemPatterns; always check docs.rs for the locked version before using crate APIs)
- serde 1.0 (derive), serde_json 1.0 — project file
- uuid 1.28 (v4, serde) — IDs
- rfd 0.17 — native file dialogs
- directories 6.0 — %APPDATA% config path
- anyhow (app edge), thiserror (core) — errors
- tempfile (dev) — atomic write tests
- NOT YET ADDED (later phases): cpal, symphonia (wav, mp3, pcm), hound, mp3lame-encoder, rubato, crossbeam-channel, arc-swap

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