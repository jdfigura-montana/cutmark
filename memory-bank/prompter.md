# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done (Phase 1, V4 on-screen check passed)
- V1, V2: crates/prompter-platform (os::check_windows_11; view::ViewWindow on NativeWindow(isize); PlatformError; non-Windows stubs return Unsupported) and apps/prompter (cutmark-prompter).
- V3: renderer.rs (DX12, DxgiFromVisual, logs surface alpha modes; PreMultiplied on JD's PC) and view.rs (one Floating view with sample text, drag bar, window setup, flag repair). White backing fix via clear_backing (JD approved the platform change).
- V4: Library shell in app.rs (title bar, Scripts, editor and Run panels, status bar, Appearance and About dialogs). menu.rs holds the menu list and enable rules. View controls are in the Run panel. Minimum window 900 x 480.

## State
- Phase 2 C1 done: crates/prompter-core. script::{parse, sections} and the block model; limits::{MAX_WORDS, split_at_heading}. Deps: pulldown-cmark (no default features), serde.
- C1b done. Mark "type: text" takes the category and keeps only the text; other labels are kept as written, first word may name the category. "B-roll" and "b_roll" both work.
- MarkCategory serializes like marks::MarkType (snake_case). It has no System: marks says the app writes that one.
- ListItem: `continues` is true after a cue split (draw no marker). `start` is set only on an ordered list's first marked block; an item with no text uses up its number.
- Brackets in code never open or close a cue; a cue outside code may hold code. A cue ends at the last `]]` of a run, so `[[pause]]]` is plain text.
- Cues in a heading do not split it. List depth starts at 0.
- Open: a second paragraph in a loose list item (no cue) is a ListItem with continues false, so it would draw its own marker.
- C2 done: index::{WordIndex, Token, FORMAT_VERSION}; rules in index/normalize.rs. Script now has source_hash (std hash of the Markdown). WordIndex::is_current checks a cache; bump FORMAT_VERSION when parsing or normalizing changes. index_10k: 9.6 ms in release (10,010 typed words, 8,151 tokens).
- C3 done: store::{Store, Library, ScriptMeta, SnapshotMeta, Imported, StoreError}; .txt escaping and the split in store/import.rs. New deps: serde_json, uuid, chrono (now, serde), thiserror (JD approved); dev tempfile.
- Store::open takes the prompter data folder; the app finds it with `directories`. No app code uses the store yet. One Store per folder. A bad library.json is an error and is never overwritten; a failed library write leaves memory unchanged.
- `edited` changes on save and restore only. duplicate adds " copy" and drops favorite, last_position and unknown fields. Unknown fields are kept; views, tabs and overrides have no typed fields yet.
- C3b done: import takes UTF-8 (BOM or not) and UTF-16 with a BOM; anything else, or text with a NUL, is StoreError::ImportEncoding. .txt escaping covers the whole line, only where a character would take effect; [[...]] cues are copied as they are, so Markdown inside a cue still applies.
- Import makes scripts with no folder and numbers split parts "Title (part 1 of 2)". cached_index parses the text to check the hash and ignores cache write errors.
- Tokens: a word may cross spans ("**world**,"), so char_range can end past its first span. source_line_of gives the block's first line. No spoken characters, no token.
- Forms: the aligner must split multi-word forms. Contractions expand inside one token only: typed "do not" (two tokens) gets no "dont", so normalize the heard words the same way. No readings yet for "1990s" (gives "nineteen ninety s"), a minus sign, or other currencies.

## Next
- C4.
- By hand: see-through, flash on open, move to another monitor or DPI, lock and sleep, scroll with the Library minimized, OBS capture.
- Open from V4: only Ctrl+Alt+C shows in the menus (V4 did not read the Hotkeys section). About notices are a short hand-written list.
- scripts\check.ps1 fails in this worktree at clippy cutmark-audio: FFmpeg headers not found.

## White layer findings
- The view's GDI redirection surface is opaque white after a show, under the DirectComposition visual. Who paints it is unknown. Only the area present at show was white.
- The window class background brush already reads 0. PatBlt BLACKNESS on the window DC fixes it, but does nothing on a hidden window.
- An outside hide and show needs a new clear; the once-a-second read-back catches it.
- A faint one-frame flash on open is not ruled out.

## Notes
- Phase 9 normalize fixes: decades (1990s, '90s), minus signs, currency words for €, £ and ¥.
- winit drops no-activate on every MousePassthrough toggle; view.rs re-applies it.
- egui-winit 0.36.2 ignores StartDrag without focus, so the view moves itself with OuterPosition.
- egui-winit sends Ctrl+Alt+C as Event::Copy, not a C key. app.rs puts the C key back, or menu_bar reads the Alt release as a tap and focuses File.
- The root viewport must be transparent or egui-wgpu uses alpha Auto for every viewport. clear_color keeps the Library opaque.
- To screenshot or pixel-test the Library or a view, use a temporary build with capture exclusion off.
- The shown alpha mode repeats egui-wgpu's rule (it does not expose its choice). Recheck on upgrade.
- "Primary monitor" size is the monitor the Library opens on; the platform crate has no monitor query.
- os::OsError is still separate from PlatformError.
- Prompter tasks never read or edit activeContext.md or progress.md.
- Only prompter-platform imports windows or other OS crates.
- The Windows 11 dialog path has not been run on an older Windows. The Linux stubs have not been compiled.
- Do not use cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile. Check the Cargo.lock diff.
