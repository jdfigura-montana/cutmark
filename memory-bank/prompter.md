# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done
- Phase 1 (V4 on-screen check passed): prompter-platform and apps/prompter: DX12 renderer (PreMultiplied on JD's PC), one Floating view, white backing fix (JD approved), Library shell. Minimum window 900 x 480.
- Phase 2, pending JD's check: C1 to C3b prompter-core; C4 library, editor, autosave, File and Edit menus; C4b find bar and right-click menus; C5 script renderer (renderer.rs, shared with the views later), Preview chip, File > Snapshots.
- Phase 3: E1 prompter-platform instance and clipboard (not wired into the app yet).

## Platform (E1)
- App wiring: poll try_recv with request_repaint_after (a message does not wake egui); log setup_error.
- No session id in the pipe name: in a user's second session the primary hears nothing. Signaling an elevated copy likely fails (untested).

## Core
- Mark "type: text" takes the category and keeps only the text; other labels are kept as written, first word may name the category ("B-roll" or "b_roll").
- MarkCategory serializes like marks::MarkType, without System (the app writes that one).
- ListItem: `continues` is true after a cue split (no marker). `start` is set only on an ordered list's first marked block.
- Brackets in code never open or close a cue; a cue outside code may hold code. A cue ends at the last `]]` of a run, so `[[pause]]]` is plain text. Cues in a heading do not split it. List depth starts at 0.
- Open: a second paragraph in a loose list item (no cue) is a ListItem with continues false, so it draws its own marker. An empty item inside an ordered list uses no number ("1. a", "2.", "3. c" shows 1, 2).
- Bump FORMAT_VERSION when parsing or normalizing changes (the index cache).
- Store: one per folder. A bad library.json is an error and is never overwritten; a failed library write leaves memory unchanged.
- `edited` changes on save and restore only. duplicate adds " copy" and drops favorite, last_position and unknown fields. Unknown fields are kept; views, tabs and overrides have no typed fields yet.
- Import: UTF-8, or UTF-16 with a BOM. .txt cues are copied as they are (Markdown inside applies). Imports have no folder; split parts are "Title (part 1 of 2)".
- Tokens: a word may cross spans, so char_range can end past its first span. source_line_of gives the block's first line. No spoken characters, no token.
- Forms: the aligner must split multi-word forms. Contractions expand inside one token only, so normalize heard words the same way. No readings yet for "1990s", a minus sign, or other currencies.
- C5 added Store::snapshot_text (JD approved the change outside apps/prompter).

## App
- Data folder: BaseDirs config_dir + Cutmark\prompter (ProjectDirs would add a subfolder).
- Edit menu picks run a frame late (the click takes focus) and inject key, Cut and Copy events.
- Autosave runs in App::logic. A failed save retries every 500 ms with one toast. A failed save on close cancels the close once.
- editor.rs restores the selection egui drops on focus loss or right-click. egui cannot read the clipboard (clipboard.rs).
- Esc clears egui focus before the frame, so find.rs notes focus after each frame.
- A laid block's job text is the block's text (markers excluded), for the token to glyph map.
- Preview follows the cursor's block only; a block taller than the pane shows from its top. Not remembered across launches. About 208 px of text at the default window.
- Headless Library tests: app/library_tests.rs.
- Not done: remembered sort and groups, favorites, tabs, stats bar, plain-text paste. The single-instance guard exists (E1) but is not wired in, so two copies could write one store.
- Untimed on long scripts: Find, preview layout, Snapshots word counts (UI thread).

## Next
- JD's Phase 2 check; Phase 3 after E1.
- By hand: preview look and follow, Snapshots dialog, dialogs (Enter, Esc, Import, Export), right-click menus, Paste with empty, text and image clipboards, Ctrl+Shift+F, autosave on switch and exit.
- By hand: see-through, flash on open, move to another monitor or DPI, lock and sleep, scroll with the Library minimized, OBS capture.
- Open: Preview has no menu item (spec wants one for every action; the View list has none). Most File, Script, View and Help shortcuts are missing. About notices are hand-written.
- check.ps1 not run for C4 to C5 (prompts). It fails here at clippy cutmark-audio: FFmpeg headers not found.

## White layer
- The view's GDI redirection surface is opaque white after a show, under the DirectComposition visual (painter unknown), only over the area present at show.
- PatBlt BLACKNESS on the window DC fixes it (class brush is 0), but not on a hidden window. An outside hide and show needs a new clear; the once-a-second read-back catches it. A one-frame flash on open is not ruled out.

## Notes
- Phase 9 normalize fixes: decades (1990s, '90s), minus signs, currency words for €, £ and ¥.
- winit drops no-activate on every MousePassthrough toggle; view.rs re-applies it.
- egui-winit 0.36.2 ignores StartDrag without focus, so the view moves itself with OuterPosition.
- egui-winit sends Ctrl+Alt+C as Event::Copy; app.rs puts the C key back, or menu_bar reads the Alt release as a tap and focuses File.
- The root viewport must be transparent or egui-wgpu uses alpha Auto for every viewport. clear_color keeps the Library opaque.
- To screenshot or pixel-test a window, use a temporary build with capture exclusion off.
- The shown alpha mode repeats egui-wgpu's rule. Recheck on upgrade.
- "Primary monitor" size is the monitor the Library opens on. os::OsError is separate from PlatformError.
- Prompter tasks never read or edit activeContext.md or progress.md.
- Only prompter-platform imports windows or other OS crates.
- Untested: the Windows 11 dialog on older Windows; the Linux stubs are not compiled.
- No cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile. Check the Cargo.lock diff.
