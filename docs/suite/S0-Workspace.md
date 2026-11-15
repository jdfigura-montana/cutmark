# S0 Workspace: v1 Spec

Oct 28, 2026 · @JD

## Overview

S0 turns the audio editor into the first app of the `cutmark` Cargo workspace and fixes the rules every later track builds on. It is done when `cargo build` and `cargo deny check` pass at the repo root and the audio editor exports an MP3 through FFmpeg.

**What S0 delivers** (the S0 row of the roadmap)

1. The audio editor moved into the workspace as `apps/audio-editor`.
2. The `marks` crate as a compiling skeleton, ready for its three phases in the Marks Sidecar Schema spec.
3. `CLAUDE.md`, the rules for Cline and Claude Code.
4. `deny.toml` and a passing `cargo deny check`.
5. A script in `third_party/ffmpeg` that builds an LGPL FFmpeg.
6. The LAME swap: the audio editor drops its static LAME link and exports MP3 through those FFmpeg DLLs.

**Goals**

1. One private repo, one Cargo workspace, one lockfile, one set of shared dependency versions.
2. The folder layout from the Overview becomes real, with placeholders only where something exists today.
3. License rules enforced from the first commit, so nothing GPL can slip in later without a failed check.
4. One repeatable FFmpeg build that every later app reuses.
5. A `CLAUDE.md` that keeps Cline inside the rules: specs first, no CI changes without asking.

**Non-goals**

- No other shared crates. A crate is extracted only at the milestone where a second app needs it: `ui-kit` at S1, `cutmark-hotkeys`, `media-io` and `capture` at S2, `audio-engine` and `core-model` at S4, `remote` at S6.
- No Recorder, Prompter, or Video editor work. Their folders appear when their milestone starts, apart from what already exists on disk.
- No new audio editor features and no bug fixing. That is S1.
- No CI beyond the one manual workflow. See CI.

## Folder layout

S0 adds three things to the existing `cutmark` repo (`apps/audio-editor`, `crates/marks`, `third_party/ffmpeg`) plus a handful of root files. Nothing already there moves. This is the layout on disk today, read from `cm_suite` on Oct 28, plus what S0 adds:

```
cm_suite/
  filmcraft/                  reference clone, read-only, not a workspace member
  cutmark/                    private repo and Cargo workspace root
    Cargo.toml  Cargo.lock    exist; S0 adds [workspace.lints] and shared dependencies
    CLAUDE.md                 exists; S0 rewrites it (see Conventions)
    deny.toml                 S0
    rust-toolchain.toml       S0
    rustfmt.toml  clippy.toml exist
    .cargo/config.toml        exists; S0 adds static CRT and FFMPEG_DIR
    .clinerules/              cutmark.md exists; S0 adds audio-editor.md
    .github/workflows/ci.yml  S0, manual trigger only
    scripts/check.ps1         S0, the four local checks in one command
    docs/
      suite/                  S0, specs exported from Claude Docs as Markdown
      audio-editor/           S0, from the old repo (spec.md, ui-target.png, security/)
      video-editor/           exists
      spikes/                 exists on feat/editor-shell
    apps/
      audio-editor/           S0, moved in, package cutmark-audio
      video-editor/           exists, untouched
      cutmark-editor/         exists on feat/editor-shell, untouched
      host-spike/             exists on feat/editor-shell, untouched
    crates/
      marks/                  S0, compiling skeleton
      video-ui/               exists, untouched
    third_party/
      filmcraft/              exists, keep intact
      ffmpeg/                 S0, fetch script, notices, BUILDINFO.txt
```

Rules:

1. **FFmpeg DLLs are not committed.** They run to tens of megabytes and would bloat the repo. `third_party/ffmpeg/dist/` is git-ignored, and the script recreates it from a pinned source with a checked SHA-256. This replaces the Overview's "DLLs" in that folder.
2. **The audio editor keeps its own folder tree.** `src/`, `assets/`, `tests/`, `installer/`, `scripts/`, `licenses/` and `memory-bank/` all move under `apps/audio-editor/` unchanged. Only its docs go to `docs/audio-editor/`.
3. **One `target/` at the repo root.** The old `Audio_Editor/target` is not moved.
4. **The old `Audio_Editor` folder stays** as a read-only archive until S1 is tagged. After that JD may delete it.
5. **Cline rules live at the root.** Cline reads `.clinerules/` from the folder VS Code opens, so the audio editor's rules become `.clinerules/audio-editor.md` there, with paths updated. The memory bank stays inside `apps/audio-editor/`.
6. **Suite specs are exported to `docs/suite/` before any Cline prompt relies on them**, so the repo, not a chat, is the source of truth.

## Cargo workspace

The root `Cargo.toml` is already right, so S0 keeps it and changes six things.

**Stays as is:** `resolver = "3"`, members `apps/*` and `crates/*`, edition 2024, `rust-version = "1.95"`, `publish = false`, license `LicenseRef-Cutmark-Proprietary`, the 17 FilmCraft crates pinned to one git revision, egui and eframe 0.36, and the dev and release profiles.

**S0 changes**

1. **Shared dependencies.** A dependency moves into `[workspace.dependencies]` only when two members use it. At S0 that adds `uuid` (features `v4`, `v7`, `serde`) and `thiserror`, used by both the audio editor and `marks`. Audio-only dependencies (`cpal`, `rubato`, `symphonia`, `hound`, `arc-swap`, `assert_no_alloc`, `crossbeam-channel`, `egui-phosphor`, `image`, `trash`, `anyhow`, `directories`) stay in its own manifest. Versions are added with `cargo add`, never typed by hand.
2. **One date library.** Everything uses `chrono`, as the root manifest already says. The `marks` crate uses `chrono`, not `time`.
3. **Feature unification.** Building from the root merges the features each member asks for. `eframe` stays `default-features = false` at the root and each app lists what it needs; the audio editor spells out its current default features. Every app sets `NativeOptions.renderer` explicitly, so a video app turning on wgpu never changes the audio editor's renderer.
4. **Lints.** Add `[workspace.lints]` with the baseline (`dbg_macro` and `todo` at warn). Members opt in with `[lints] workspace = true`. The strict set (`unwrap_used`, `expect_used` and `panic` at deny, `unsafe_code` forbidden) stays per crate, as in `cutmark-editor`, because the video crates carry FilmCraft-derived code. `marks` opts into the strict set. The audio editor opts into the baseline only; strictness is S1.
5. **Static CRT.** `.cargo/config.toml` gains `[target.x86_64-pc-windows-msvc]` with `rustflags = ["-C", "target-feature=+crt-static"]`, which the audio editor has today so users need no VC++ redistributable. It now applies to every app. The Test plan checks that the video apps still build and run. If they cannot, the suite goes back to the dynamic CRT and the installers bundle the VC++ redistributable.
6. **Package names.** The audio editor's package is renamed from `cutmark` to `cutmark-audio`, matching `cutmark-video`. Its exe, installer script, and Windows resources follow. It keeps its own `version` (1.1.0) because the installer name and exe resources read it. Crates use `version.workspace = true`.

Two cleanups ride along. The audio editor's own `[profile.release]` is deleted, since profiles in member manifests are ignored and the root one already sets thin LTO and one codegen unit. The `exclude = ["filmcraft"]` line can stay: it does nothing, because `filmcraft` is a sibling folder, not inside the repo.

## Crate map

S0 creates one shared crate, `marks`. The others are names on a list until the milestone where a second app needs them.

| Crate | Holds | Created at | Used by |
| --- | --- | --- | --- |
| `marks` | Sidecar schema, read and write, md and csv export | S0 | Every app, TuteTool later |
| `ui-kit` | egui dark theme, meters, faders, common dialogs | S1 | Audio editor, Recorder, Video editor |
| `cutmark-hotkeys` | Suite hotkey map, one owner per chord, per-OS defaults | S2 | Recorder, Prompter, later every app with global hotkeys |
| `media-io` | FFmpeg wrapper, `Encoder` trait, decode, resample, mux | S2 | Every app |
| `capture` | D3D11 device, screen capture, WASAPI, webcam | S2 | Recorder, TuteTool on Windows |
| `audio-engine` | Mixer, meters, LUFS, playback | S4 | Audio editor, Recorder, Video editor |
| `core-model` | Timeline, clips, undo history | S4 | Audio editor, Video editor |
| `remote` | WebRTC client, session sync | S6 | Recorder |
| `prompter-core`, `prompter-platform`, `prompter-voice` | The Prompter's own logic | Prompter milestone | Prompter only, not shared |

`video-ui` already exists. It is the video editor's UI, copied from FilmCraft and now ours, so it belongs to the video apps and is not a shared crate.

**Dependency rules**

1. Apps depend on crates. Crates never depend on apps.
2. Only `ui-kit` and the apps may depend on egui. Every other shared crate stays UI-free, so TuteTool's Tauri backend can use them.
3. FilmCraft is used only by the video apps and `video-ui`. No shared crate depends on it.
4. `marks` depends on nothing else in the workspace.
5. Windows-only code lives behind a trait or `cfg(windows)` inside `capture`, `media-io`, and `prompter-platform`. Every other crate is OS-neutral, so it builds and tests on a Linux runner and later on macOS.
6. No cycles. A new crate needs a second consumer, and gets listed in this table first.
7. Naming: apps are `cutmark-{name}` (`cutmark-audio`, `cutmark-video`). Shared crates keep the short names above.

## Apps

S0 changes one app, the audio editor. Everything else is named here so the layout stays honest.

| App | Folder | Package | Stack | In S0 |
| --- | --- | --- | --- | --- |
| Audio editor | `apps/audio-editor` | `cutmark-audio` | Rust, egui | Moved in, renamed, LAME swapped for FFmpeg |
| Video editor, baseline | `apps/video-editor` | `cutmark-video` | Rust, egui (FilmCraft UI copy in `video-ui`) | Untouched |
| Video editor, new shell | `apps/cutmark-editor` | `cutmark-editor` | Rust, egui host around FilmCraft | Untouched (lives on `feat/editor-shell`) |
| Host spike | `apps/host-spike` | `host-spike` | Spike | Untouched. Retiring it is the video track's call |
| Recorder | `apps/recorder` | `cutmark-recorder` | Rust, egui | Created at S2 |
| Prompter | `apps/prompter` | `prompter-app` | Tauri 2 and React | Created at the Prompter milestone |
| TuteTool | Its own repo, `Programming_Projects\TuteTool` | n/a | Tauri 2 and React | Not in this workspace |

1. **S0 touches no video code.** One thing to know: `apps/cutmark-editor` depends on FilmCraft by path (`../../../filmcraft/crates/...`), while the root pins a git revision. A clean checkout of that branch builds only with the sibling `filmcraft` clone present. Moving those two lines to the pinned revision is the video track's first job, and CI cannot run until it is done (see CI).
2. **The Prompter's Tauri shell** needs a `Cargo.toml` that the `apps/*` glob can find. Whether it sits at `apps/prompter` or in a `src-tauri` subfolder with one extra explicit member is settled in the Prompter milestone spec.
3. **TuteTool stays in its own repo.** How it consumes `marks`, `capture` and `media-io` is decided at T2, when it first needs one. The default is a git dependency on the private `cutmark` repo, pinned by revision.

## Toolchain and tooling

The repo already has `rustfmt.toml` and `clippy.toml`. S0 adds the toolchain pin, `deny.toml`, and one script that runs every local check.

| Tool | Setting | State |
| --- | --- | --- |
| Rust toolchain | `rust-toolchain.toml`: the stable version installed on S0 day (must be 1.95 or newer), components `rustfmt` and `clippy` | S0 adds |
| rustfmt | `max_width = 160`, `use_small_heuristics = "Max"` | Exists |
| clippy | Allows `unwrap`, `expect`, `panic` and indexing in tests only | Exists |
| cargo-deny | `deny.toml`; install with `cargo install cargo-deny --locked` | S0 adds |
| LLVM (libclang) | Needed by the FFmpeg bindings' `bindgen`; set `LIBCLANG_PATH` to its `bin` folder | Installed in Phase 0 |
| Inno Setup 6 | Builds the audio editor installer | Already needed today |
| `scripts/check.ps1` | Runs the four local checks in order and stops at the first failure | S0 adds |

**The check script** runs, in this order: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`. It prints which step failed. A list of excluded packages sits at the top of the script and starts empty. If the first run shows the copied FilmCraft UI crates already fail clippy, they go on that list by name, with a comment, until the video track cleans them. Nothing else may be excluded.

**Rules**

1. Cline runs one command at a time in PowerShell and never chains with `&&`. That is why the checks live in a script.
2. The first `cargo fmt` over the moved audio editor is its own commit, "format only", because the root `rustfmt.toml` is wider than the default it was written with. That keeps the real diffs reviewable.
3. Dependencies are added with `cargo add`, never by typing a version.

**Left out on purpose:** xtask, cargo-nextest, cargo-hack, pre-commit hooks, Dependabot or Renovate, code coverage, and sccache. Each is one more thing to maintain, and none solves a problem the suite has today.

## Licensing and dependency policy

The suite is commercial and closed source, so `cargo deny check` is the gate: no GPL, AGPL, or unknown license gets in, and LGPL gets in only as the FFmpeg DLLs.

**Rules**

1. **FFmpeg is an LGPL build, linked dynamically.** No `--enable-gpl`, no `--enable-nonfree`, no x264, x265 or openh264. The DLLs ship as loose files beside the exe so a user can swap them. Each installer carries the LGPL text, a notice naming FFmpeg and its version, and a link to the exact source used.
2. **No LGPL Rust crate is linked statically.** Today that is `mp3lame-encoder` and `mp3lame-sys` (LAME, LGPL-3.0), which the LAME swap removes. After the swap, `deny.toml` bans both by name.
3. **`CLAUDE.md` rule 7 is reworded.** "No GPL or LGPL dependencies" contradicts rule 1. The new wording: no GPL or AGPL anywhere, and LGPL only as the FFmpeg DLLs from `third_party/ffmpeg`.
4. **H.264 and HEVC are hardware-encoded only.** Software encoding is AV1 (SVT-AV1) only. S0 enforces this through the FFmpeg configuration: no software H.264 or HEVC encoder is enabled.
5. **FilmCraft is MIT or Apache-2.0.** Its notices stay intact in `third_party/filmcraft`. No FilmCraft or ArtCraft name or logo appears anywhere a user can see it.
6. **Fonts** (IBM Plex Sans, JetBrains Mono, OFL 1.1) keep their license files in the installer.
7. **Notices.** The audio editor's `gen_notices.py` stays for now. S0 updates its preamble (LAME out, FFmpeg in). The suite-wide notice tool is decided at R.

**`deny.toml` starting point**

```toml
[graph]
targets = ["x86_64-pc-windows-msvc"]

[advisories]
version = 2
yanked = "deny"
# Any ignore needs a comment with the advisory id and the reason.

[licenses]
version = 2
allow = [
  "MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause",
  "ISC", "Zlib", "Unicode-3.0", "CC0-1.0", "0BSD", "Unlicense", "BSL-1.0", "MPL-2.0", "WTFPL",
]
confidence-threshold = 0.9
exceptions = [
  # egui's bundled fonts. Confirm the crate name and licenses on the first run.
  { crate = "epaint_default_fonts", allow = ["OFL-1.1", "Ubuntu-font-1.0"] },
]

[licenses.private]
ignore = true   # our own crates carry LicenseRef-Cutmark-Proprietary

[bans]
multiple-versions = "warn"
wildcards = "deny"
allow-wildcard-paths = true
deny = [{ name = "mp3lame-sys" }, { name = "mp3lame-encoder" }]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-git = [
  "https://github.com/jdfigura-montana/filmcraft",
  "https://github.com/storytold/craft-libs",
]
```

This was written without running the tool, so the first `cargo deny check` will name anything missing or misspelled for the installed version. Cline fixes the syntax, and adds a license only after confirming it is permissive. GPL, AGPL, LGPL, SSPL, and anything with a commercial-use limit never go on the list.

## Conventions

The repo has two rule sets today: `cutmark/CLAUDE.md` (video side) and the audio editor's `CLAUDE.md` plus `.clinerules`. S0 merges them into one.

**Rule files after S0**

1. **`CLAUDE.md`** at the root is the single rule file for Claude Code and Cline. It keeps the current hard rules (rules 1, 2, 5, 6, 8, 9 and 10 as they stand, rules 3 and 4 limited to the video apps (the audio editor has its own rules), rule 7 reworded per Licensing) and adds the suite rules below.
2. **`.clinerules/cutmark.md`** stays a one-line pointer to `CLAUDE.md`.
3. **`.clinerules/audio-editor.md`** carries what is specific to the audio editor: the module call flow (`ui` calls only `editor`, `editor` is the only code that changes a `Project`, `audio` never touches files, `io` never plays audio), the audio-thread rules (no allocation, locks, file I/O, logging or panics in the cpal callback), UTF-8 without a BOM with non-ASCII escaped as `\u{...}` in string literals, and the egui 0.36 gotchas. The old "edition 2021" and "Windows 10/11" lines are corrected to edition 2024 and Windows 11.
4. **Spec outranks memory bank.** `docs/audio-editor/spec.md` (the file name is lowercase on disk, though the old rules wrote `SPEC.md`) outranks `apps/audio-editor/memory-bank/`.

**Suite rules added to `CLAUDE.md`**

1. **Work comes from a spec.** Read the spec sections the prompt names, in `docs/suite/` or the app's docs folder. Do only what the prompt asks, and do not build ahead.
2. **Done means** `scripts\check.ps1` passes, plus a release build of the app the task touched.
3. **No CI changes without asking.** No workflows, jobs, triggers, matrix entries, retries or schedules.
4. **One command at a time** in PowerShell, no `&&` chaining.
5. **No em dashes** in docs or UI text.

**Code conventions**

- **Errors:** `thiserror` enums in libraries. `anyhow` only in `main.rs` and `app.rs`. No `unwrap` or `expect` outside tests and `main.rs`; the lint enforces it in new crates.
- **Logging:** the `log` facade. Apps pick the backend. Nothing logs on an audio thread.
- **Time:** audio positions are `u64` sample counts, marks are integer milliseconds, wall-clock time goes through `chrono`.
- **Paths:** suite-wide files live in `%APPDATA%\Cutmark\`, and each app's settings in `%APPDATA%\Cutmark\{app}\`. Code gets them from the `directories` crate and never builds them by hand. The Recorder v1 spec says `%APPDATA%\{app}\`, which gets corrected in its v1.2 update.
- **Files that matter** (projects, autosave, exports, sidecars) are written to a temp file in the same folder and renamed over the target.

**Git**

- `main` stays buildable. S0 work happens on `s0-workspace`.
- One commit per Cline phase, titled `S0 phase N: <what>`.
- The milestone ends with the tag `s0`, after `scripts\check.ps1` and one manual CI run are green.

## Local control ports and shared config

S0 builds none of this. It writes the addresses and file names into `CLAUDE.md` so no later app picks a clashing port or invents a second hotkey file.

| Service | Address | Auth | Built at |
| --- | --- | --- | --- |
| Recorder | `ws://127.0.0.1:4466` | Token from Recorder settings | S2 |
| Prompter | `ws://127.0.0.1:4467` | Token from Prompter settings | Prompter milestone |
| OBS (external) | `ws://127.0.0.1:4455`, obs-websocket v5 | OBS WebSocket password | Stream companion |

1. Ports are configurable. Every listener binds to `127.0.0.1` and never to `0.0.0.0`.
2. Tokens and passwords live in Windows Credential Manager, never in a JSON file.
3. **Suite hotkey map:** one file, `%APPDATA%\Cutmark\hotkeys.json`, read by every app through `cutmark-hotkeys` (created at S2). Each chord has exactly one owner app, and assigning a chord another app owns is refused with the owner's name. Defaults are stored per OS.
4. `hotkeys.json` is the only shared file under `%APPDATA%\Cutmark\` for now. A new shared file needs an entry here first.

## FFmpeg and the LAME swap

The audio editor exports MP3 through FFmpeg's `libmp3lame` instead of a statically linked LAME crate. The same DLLs become `media-io`'s engine at S2, so S0 sets them up once, properly.

1. **Source of the DLLs.** `third_party/ffmpeg/get-ffmpeg.ps1` puts an LGPL shared build in `third_party/ffmpeg/dist/` (`bin`, `lib`, `include`). Only a pinned source with a pinned SHA-256 is accepted. Two sources are allowed, tried in this order:
   - A prebuilt LGPL shared Windows build, but only if `ffmpeg -buildconf` shows `--enable-libmp3lame` and neither `--enable-gpl` nor `--enable-nonfree`. The public prebuilt-build pages do not say whether `libmp3lame` is included, so Phase 3 checks this first.
   - A build from source in MSYS2 with `--enable-shared --disable-static --disable-gpl --disable-nonfree --disable-autodetect --disable-programs --disable-doc --enable-libmp3lame`, kept in the same folder. S2 extends this line with the hardware encoders.
2. **Paper trail.** Whichever source is used, the script writes `BUILDINFO.txt` with the URL, version, SHA-256 and configure line. The installer ships it as the source offer, along with the license text that matches the build.
3. **Rust side.** The `ffmpeg-next` crate with `default-features = false` and features `codec` and `format`, in the audio editor only. `.cargo/config.toml` sets `FFMPEG_DIR = { value = "third_party/ffmpeg/dist", relative = true }`. The build needs LLVM for `bindgen`. Phase 3 checks that the `ffmpeg-next` release supports the chosen FFmpeg major version, and if not, picks the newest major it does support.
4. **Runtime.** The audio editor's `build.rs` copies `dist/bin/*.dll` next to the exe in the target folder, so `cargo run` and tests find them. The installer copies them beside the installed exe.
5. **Behavior must not change.** The public API of `io::mp3` stays the same; only its insides swap.
   - CBR only, with the bitrates the export dialog offers today.
   - Resampling stays with `rubato`, so FFmpeg receives PCM at the final rate and needs no `swresample`.
   - The output starts with a correct Xing or Info frame, so the editor's own decoder reads the exact length back.
   - Progress, cancel, the temp-file-then-rename write, and the clipping warning pass are untouched.
6. **Removal.** `mp3lame-encoder` leaves `Cargo.toml`, and `Cargo.lock` must no longer contain `mp3lame-sys`. The old `licenses/LGPL-3.0.txt` is replaced by the license text that matches the FFmpeg build.
7. **Out of scope:** hardware encoders, decoding through FFmpeg (the audio editor keeps `symphonia`), and extracting `media-io`. Those are S2.

## Build and run

After S0, a clean checkout builds with the steps below. All commands are PowerShell, one at a time.

**One-time setup**

1. Install the checker: `cargo install cargo-deny --locked`
2. Install LLVM: `winget install LLVM.LLVM`
3. Point `bindgen` at it: `[Environment]::SetEnvironmentVariable("LIBCLANG_PATH", "C:\Program Files\LLVM\bin", "User")`, then open a new terminal.
4. Inno Setup 6 stays installed for the audio editor installer. MSYS2 is needed only if FFmpeg has to be built from source (see FFmpeg and the LAME swap).

**Clean checkout**

```powershell
cd C:\Users\JD_Admin\Documents\Programming_Projects\cm_suite
git clone https://github.com/jdfigura-montana/cutmark.git
git clone --branch cutmark-base https://github.com/jdfigura-montana/filmcraft.git
cd cutmark
powershell -ExecutionPolicy Bypass -File third_party\ffmpeg\get-ffmpeg.ps1
cargo build --workspace
```

The `filmcraft` clone is needed only while `apps/cutmark-editor` uses path dependencies. Its checked-out commit must match the revision pinned in the root `Cargo.toml`. If `dist/` is missing, the audio editor's `build.rs` stops with a message telling you to run `get-ffmpeg.ps1`.

**Everyday commands**

```powershell
cargo run -p cutmark-audio
cargo run -p cutmark-video
scripts\check.ps1
cargo build --release -p cutmark-audio
powershell -ExecutionPolicy Bypass -File apps\audio-editor\scripts\build-installer.ps1
```

The release audio exe is now `target\release\cutmark-audio.exe`. The installer lands in `apps\audio-editor\dist\`.

## Test plan

S0 is done when all twelve checks pass. The roadmap's two headline checks are #1 with #3, and #6.

| # | Check | Passes when |
| --- | --- | --- |
| 1 | Clean build | From a fresh clone, `get-ffmpeg.ps1`, and the one-time setup, `cargo build -p cutmark-audio` and `cargo build -p cutmark-video` each succeed, run as two separate commands (CLAUDE.md rule 15). |
| 2 | Local checks | `scripts\check.ps1` passes: fmt, clippy, tests, deny. Any excluded package is named in the script with a reason. |
| 3 | License gate | `cargo deny check` passes with no LGPL exception, and `Cargo.lock` contains neither `mp3lame-sys` nor `mp3lame-encoder`. |
| 4 | Audio tests carried over | `cargo test -p cutmark-audio` passes, with at least the Phase 0 baseline (656). Phase 3 adds one FFmpeg test, so 657. |
| 5 | History kept | `git merge-base --is-ancestor 67ec7e4 HEAD` succeeds, which shows the old repo's history is in the tree. |
| 6 | MP3 export through FFmpeg | A 3-minute project exports at 128 and 192 kbps CBR, mono and stereo, at 44.1 and 48 kHz. The editor re-imports each file at the same length within one MP3 frame (about 26 ms), and the files play in Windows Media Player with no clicks. An automated test encodes one second of sine through `io::mp3`, decodes it with `symphonia`, and checks length and non-silence. `ffprobe` is an optional second opinion. |
| 7 | Cancel | Cancelling an export mid-way leaves no partial file, as today. |
| 8 | Static CRT | `cargo build --release` of `cutmark-audio` and `cutmark-video` both succeed and launch. The audio installer then installs and runs in Windows Sandbox or a clean VM with no VC++ redistributable. |
| 9 | Installer | `build-installer.ps1` produces `CutMark-Setup-1.1.0.exe`. The installed folder holds the exe, the FFmpeg DLLs, the license text, and `BUILDINFO.txt`. |
| 10 | Rename | The exe is `cutmark-audio.exe`, and its product name and window title still read "CutMark Audio Editor". |
| 11 | `marks` skeleton | `cargo test -p marks` runs one smoke test, and `cargo clippy -p marks` is clean under the strict lint set. |
| 12 | One manual CI run | Actions > CI > Run workflow finishes green on Linux. See CI for what it covers. |

## CI

S0 adds one workflow that only a person can start, and it runs once per milestone. The repo has no `.github` folder today, so this is the first CI it gets.

**Rules** (the Overview's CI rules, unchanged)

1. No run on push, pull request, or tag. The only trigger is `workflow_dispatch`.
2. Local checks come first: `scripts\check.ps1`. CI repeats the OS-neutral part once, right before tagging.
3. Linux runners only. Windows-only code (both editors) is covered by the local checks, because Windows runners cost twice as much.
4. Every job has a timeout. Caches are on, and a new run cancels a running one. Artifacts upload only on failure and live for 1 day.
5. The Actions budget is $0 with "Stop usage when budget limit is reached" switched on.
6. Cline never adds workflows, jobs, triggers, matrix entries, retries or schedules without asking.

**`.github/workflows/ci.yml` (S0)**

```yaml
name: CI
on: workflow_dispatch
concurrency:
  group: ci
  cancel-in-progress: true
permissions:
  contents: read
jobs:
  neutral:
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy -p marks --all-targets -- -D warnings
      - run: cargo test -p marks
      - uses: EmbarkStudios/cargo-deny-action@v2
```

The `-p` list grows by one name each time an OS-neutral crate is added, and only then.

**Blocker:** any Cargo command that resolves the workspace fails on a fresh Linux checkout while `apps/cutmark-editor` depends on FilmCraft by a path outside the repo. Phase 5 therefore checks for that first. On Oct 28 main had no such dependencies, because cutmark-editor and host-spike live only on feat/editor-shell, so the blocker returns only when that branch merges. If the path dependencies are still there, Phase 5 skips the CI steps and says so, and S0 closes without its CI run until the video track switches them to the pinned git revision. If `jdfigura-montana/filmcraft` is private, so once the git dependencies are in place the workflow also needs a step that gives git a read-only token from a repository secret.

A run should take under 10 minutes on a cold cache, so a milestone costs a few minutes of the free allowance.

## Build phases

Six phases, in order. Phase 0 is JD by hand. Phases 1 to 5 each get one Cline prompt (`S0 Cline Prompts`, written after this spec is approved). After every phase: run `scripts\check.ps1` (from Phase 1 on), read the diff, commit, and only then start the next phase. If a phase fails its "done when", fix it before moving on; never stack phases.

### Phase 0: Preflight (JD, no Cline)

1. `cutmark` repo: done on Oct 28. `feat/editor-shell` stays a branch, and S0 runs on `main`. Checked: `git status` is clean and the workspace loads.
2. `Audio_Editor` repo: `master` is the source. Checked on Oct 28: `master` is an ancestor of `security-audit`, which is 6 commits ahead (security batches 1 and 2, the rename, the installer), and `master` has nothing `security-audit` lacks. So fast-forward: `git switch master`, then `git merge --ff-only security-audit`. Update Nov 14: the recording branch (version 1.1.0, mic recording) had since moved 8 commits ahead of master, so master was fast-forwarded to recording instead (67ec7e4). Then note `git rev-parse HEAD`.
3. Install the license gate: `cargo install cargo-deny --locked`.
4. Install LLVM (for bindgen) and set `LIBCLANG_PATH` to its `bin` folder.
5. Write down `rustc --version`, and the number of passing tests from `cargo test` in `Audio_Editor` after the fast-forward. The "audio tests carried over" check compares against that number.

**Done when:** both repos are clean, and the three facts are written down. Recorded Nov 14: Audio\_Editor source commit 67ec7e4 (master, fast-forwarded to the recording branch, version 1.1.0), rustc 1.99.1 (2026-10-13), and 656 passing tests in Audio\_Editor.

### Phase 1: Root config

1. `rust-toolchain.toml` pinned to the rustc version from Phase 0.
2. Root `Cargo.toml`: `[workspace.package]`, `[workspace.dependencies]`, `[workspace.lints]` as in the Cargo workspace section.
3. `deny.toml` from the starting point in the licensing section.
4. `CLAUDE.md` and `.clinerules/cutmark.md` with the suite rules.
5. `scripts\check.ps1`.
6. `.cargo/config.toml` with `+crt-static` for the MSVC target (it applies to every app) and the `[env]` block for `FFMPEG_DIR`.
7. `.gitattributes` with `* text=auto eol=lf` and `*.ps1 text eol=crlf`, then one `git add --renormalize .` commit, so line endings stop flipping between Windows tools and show up as whole-file diffs.
8. The two video crates (`apps/video-editor` and `crates/video-ui`) keep their own lint tables. Those tables already hold the baseline plus three FilmCraft allows (`too_many_arguments`, `needless_range_loop`, `type_complexity`), and Cargo does not allow `workspace = true` next to local lint tables. Only new crates and the audio editor use `[lints] workspace = true`. Any other copied crate that fails clippy gets a per-crate `#![allow(...)]` with a comment, not a fix. Known flake: perf\_ui::program\_monitor\_follows\_a\_value\_that\_changes\_every\_pass (crates/video-ui/tests/it/perf\_ui.rs) can time out under full parallel load and passed on rerun. If it recurs, the video track fixes it, not S0.

**Done when:** `scripts\check.ps1` passes on the workspace as it stands today.

### Phase 2: Audio editor moves in

1. `git subtree add --prefix=apps/audio-editor <path to Audio_Editor> <source branch>` with no `--squash`, so history survives.
2. One commit, "format only": `cargo fmt --all` and nothing else.
3. Rename the package and binary to `cutmark-audio`. Fix the paths in `installer/cutmark.iss`, `licenses/`, `scripts/gen_notices.py`, and the README.
4. Join the workspace: inherit `[workspace.package]` and `[workspace.dependencies]`, and set `[lints] workspace = true`.
5. Docs and rules: `docs/` moves to `docs/audio-editor/`. The old CLAUDE.md and .clinerules are replaced by the root `.clinerules/audio-editor.md`, and its memory bank rule reads only two files per task. The duplicate `.cargo/config.toml` and `Cargo.lock` are deleted.
6. cpal conflict: the video editor uses cpal 0.16 and the audio editor 0.18, and their alsa-sys crates both link native alsa, which Cargo refuses in one workspace. An empty stand-in, `patches/alsa-sys`, goes in `[patch.crates-io]`. It is removed when the video editor moves to cpal 0.18.
7. Installer and notices follow the workspace: `build-installer.ps1` and `cutmark.iss` read the exe from the workspace `target` folder, and `gen_notices.py` lists only the `cutmark-audio` dependency tree, read from `cargo tree`.

**Done when:** the audio test count equals the Phase 0 number, `git` merge-base --is-ancestor 67ec7e4 HEAD succeeds (git log --follow stops at the subtree join, so it cannot show the old commits), and the app starts.

### Phase 3: FFmpeg and the LAME swap

1. `scripts\get-ffmpeg.ps1` downloads the pinned LGPL shared build into `third_party\ffmpeg` and checks its hash.
2. `ffmpeg-next` replaces the LAME crate. MP3 export goes through FFmpeg's `libmp3lame`, loaded from the DLLs.
3. `build.rs` copies the FFmpeg DLLs next to the executable.
4. Installer and `THIRD_PARTY.txt` list the DLLs and their LGPL notice.

**Done when:** the MP3 export, cancel, license gate, and installer checks pass. If the prebuilt FFmpeg lacks `libmp3lame`, stop and take the open question below to JD.

**Recorded Nov 14:** the BtbN LGPL shared builds of FFmpeg 8.1 and 9.0 both carry `--enable-libmp3lame` and `--enable-version3`, with no `--enable-gpl` or `--enable-nonfree`, so no source build is needed (open questions 4 and 5 answered). The pinned build is FFmpeg 9.0.2 (`ffmpeg-n9.0.2-57-gd838f9f9ae-win64-lgpl-shared-9.0.zip`, tag `autobuild-2026-11-14-12-37`), licensed LGPL-3.0-or-later and used with `ffmpeg-next` 9.1.0. A backup copy of the zip lives in `cm_suite\ffmpeg-archive`, because BtbN keeps daily tags only briefly. The mp3 muxer runs with `id3v2_version 0`, so files start at the first MPEG frame as before. The Info tag test now accepts FFmpeg's `Lavc` encoder string in place of `LAME`. `assert_no_alloc` is BSD-1-Clause, which goes on the allow list.

### Phase 4: `marks` skeleton

1. `crates/marks` with an empty `lib.rs`, the dependencies the Marks Sidecar spec names, and one smoke test.
2. Add it to the workspace member list and to the `-p` list in `scripts\check.ps1`.

**Done when:** `cargo test -p marks` passes locally. Building the crate itself continues in Marks Sidecar phases 1 to 3.

**Recorded Nov 14:** feature unification breaks a workspace-wide build. `cutmark-audio` enables eframe `glow`, the video crates use `wgpu`, and with `glow` on, `eframe::App::on_exit` takes a second parameter, so `cutmark-video-ui` stops compiling. So `scripts\check.ps1` runs clippy and tests once per package, and CLAUDE.md rule 15 bans building the audio and video crates in one cargo command. Moving the audio editor to wgpu, giving the suite one renderer, is an S1 decision. `marks` carries its own lint table (the baseline plus the strict set), because Cargo does not allow `workspace = true` next to local tables.

### Phase 5: CI and close

1. Check the blocker in the CI section. If `apps/cutmark-editor` still has path dependencies outside the repo, skip steps 2 to 4 and note it.
2. Add `.github/workflows/ci.yml` as written above, plus a step that gives git a read-only token from a repository secret before any cargo command (the filmcraft repo is private).
3. JD adds the read-only filmcraft token as a repository secret, sets the Actions budget to $0 with stop-at-limit on, then starts the workflow once by hand.
4. Fix whatever it finds, locally first, and run it once more only if needed.
5. Run all twelve checks in the Test plan, then `git tag s0`.

**Done when:** the Test plan passes. With the blocker present, the manual CI check is the one check allowed to stay open, and the tag notes that.

**Closed Nov 14:** all twelve checks passed. A fresh clone with `get-ffmpeg.ps1` builds `cutmark-audio` and `cutmark-video`, `check.ps1` is green, the manual CI run is green on `main`, the installer ships the FFmpeg DLLs and notices, and both release apps launch. `s0-workspace` was fast-forwarded into `main` and tagged `s0`. Decided Nov 14: the audio editor moves to wgpu, so the suite has one renderer. That lands as a follow-up right after the tag, and CLAUDE.md rule 15 then becomes "wgpu only, never glow". Still open, for later: fixing the `perf_ui` timing tests (video track).

## Conflicts, deferred items, and open questions

**Conflicts found on disk, and how this spec settles them**

| Where | What disagrees | Decision |
| --- | --- | --- |
| Overview vs disk | The Overview's folder layout does not match the repo: `apps/cutmark-editor`, `apps/host-spike` and `crates/video-ui` exist on disk. | The Folder layout section here is the reference. The Overview is updated to match once this spec is approved. |
| `apps/cutmark-editor` | It depends on FilmCraft by path, outside the repo. The suite rule is the pinned git revision. | It exists only on feat/editor-shell, so S0 on main is not affected (checked with git grep on Oct 28). Before that branch merges, it must switch to the pinned git revision (see CI). |
| `feat/editor-shell` | The branch holds the editor shell and spikes, and is not merged. | Decided Oct 28: it stays a branch and S0 runs on main. It comes back after S0 by merge. |
| Audio\_Editor branches | `security-audit` and `master` were thought to have diverged. Checked on Oct 28: security-audit is 6 commits ahead of master and master has nothing it lacks. | Phase 0, step 2. master is the source, fast-forwarded to security-audit first so no work is lost. |
| Clippy on FilmCraft copies | Workspace lints with `-D warnings` fail on code we do not own. | The two video crates keep their own lint tables (Phase 1, step 8). Any other copied crate that fails gets a per-crate `#![allow(...)]` with a comment. We do not edit copied code. |
| Static CRT | The audio editor has `+crt-static` today. In the shared `.cargo/config.toml` it hits every app. | Test plan checks the video apps. If one cannot build or run, the suite goes back to the dynamic CRT and the installers bundle the VC++ redistributable. |
| `CLAUDE.md` rule 7 | "No GPL or LGPL dependencies" contradicts rule 1 once FFmpeg DLLs are in. | Reworded (see Licensing and dependency policy). |
| Recorder v1 spec | It says `%APPDATA%\{app}\`. The suite path is `%APPDATA%\Cutmark\{app}\`. | Corrected in the Recorder v1.2 update. |
| Old audio editor name | The package, binary, installer and docs still use the old name. | Phase 2, step 3 renames all of them. The rename check in the Test plan searches for leftovers. |

**Deferred, with the milestone that picks each up**

1. **`cutmark-hotkeys` and `media-io`** are created at S2. S0 only reserves the names and the shared dependency table.
2. **Suite-wide notices tool.** The audio editor's `gen_notices.py` stays until it is decided at R.
3. **Windows CI runners.** Windows-only code is covered by `scripts\check.ps1` until a paid budget is approved.
4. **How TuteTool consumes suite crates** is decided in the TuteTool track at T2. S0 keeps crate names and `[workspace.dependencies]` stable so that decision stays cheap.
5. **FFmpeg hardware encoders** join the build line at S2.
6. **`apps/video-editor`, `apps/cutmark-editor`, `apps/host-spike`** stay untouched in S0 including their lint tables.

**Open questions**

| # | Question | Who answers | Blocks |
| --- | --- | --- | --- |
| 1 | Which Audio\_Editor branch is the source? **Answered:** `master`, fast-forwarded to `security-audit` first (Phase 0, step 2). | JD, done | Nothing |
| 2 | Merge `feat/editor-shell` into `main` now, or leave it as a branch? Answered: left as a branch; S0 runs on main. | JD | Phase 0 |
| 3 | Is `jdfigura-montana/filmcraft` private? **Answered:** yes, so CI needs a read-only token stored as a repository secret. | JD, done | Phase 5, CI only |
| 4 | Does the prebuilt LGPL FFmpeg include `libmp3lame`? If not, we build from source in MSYS2. | Phase 3 finds out | Phase 3 |
| 5 | Which `ffmpeg-next` version matches the chosen FFmpeg major? | Phase 3 finds out, recorded in `BUILDINFO.txt` | Phase 3 |
| 6 | Do the video apps build and run with the static CRT? | Partly answered Oct 28: cutmark-video builds with the static CRT (Phase 1, prompt 1a). Running it is still a Test plan check. | Close of S0 |

Questions 1, 2 and 3 are answered. The rest are settled by the phase that hits them, and none of them stops this spec from being approved.
