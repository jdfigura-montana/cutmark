# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done
- Phase 1 (V4 passed): DX12 (PreMultiplied on JD's PC), one Floating view, white backing fix (JD approved), Library shell. Minimum 900 x 480.
- Phase 2, pending JD's check: C1 to C3b core; C4 to C5 app (views will share renderer.rs).
- Phase 3: E1 to E4 core and platform; E5 wired E1 to E3, file drop; E6 tabs, favorites, folders; E7 drags, multi-select, tab menu items; E8 styled editor.

## Platform (E1)
- ViewWindow::cursor_position (E5, JD approved): winit gives no pointer position during a file drag.
- No session id in the pipe name: a user's second session is not heard. Signaling an elevated copy likely fails.

## Core
- Mark "type: text" takes the category and keeps only the text; other labels are kept as written, first word may name the category ("B-roll" or "b_roll").
- Brackets in code never open or close a cue; a cue outside code may hold code. A cue ends at the last `]]` of a run, so `[[pause]]]` is plain text.
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
- E8: markup.rs uses pulldown-cmark (JD approved); keep its options and cue rules in step with script.rs. Show Markdown is in editor.txt.
- Typing at 10,301 words (release, headless): 2.0 ms median, 2.9 p95; old editor 0.7. Test typing_time (ignored).
- Open: wrapped list rows have no hanging indent (egui has none).
- Data folder: BaseDirs config_dir + Cutmark\prompter (not ProjectDirs).
- Edit menu picks run a frame late (a click takes focus) as injected events.
- Autosave runs in App::logic. A failed save retries every 500 ms with one toast. A failed save on close cancels the close once.
- egui-winit sends Ctrl+V as Event::Paste, no key press; with no clipboard text, only V's release. editor::PasteKeys pastes on it.
- Tabs: serial keys each tab's egui state. LRU order is not stored: restored tabs count left to right, active last.
- Copies go through app.rs write_copies; tests fake it.
- Esc clears egui focus before the frame, so find.rs notes focus after each frame.
- Preview follows the cursor's block only (a tall block shows from its top). Not remembered across launches. About 208 px of text at default size.
- UI tests: app/library_tests.rs. Panel: Unfiled first (even empty, if folders exist), then store order. A drag starts at 4 px (egui clicks to 6). Shift+F10 opens a row's menu. No folder moves while filtering; no edge scroll.
- Not done: stats bar.
- Open: Unfiled's collapsed state is not saved. Live tabs can still close (see Notes).
- Untimed on long scripts: Find, preview, Snapshots, rich paste.

## Next
- E8 by hand: look, click, double-click, IME.
- JD's Phase 2 check; E5 check: second launch focuses the first window; paste from Google Docs, Word, a browser.
- By hand: E7 drags, cursors, wheel, Shift+F10; Explorer drop (also scaled); Word, Google Docs, LibreOffice imports; E6 tabs, stars, tab keys, HTML-only Ctrl+V, relaunch; preview, Snapshots, dialogs, menus, empty and image Paste, Ctrl+Shift+F, autosave on switch and exit; see-through, flash on open (not ruled out), other monitor or DPI, lock and sleep, minimized scroll, OBS capture.
- Open: no Preview menu item; most shortcuts outside Edit missing; About notices hand-written.
## White layer
- The view's GDI surface is opaque white after a show, under the DirectComposition visual (painter unknown), only over the area present at show.
- PatBlt BLACKNESS on the window DC fixes it (class brush is 0), not on a hidden window. An outside hide and show needs a new clear; the once-a-second read-back catches it.

## Notes
- Phase 4: a Live tab cannot be closed or evicted while a session runs (JD decision).
- Phase 9 normalize fixes: decades, minus signs, currency words for €, £ and ¥.
- winit drops no-activate on each MousePassthrough toggle; view.rs re-applies it.
- egui-winit 0.36.2 ignores StartDrag without focus, so the view moves itself with OuterPosition.
- egui-winit sends Ctrl+Alt+C as Event::Copy (see app.rs).
- The root viewport must be transparent or egui-wgpu uses alpha Auto for every viewport. clear_color keeps the Library opaque.
- Screenshots need a temporary build with capture exclusion off.
- Shown alpha mode copies egui-wgpu's rule; recheck on upgrade.
- "Primary monitor" size is the Library's monitor. os::OsError is not PlatformError.
- Untested: Windows 11 dialog on older Windows; Linux stubs.
- No cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile. Check the Cargo.lock diff.
