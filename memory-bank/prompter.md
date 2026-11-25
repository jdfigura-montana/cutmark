# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done (Phase 1, V4 on-screen check passed)
- prompter-platform and apps/prompter: DX12 renderer (PreMultiplied on JD's PC), one Floating view, white backing fix (JD approved), Library shell. Minimum window 900 x 480.

## State
- Phase 2 C1 to C3b done: crates/prompter-core.
- Mark "type: text" takes the category and keeps only the text; other labels are kept as written, first word may name the category. "B-roll" and "b_roll" both work.
- MarkCategory serializes like marks::MarkType (snake_case). It has no System: marks says the app writes that one.
- ListItem: `continues` is true after a cue split (draw no marker). `start` is set only on an ordered list's first marked block; an item with no text uses up its number.
- Brackets in code never open or close a cue; a cue outside code may hold code. A cue ends at the last `]]` of a run, so `[[pause]]]` is plain text.
- Cues in a heading do not split it. List depth starts at 0.
- Open: a second paragraph in a loose list item (no cue) is a ListItem with continues false, so it would draw its own marker.
- WordIndex::is_current checks a cache; bump FORMAT_VERSION when parsing or normalizing changes.
- Store: one per folder. A bad library.json is an error and is never overwritten; a failed library write leaves memory unchanged.
- `edited` changes on save and restore only. duplicate adds " copy" and drops favorite, last_position and unknown fields. Unknown fields are kept; views, tabs and overrides have no typed fields yet.
- Import takes UTF-8 or UTF-16 with a BOM, else StoreError::ImportEncoding. .txt cues are copied as they are, so Markdown inside a cue still applies. Imports have no folder; split parts are "Title (part 1 of 2)". cached_index ignores cache write errors.
- Tokens: a word may cross spans ("**world**,"), so char_range can end past its first span. source_line_of gives the block's first line. No spoken characters, no token.
- Forms: the aligner must split multi-word forms. Contractions expand inside one token only: typed "do not" (two tokens) gets no "dont", so normalize the heard words the same way. No readings yet for "1990s" (gives "nineteen ninety s"), a minus sign, or other currencies.
- C4 done: script library, editor, autosave, File and Edit menus.
- Data folder: BaseDirs config_dir + Cutmark\prompter, as the Recorder does. ProjectDirs would add a subfolder.
- Edit menu picks run a frame late (the click takes focus from the field) and inject key, Cut and Copy events.
- Autosave runs in App::logic. A failed save retries every 500 ms with one toast. A failed save on close cancels the close once.
- The word count uses WordIndex::build, not cached_index.
- Open: no single-instance guard yet, so two copies could write one store. Not done: remembered sort and groups, favorites, tabs, preview, stats bar, plain-text paste (Phase 3).
- C4b done: find bar, editor and Scripts right-click menus, Edit shortcuts. New script asks for a title (read from the prompt's "same naming dialog"). Headless Library tests: app/library_tests.rs.
- The reported Search focus failure was not reproduced: the old call also took focus headlessly.
- editor.rs restores the selection egui drops on focus loss or right-click.
- egui cannot read the clipboard: clipboard.rs sends RequestPaste when a menu with Paste opens and takes the reply (none if empty).
- Esc clears egui focus before the frame, so find.rs notes focus after each frame.
- Find rescans the text several times a frame while open; not timed on a long script.

## Next
- JD's C4 recheck, then C5.
- By hand, C4 and C4b: dialogs (Enter, Esc, Import, Export), right-click menus, Paste with empty, text and image clipboards, Find on a long script, Ctrl+Shift+F, autosave on switch and exit.
- By hand: see-through, flash on open, move to another monitor or DPI, lock and sleep, scroll with the Library minimized, OBS capture.
- Open: File, Script, View and Help shortcuts are mostly missing (Hotkeys section unread). About notices are a short hand-written list.
- C4 and C4b skipped check.ps1 (prompt). It fails in this worktree at clippy cutmark-audio: FFmpeg headers not found.

## White layer findings
- The view's GDI redirection surface is opaque white after a show, under the DirectComposition visual. Who paints it is unknown. Only the area present at show was white.
- The class background brush reads 0. PatBlt BLACKNESS on the window DC fixes it, but does nothing on a hidden window.
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
- Untested: the Windows 11 dialog on older Windows; the Linux stubs have not been compiled.
- Do not use cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile. Check the Cargo.lock diff.
