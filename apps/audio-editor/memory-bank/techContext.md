# Technology Context

## Environment
- Windows 10/11 x64; terminal is PowerShell (run one command at a time, no `&&` chains).
- Working dir: `c:\Users\JD_Admin\Documents\Programming_Projects\Audio_Editor`
- cargo is NOT on PATH by default; prepend: `$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path`
- Renamed 2026-10-27: crate `cutmark`, app "CutMark Audio Editor", exe `target\release\cutmark.exe`, config `%APPDATA%\CutMark` (one-time migration from AudioEditor).

## Toolchain (installed 2026-10-12)
- rustup + Rust stable 1.99.0 (x86_64-pc-windows-msvc)
- Visual Studio 2022 Build Tools with VC++ workload (MSVC linker) â€” installed via winget after link.exe was missing
- Inno Setup 6.7.3 (2026-10-16, winget JRSoftware.InnoSetup) - ISCC.exe under %LOCALAPPDATA%\Programs\Inno Setup 6
- Python 3.11 with Pillow 10.4.0 (icon + notices scripts)

## Dependencies (Cargo.toml; add only via cargo add, never hand-type versions)
- eframe 0.36.2 / egui 0.36.2 - GUI (breaking-API release; see systemPatterns; always check docs.rs for the locked version before using crate APIs)
- symphonia 0.6.1 (features wav, mp3, pcm) - decode WAV and MP3 (0.6 API differs a lot from 0.5; see activeContext learnings)
- hound 3.5.1 - write/read WAV (16-bit, 24-bit int, 32-bit float)
- serde 1.0 (derive), serde_json 1.0 - project file
- uuid 1.28 (v4, serde) - IDs
- rfd 0.17 - native file dialogs
- directories 6.0 - %APPDATA% config path
- anyhow (app edge), thiserror (core) - errors
- tempfile (dev) - atomic write tests
- rubato 5.0.1 - resample imported media to the project rate (v5 API is a full rewrite; see activeContext learnings)
- arc-swap 1.9.2 - MixPlan handoff to the audio thread
- cpal 0.18.2 - WASAPI output engine (0.18 is an API rewrite: u32 SampleRate alias, by-value StreamConfig, Device Display instead of name(); see activeContext)
- rtrb 0.4.1 - lock-free SPSC recording ring from the input callback to the record writer (0.4 renamed push_slice/pop_slice to push_partial_slice/pop_partial_slice etc.; see activeContext)
- assert_no_alloc 1.1.2 (feature warn_debug) - no-alloc guard around the output callback (wrapped in 11.1b) and the input callback (11.1); debug builds print a warning to stderr, release builds are a plain call
- crossbeam-channel 0.5 - editor-to-audio command queue and job channels
- mp3lame-encoder 0.2.5 - CBR MP3 export (LAME 3.100 compiled from source by mp3lame-sys with cc/MSVC; LGPL-3.0; API: Builder -> Encoder, encode_to_vec(MonoPcm/InterleavedPcm<f32>), flush_to_vec::<FlushGap>, lame_tag_encode_to_vec)
- winresource 0.1.34 (build-dep) - exe icon + version info (build.rs, windows only)
- image 0.25 (no default features, png only) - decodes assets/icon/app.png for the window icon
- crt-static enabled via .cargo/config.toml for x86_64-pc-windows-msvc

## Commands
- Validate per task, in order: `cargo fmt`; `cargo clippy --all-targets -- -D warnings`; `cargo test`
- Run: `cargo run` (debug build ~8 s after first full build)
- Installer: `.\scripts\build-installer.ps1` (builds release exe, runs ISCC with /DAppVersion from Cargo.toml, writes dist\CutMark-Setup-<version>.exe; fails if cutmark.exe is running). iss: installer\cutmark.iss (AppId GUID is fixed - never change). Licences bundled from licenses\ + assets\fonts.

## Constraints from clinerules
- Errors: thiserror enums in model/editor/audio/io; anyhow only in app.rs and main.rs.
- No unwrap()/expect() outside tests and main.rs.
- Timeline positions/lengths are u64 sample counts; never store seconds as floats in the model.
- Small files, one concern each; every pub item gets a one-line doc comment.
- Unit tests in same file under `#[cfg(test)] mod tests`.
- Files that matter are written to a temp file in the same folder, then renamed over the target.
- Do only what the current task asks; no building ahead.
