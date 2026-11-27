# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done
- Phase 1 (V4 passed): DX12, Floating view, white fix, Library shell. Minimum 900 x 480.
- Phase 2 (pending JD's check): C1 to C5.
- Phase 3: E1 to E4 core, platform; E5 wiring, file drop; E6 tabs, folders; E7 drags, menus; E8, E8b styled editor; E9a underline, strike; E9b layout, toolbar, Format menu; E9c hidden markers, "/" menu.

## Platform (E1)
- ViewWindow::cursor_position (JD approved): winit has no pointer position in a file drag.
- No session id in the pipe name: a second session is not heard. Signaling an elevated copy likely fails.

## Core
- Mark "type: text" takes the category and keeps only the text; other labels are kept as written, first word may name the category ("B-roll" or "b_roll").
- Brackets in code never open or close a cue; a cue outside code may hold code. A cue ends at the last `]]` of a run, so `[[pause]]]` is plain text.
- Open: a loose item's second paragraph (no cue) has continues false, so draws its own marker. An empty ordered item uses no number.
- Bump FORMAT_VERSION (index cache) when parsing or normalizing changes.
- `~one~` is text; an open `<u>` ends with its block. Writer: if strike clashes with bold or italic, those drop for the block.
- Store: one per folder. A bad library.json (or mistyped field) is an error, never overwritten; a failed write leaves memory unchanged. Open drops bad tab and collapsed entries.
- `edited` changes on save and restore only. duplicate adds " copy", drops favorite, last_position and unknown fields (else kept). No typed views or overrides yet.
- Import: UTF-8, or UTF-16 with a BOM. .txt cues are copied as they are (Markdown inside applies). Imports have no folder; split parts are "Title (part 1 of 2)".
- Tokens: a word may cross spans (char_range can pass its first span). source_line_of: block's first line. No spoken characters, no token.
- Forms: the aligner must split multi-word forms. Contractions expand inside one token only, so normalize heard words the same way.
- convert: htmd only parses; one writer serves HTML, .docx, .odt. Unwritable styles shrink or drop. More paragraphs in an item become hard breaks.
- Not read: .docx styles.xml and w:dstrike, .odt paragraph styles, automatic heading numbers. Table paragraphs are kept.

## App
- Screenshots: CUTMARK_PROMPTER_SHOW_IN_CAPTURE=1.
- Caret at first text and focus on a tab's first showing only.
- E9c: caret.rs, typing.rs; keys.rs takes editing keys before TextEdit; egui's moves are snapped. IME is not marker-aware.
- No key may make a setext underline (an empty item under text parses as one).
- Open: a letter typed after "**word.**" or editing "[[pause]]" shows the marks; Enter in a quote adds no ">".
- E9b: formatting.rs actions are text in, text out; Editor::format makes one undo step. Format keys run before the editor, only while it has the keyboard.
- Ctrl+Shift+X arrives as Event::Cut.
- markup.rs uses pulldown-cmark (JD approved); keep options and cue rules in step with script.rs. Show Markdown is in editor.txt.
- Typing, 10,301 words (release, headless): 1.9 ms median, 2.3 p95.
- Data folder: BaseDirs config_dir + Cutmark\prompter (not ProjectDirs).
- Edit menu picks run a frame late.
- Autosave runs in App::logic. A failed save retries every 500 ms, one toast; on close it cancels the close once.
- egui-winit sends Ctrl+V as Event::Paste, no key press; with no clipboard text, only V's release. editor::PasteKeys pastes on it.
- Tabs: serial keys egui state. LRU order not stored: restored tabs count left to right, active last.
- Copies use app.rs write_copies; tests fake it.
- Esc clears egui focus before the frame, so find.rs notes focus after each frame.
- Preview follows the cursor's block only (tall: from its top). Not remembered. About 208 px of text at default size.
- Panel: Unfiled first (even empty, if folders exist), then store order. Drag starts at 4 px (egui clicks to 6). No folder moves while filtering; no edge scroll.
- No stats bar.
- Open: Unfiled's collapse is not saved. Live tabs can still close (see Notes).
- Not timed on long scripts: Find, preview, Snapshots, paste.

## Next
- E8 to E9c by hand: click, drag, double-click, IME, format keys (non-US layout too), toolbar and "/" menu look.
- JD's Phase 2 check; E5: second launch focuses the first window; paste from Google Docs, Word, browser.
- By hand: E6, E7 drags, wheel, Shift+F10; Explorer drop (also scaled); Word, Google Docs, LibreOffice imports; HTML-only Ctrl+V; relaunch, preview, Snapshots, dialogs, menus, empty and image Paste, Ctrl+Shift+F, autosave; see-through, flash on open (not ruled out), other monitor or DPI, lock and sleep, minimized scroll, OBS capture.
- Open: no Preview menu item; most shortcuts outside Edit and Format missing; About notices by hand.
## White layer
- After a show the view's GDI surface is white under the DirectComposition visual (painter unknown), only over the area then present.
- PatBlt BLACKNESS on the window DC fixes it (class brush is 0), not on a hidden window. An outside hide and show needs a new clear; the once-a-second read-back catches it.

## Notes
- Phase 4: a Live tab cannot be closed or evicted while a session runs (JD decision).
- Phase 9 normalize fixes: decades, minus signs, words for €, £ and ¥.
- winit drops no-activate on MousePassthrough toggles; view.rs re-applies it.
- egui-winit 0.36.2 ignores StartDrag without focus, so the view moves itself with OuterPosition.
- egui-winit sends Ctrl+Alt+C as Event::Copy (see app.rs).
- Root viewport must be transparent, else egui-wgpu uses alpha Auto everywhere. clear_color keeps the Library opaque.
- Shown alpha mode copies egui-wgpu; recheck on upgrade.
- "Primary monitor" size is the Library's. os::OsError is not PlatformError.
- Untested: Win 11 dialog on older Windows; Linux stubs.
- No cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile. Check the Cargo.lock diff.
