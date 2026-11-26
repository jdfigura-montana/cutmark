# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done
- Phase 1 (V4 passed): DX12 (PreMultiplied on JD's PC), one Floating view, white backing fix (JD approved), Library shell. Minimum 900 x 480.
- Phase 2, pending JD's check: C1 to C3b core; C4, C4b library, editor, autosave, menus, find; C5 renderer.rs (to share with views), Preview, Snapshots.
- Phase 3: E1 to E4 core and platform; E5 wired E1 to E3, file drop; E6 tabs, favorites, remembered sort and folders.

## Platform (E1)
- ViewWindow::cursor_position (E5, JD approved): winit gives no pointer position during a file drag.
- No session id in the pipe name: a user's second session is not heard. Signaling an elevated copy likely fails (untested).

## Core
- Mark "type: text" takes the category and keeps only the text; other labels are kept as written, first word may name the category ("B-roll" or "b_roll").
- MarkCategory serializes like marks::MarkType, without System (the app writes that one).
- ListItem: `continues` is true after a cue split (no marker). `start` is set only on an ordered list's first marked block.
- Brackets in code never open or close a cue; a cue outside code may hold code. A cue ends at the last `]]` of a run, so `[[pause]]]` is plain text. Cues in a heading do not split it. List depth starts at 0.
- Open: a loose list item's second paragraph (no cue) has continues false, so it draws its own marker. An empty ordered item uses no number ("1. a", "2.", "3. c" shows 1, 2).
- Bump FORMAT_VERSION when parsing or normalizing changes (the index cache).
- Store: one per folder. A bad library.json (also a mistyped field) is an error, never overwritten; a failed write leaves memory unchanged. Open drops bad tab and collapsed entries.
- `edited` changes on save and restore only. duplicate adds " copy", drops favorite, last_position and unknown fields (else kept). No typed views or overrides yet.
- Import: UTF-8, or UTF-16 with a BOM. .txt cues are copied as they are (Markdown inside applies). Imports have no folder; split parts are "Title (part 1 of 2)".
- Tokens: a word may cross spans, so char_range can end past its first span. source_line_of gives the block's first line. No spoken characters, no token.
- Forms: the aligner must split multi-word forms. Contractions expand inside one token only, so normalize heard words the same way.
- convert: htmd only parses HTML; one writer serves HTML, .docx, .odt. Unwritable styles shrink or drop. More paragraphs in an item become hard breaks.
- Not read: .docx styles.xml (bold, italic or lists set only by a style), .odt paragraph-style bold or italic, automatic numbers of .docx and .odt headings. Table paragraphs are kept.

## App
- Data folder: BaseDirs config_dir + Cutmark\prompter (ProjectDirs would add a subfolder).
- Edit menu picks run a frame late (click takes focus) and inject events.
- Autosave runs in App::logic. A failed save retries every 500 ms with one toast. A failed save on close cancels the close once.
- editor.rs restores the selection egui drops on focus loss or right-click.
- egui-winit sends Ctrl+V as Event::Paste, no key press; with no clipboard text, only V's release. editor::PasteKeys pastes on it.
- Tabs: serial keys each tab's egui state. LRU order is not stored: restored tabs count left to right, active last.
- Copies go through app.rs write_copies. Tests use a fake Clipboard.
- Esc clears egui focus before the frame, so find.rs notes focus after each frame.
- A laid block's job text is the block's text (markers excluded), for the token to glyph map.
- Preview follows the cursor's block only (a tall block shows from its top). Not remembered across launches. About 208 px of text at default size.
- UI tests: app/library_tests.rs.
- Not done: stats bar, drag to move scripts and reorder folders (panel still sorts folders by name, not store order).
- Open: Unfiled's collapsed state is not saved (not a folder). No menu items for tab keys. Phase 4: can a live tab be evicted or closed? (now yes).
- Untimed on long scripts: Find, preview, Snapshots counts, rich paste.

## Next
- JD's Phase 2 check; E5 check: second launch focuses the first window; paste from Google Docs, Word, a browser.
- By hand: Explorer drag onto rows (also scaled monitor); Word, Google Docs, LibreOffice imports; E6 tab and star looks, Ctrl+Tab, Ctrl+W, HTML-only Ctrl+V, relaunch; preview, Snapshots, dialogs, right-click menus, Paste with empty and image clipboards, Ctrl+Shift+F, autosave on switch and exit; see-through, flash on open, other monitor or DPI, lock and sleep, scroll while minimized, OBS capture.
- Open: no Preview menu item; most File, Script, View, Help shortcuts missing; About notices hand-written.
## White layer
- The view's GDI surface is opaque white after a show, under the DirectComposition visual (painter unknown), only over the area present at show.
- PatBlt BLACKNESS on the window DC fixes it (class brush is 0), not on a hidden window. An outside hide and show needs a new clear; the once-a-second read-back catches it. A flash on open is not ruled out.

## Notes
- Phase 9 normalize fixes: decades, minus signs, currency words for €, £ and ¥.
- winit drops no-activate on each MousePassthrough toggle; view.rs re-applies it.
- egui-winit 0.36.2 ignores StartDrag without focus, so the view moves itself with OuterPosition.
- egui-winit sends Ctrl+Alt+C as Event::Copy; app.rs puts the C key back, or menu_bar reads the Alt release as a tap and focuses File.
- The root viewport must be transparent or egui-wgpu uses alpha Auto for every viewport. clear_color keeps the Library opaque.
- Screenshots need a temporary build with capture exclusion off.
- Shown alpha mode copies egui-wgpu's rule; recheck on upgrade.
- "Primary monitor" size is the Library's monitor. os::OsError is not PlatformError.
- Untested: Windows 11 dialog on older Windows; Linux stubs.
- No cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile. Check the Cargo.lock diff.
