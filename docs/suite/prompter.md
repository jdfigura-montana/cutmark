# Cutmark Prompter v1 Spec

Nov 15, 2026 · @JD · Revises the Oct 27 version · Updated Nov 19 (UI stack moved to egui) and Nov 25, 2026 (design language rev 3, accent per app, Help, existing crates)

## Overview

Cutmark Prompter is the suite's standalone teleprompter: capture-invisible prompter views (docked strip, floating window, full screen) that scroll by hand or follow your voice, mirror for glass rigs, and drive or follow the Cutmark Recorder and OBS. The feature benchmark is the Virtual Prompter in Elgato Studio 2.0; this spec matches that feature set and keeps the suite integration on top.

**Goals**

- Prompter views that no screen capture, meeting share or recorder can see, with a click-through toggle.
- Three view kinds: Docked (thin strip under the camera), Floating (resizable, any monitor), Full screen (second display or glass rig). Several views can run at once on one script.
- Two scroll modes: Manual (speed and nudge) and Voice Follow (scrolls to the word you are saying), fully offline, CPU only.
- A Markdown editor with rich paste, wide import, tabs, favorites, search, live word count and read time.
- Readability controls: font, size, colors, spacing, margins and a dyslexia-friendly preset.
- Widgets: session timer, progress, outline.
- Mirror modes for beam-splitter glass rigs.
- One countdown and one transport with the Cutmark Recorder; follows OBS recording state when the Recorder is not in use.
- Script-aware marks: every Recorder mark carries the script line being read, and the video editor can show the script against the timeline.
- Every action reachable by hotkey and by the local API (Stream Deck).
- Fully useful on its own with Zoom, Teams, OBS or any recorder.
- No tiers. One product, every feature included.

**Non-goals (v1)**

- Cloud speech recognition, accounts, telemetry, sync.
- Windows 10, macOS and Linux.
- Collaborative script editing or a full WYSIWYG editor with no Markdown underneath. Scripts are Markdown, edited in a styled editor (see Library window).
- A phone or tablet remote app (the local API covers Stream Deck and scripts).
- Different scripts running at the same time (see Deferred).

**What changed in this revision (Nov 15)**

| Area | Oct 27 spec | This spec |
| --- | --- | --- |
| Windows | Overlay plus one optional Output window | Views: Docked, Floating, Full screen, up to 4 at once |
| Docked mode | None | Thin strip pinned to the top of a monitor |
| Paste | Plain text | Rich paste converted to Markdown (Google Docs, FigNote, Word, browsers) |
| Import | .txt, .md, .docx | Adds .odt and .html |
| Library | List, search, folders | Adds tabs and favorites |
| Editor stats | None | Live word count and estimated read time |
| Typography | Font, size, line height, margins, colors | Adds letter, word and paragraph spacing and a dyslexia-friendly preset |
| Widgets | Status strip, progress rail | Adds session timer and outline panel, each toggleable per view |
| Tiers | Not stated | None |
| Build phases | 11 | 13 |

**What changed on Nov 19**

| Area | Nov 15 | Now |
| --- | --- | --- |
| UI stack | Tauri 2 with React, TypeScript and Tailwind | egui on wgpu through eframe, with `ui-kit`, like every other suite app |
| Windows | Tauri windows | eframe viewports in one process |
| Rich paste | Clipboard HTML read by the webview | Clipboard HTML read by `prompter-platform` |
| Packaging | Tauri NSIS installer | Same installer approach as the other suite apps |
| Start gate | Not stated | Phase 1 starts after S1 is tagged, so `ui-kit` exists |

**What changed on Nov 25**

| Area | Before | Now |
| --- | --- | --- |
| Library window | Framed window with a toolbar | Rev 3 title bar: menu bar, centered document title, Start Prompter as the primary button (see Library window) |
| Accent | Shared suite accent | Prompter's own accent, default Violet, own picker (Design Language accent-per-app amendment) |
| Glass | Glass switch | Always on, no switch |
| Help | None | `ui-kit` Help window, shortcut sheet and first-run tour, same as every app |
| `cutmark-hotkeys` | Built by whichever app gets there first | Already built by the Recorder; the Prompter reuses it |
| Recorder side of the link | Required Recorder spec changes | Already built in the Recorder; Phase 11 tests against it |
| Memory bank | Not stated | Prompter tasks use their own `memory-bank/prompter.md`, not the shared files |
| Worktree | Not stated | Built in `cm_suite\cutmark-prompter` on branch `prompter` |

## Platform and tech stack

Rust in the `cutmark` workspace, with egui on wgpu through eframe and `ui-kit` for the look, the same UI stack as the audio editor, Recorder and video editor. Speech and alignment live in Rust crates that have no UI dependencies.

- **Target:** Windows 11, x64. Windows 10 is not supported; the installer refuses to run there, and the app shows a clear message and exits.
- **Hardware:** CPU only for speech. No GPU dependency for speech, so AMD, Nvidia and Intel machines behave the same. The UI renders on wgpu (DX12), never OpenGL.

**Workspace crates**

| Crate | Responsibility | Depends on |
| --- | --- | --- |
| `prompter-core` | Script parsing, word index, aligner, scroll state machine, import converters, settings types. Pure Rust, unit-tested | none |
| `prompter-platform` | Every OS call behind traits: view window tweaks (capture exclusion, topmost, no-activate), clipboard HTML, single-instance signal, mic capture backend, credential store, OS version check, default fonts. Windows backend only in v1 | none |
| `prompter-voice` | Resampling, VAD, `SpeechEngine` trait, sherpa-onnx engine, model manager; gets mic audio from `prompter-platform` | `prompter-core`, `prompter-platform` |
| `cutmark-prompter` (`apps/prompter`) | eframe app: Library window and view viewports, text renderer, local API server, Recorder and OBS clients | all above, `ui-kit`, `cutmark-hotkeys` |
| `cutmark-hotkeys` | Suite hotkey map shared by every Cutmark app, with per-OS default chords. Already in the workspace (built by the Recorder) | none |

Rules:

1. Only `prompter-platform` may import `windows` or any other OS-specific crate. Everything else stays OS-neutral so a macOS backend can be added without touching it.
2. Only `cutmark-prompter` imports egui and `ui-kit` (S0 dependency rule 2).
3. Every color, size and radius comes from `ui-kit` tokens. No hard-coded values (the Design Language lint applies).

**UI:** egui with `ui-kit` components and the Glass Deck look, set up the same way as the Recorder: `ui_kit::chrome::viewport` for the root window, `ui_kit::apply_for` with the Prompter's `AppAppearance` (`"prompter"`, `Preset::Violet`), and `ui_kit::chrome::install`. Views are drawn by the app's own text renderer on top of `ui-kit` panels. Scrolling is animated in the frame loop: while a view is moving it requests a repaint every frame; when still, it repaints only on events.

**Key crates**

| Job | Crate | Notes |
| --- | --- | --- |
| Shell and windows | `eframe` (wgpu backend), egui multi-viewport | One process; the root viewport is the Library, every view is its own viewport |
| Look | `ui-kit` | Tokens, chrome, menus, buttons, fields, sliders, switches, meters, help, tour |
| Global hotkeys | `cutmark-hotkeys` | Same registration path as the Recorder |
| Single instance | `prompter-platform` (named mutex plus named pipe) | A second launch signals the first to focus the Library, then exits |
| Clipboard HTML | `clipboard-win`, inside `prompter-platform` | Reads the "HTML Format" clipboard entry for rich paste |
| Mic capture | `wasapi` | Same crate as the Recorder; shared-mode capture so both apps can open the same mic |
| Resampling | `rubato` | Device rate to 16 kHz mono for the speech model |
| Speech recognition | `sherpa-onnx` (official Rust API, 1.13.x) | Streaming zipformer transducer, static link on Windows ([docs.rs](https://docs.rs/sherpa-onnx)) |
| Markdown | `pulldown-cmark` | Parsed in Rust so the word index and the rendered text agree |
| HTML to Markdown | `htmd` | Rich paste and .html import |
| Document import | `zip`, `quick-xml` | .docx and .odt are zipped XML |
| Local API | `tokio-tungstenite`, `serde` | Same pattern as the Recorder API |
| OBS client | `obws` | obs-websocket v5 |
| Recent files, paths | `directories` | `%APPDATA%\Cutmark\prompter` |

**View window setup (eframe viewports)**

- `ViewportBuilder`: `with_transparent(true)`, `with_decorations(false)`, `with_window_level(AlwaysOnTop)`, `with_taskbar(false)`, `with_active(false)`, and a unique title per view (for example "Cutmark Prompter View 1").
- Click-through: `ViewportCommand::MousePassthrough(true)`, toggled for all views at once.
- Window handle: use the handle eframe or winit exposes for the viewport where the pinned versions offer one; otherwise `prompter-platform` finds the HWND by the view's unique title on the app's own process.
- Capture exclusion: `prompter-platform` calls `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on each window right after creation, so the window is absent from the captured frame rather than blacked out. The Library and the debug HUD get the same call.
- Never take focus: `prompter-platform` adds `WS_EX_NOACTIVATE` to every view.
- Topmost re-assert: `SetWindowPos(HWND_TOPMOST)` every time a view is shown, since fullscreen apps can push it down.
- Per-pixel transparency on the wgpu surface is the main risk of this stack. Phase 1 proves it on DX12. If it fails, Phase 1 stops and JD picks the fallback for the view windows before Phase 2 starts.

## Windows and views

One Library window where you write and set up, plus up to four prompter views. Every view shows the same script at the same position.

**Session model**

1. A session is one script, one transport, one scroll position and one mic.
2. Up to 4 views can be open at once, in any mix of kinds. At most one Docked view per monitor.
3. Every view parks the same token on its own reading line, so views with different sizes and fonts stay in step.
4. One view is the Primary view (the first opened, or picked in the Run panel). Manual speed is measured on the Primary view; the others follow its reading-line token with a glide.
5. Launching the app a second time focuses the Library. There is never a second process.

**Library window** (Design Language rev 3)

- `ui-kit` window chrome, default 1100 x 720, size and position remembered.
- Title bar (44): app mark, then the menu bar File, Edit, Script, View, Help. Center: the active script's name in `type.body` 600, the unsaved dot, and a caption with word count and read time (for example "1,240 words · 9 min"); with no script open, "Cutmark Prompter" in `text.secondary`. Right: Start Prompter as the primary button, a separator, caption buttons. No hamburger button, no Undo or Redo in the title bar.
- Menus: File (New script, New folder, Import, Export, Duplicate, Delete, Settings, Exit), Edit (Undo, Redo, Cut, Copy, Paste, Paste as plain text, Select all, Find in script, Search all scripts), Script (Start or Pause, Reset to top, Jump to section, Toggle mode), View (views, click-through, show or hide views, Outline, Appearance), Help (Help, Keyboard shortcuts, Take the tour, About). Every item shows its shortcut. Every action is reachable from a menu; icon buttons are shortcuts to menu items, never the only way. The editor and list rows have right-click context menus.
- Left panel (header "Scripts"): script list with search, sort by name or last edited, folders, and a Favorites filter. A star on each script marks a favorite; favorites sort first. Drag and drop: drag scripts onto a folder or Unfiled to move them, drag folders to reorder them; files dropped from Explorer (.txt, .md, .docx, .odt, .html) are imported into the folder they land on.
- Center panel: tab bar of open scripts (up to 12, restored on next launch), a panel toolbar with Undo and Redo at the left, then the editor, modeled on the Elgato Studio prompter editor: a centered text column (max 760 px) on the panel background with no box, a placeholder "Enter text or type '/' for commands" in an empty script, and text formatted as you type (headings, bold, italic, underline, strikethrough, lists, quotes, cues as chips). Markdown markers are hidden everywhere, including the cursor line, and the caret steps over them; View > Show Markdown reveals them dimmed. A formatting toolbar across the top of the editor panel: a paragraph style dropdown (Paragraph, Heading 1, Heading 2, Heading 3, Quote), Bold, Italic, Underline, Strikethrough, Bulleted list, Numbered list, Outdent, Indent, Horizontal rule, Insert cue, Undo, Redo, Find. Typing / at the start of a line or after a space opens a command menu (headings, lists, quote, rule, each cue) filtered as you type. A live preview toggle shows the views' renderer beside it.
- Under the editor: a stats bar with word count and estimated read time (see Scripts).
- Right panel (header "Run"): scroll mode (Manual or Voice Follow), speed, countdown, mic picker with a level meter, the Views list (add, remove, kind, monitor, Primary, per-view font size, mirror and widgets).
- Status bar (26): session state, mode, Recorder and OBS link dots, API clients.
- Appearance (View menu): the `ui-kit` accent picker for the Prompter only, "Reset to default" returns to Violet. No glass switch.
- Autosave on edit, 500 ms debounce.
- Excluded from capture too, so an accidental full-screen recording never shows your script.

**Help**

- The `ui-kit` Help window, keyboard shortcut sheet and first-run tour, built the same way as the audio editor and Recorder (docs/suite/S1-Help.md).
- The shortcut sheet is generated from the Prompter's entries in the suite hotkey map, so it never goes stale.
- Claude drafts the help text, JD edits.

**Tabs and the live script**

- The Run panel acts on the active tab. Switching tabs in Ready loads that script into the views.
- While a session is running, switching tabs changes the editor only. The running script's tab shows a Live badge.
- Edits to the live script apply to the views at once (the index rebuilds on every edit).

**Docked view**

- A thin strip pinned to the top edge of a chosen monitor, meant to sit directly under the camera.
- Width 20% to 100% of the monitor (default 40%), centered, with a horizontal offset for an off-center webcam.
- Height by line count: 1 to 4 lines, default 2.
- The reading line is the top line. Text glides up as you finish each line.
- No drag bar and no status strip. Right end: a state dot and the optional timer. Bottom edge: a 2 px progress bar.
- Position is set in the Views list (monitor, width, offset), not by dragging.
- Countdown replaces the text with a centered number.
- Floats over other windows; it does not reserve screen space.

**Floating view**

- Frameless, transparent, always on top, hidden from the taskbar, hidden from capture.
- Default 760 x 420, centered on the top third of the primary monitor. Movable to any monitor. Size, position and monitor remembered.
- Dark glass panel, opacity 20% to 100% (default 75%), white text.
- Reading line: a thin accent bar plus a soft highlight band. Default position 33% from the top, adjustable 10% to 60%.
- Top drag bar with the script title. Visible only while click-through is off.
- Status strip at the bottom: state, mode, speed (Manual) or confidence dot (Voice Follow), mic level, click-through, connection dots for Recorder and OBS.
- Countdown: large centered number with a ring, counting down, then the state changes.

**Full screen view**

- A borderless full-screen window on a chosen monitor.
- Shows text, the reading line and any widgets turned on for it. No status strip, no controls.
- For a monitor under beam-splitter glass, an external field monitor, or a confidence monitor beside the camera.

**All views**

- Excluded from capture, always on top, never take focus.
- Click-through is one toggle for all views.

**Widgets** (each toggleable per view)

| Widget | Shows | Views |
| --- | --- | --- |
| Session timer | Elapsed time and estimated time remaining | All |
| Progress | Position in the script with section markers: a rail on the right edge (Floating, Full screen) or a bottom bar (Docked) | All |
| Outline | Collapsible left panel listing headings, current section highlighted; click a heading to jump while click-through is off | Floating, Full screen |

Timer rules:

1. Starts when scrolling starts, holds on Pause, clears on Reset.
2. Remaining time: Manual uses remaining distance over current speed. Voice Follow uses remaining words over your pace in the last 60 seconds, falling back to the read-time setting.
3. When the Recorder is linked, elapsed time is the Recorder's recording clock.

**Session states**

| State | Meaning | Leaves by |
| --- | --- | --- |
| Ready | Script loaded, at the top or a set position | Start |
| Countdown | Counting down | Count ends, or Esc / Stop cancels to Ready |
| Scrolling | Manual mode, moving | Pause, Stop, end of script |
| Following | Voice Follow, listening and tracking | Pause, Stop, end of script |
| Lost | Voice Follow could not match speech for the timeout | Speech matches again, or a manual nudge |
| Paused | Frozen in place | Resume (no countdown), Stop |
| Ended | Reached the end marker or the last word | Reset, Stop |

**Gaming setups**

- Docked and Floating views sit above games in borderless or windowed mode. Exclusive fullscreen covers them; for those games, use a Full screen view on a second monitor.
- Views never take focus or steal input from the game, even when shown or toggled by hotkey.
- Virtual mic devices (Wave Link, Voicemeeter, SteelSeries Sonar and similar) appear in the mic picker like any other input.
- Voice Follow targets an FPS impact you cannot measure: speech runs on 2 CPU threads at below-normal priority and never touches the GPU.

## Scripts

A script is a Markdown file plus a word index built in Rust; the views render from that index, so the words the voice engine tracks and the words on screen are always the same words.

**Format**

- Markdown: headings, bold, italic, underline (`<u>text</u>`), strikethrough (`~~text~~`), lists, blockquotes, horizontal rules. Links and images render as plain text.
- Headings are sections: they appear in the Outline, on the progress widget and in the Jump to section menu.

**Import**

| Source | Keeps | Drops |
| --- | --- | --- |
| .txt | One paragraph per blank-line block | n/a |
| .md | Everything in the supported Markdown set | Images, link targets |
| .docx, .odt | Paragraphs, headings, bold, italic, lists | Everything else |
| .html | Headings, bold, italic, lists, blockquotes | Styles, scripts, images, tables (cell text kept as lines) |

Import copies content into a new script. The original file is never touched.

**Rich paste**

1. Ctrl+V in the editor reads the clipboard's HTML when present (Google Docs, FigNote, Word, browsers) and converts it to Markdown with the same converter as .html import.
2. Kept: headings, bold, italic, lists, blockquotes. Dropped: fonts, colors, sizes, images, comments.
3. Ctrl+Shift+V pastes plain text.
4. `prompter-platform` reads the clipboard HTML; conversion runs in `prompter-core`.
5. FigNote is the suite's notes app: rich paste from FigNote works in v1; "Open from notes" for FigNote waits on its local API (see Script sources).

**Stats bar**

- Word count: spoken words only. Notes and cues are not counted.
- Estimated read time: spoken words divided by the read-time pace, default 150 words per minute, adjustable 80 to 220.
- With text selected, both show the selection.
- Updates live with the index rebuild.

**Cues** (double square brackets, so they never collide with Markdown links)

| Cue | Shows as | Behavior |
| --- | --- | --- |
| `[[note: show the settings menu]]` | Dimmed italic line | Not spoken; Voice Follow skips it |
| `[[pause]]` | A small pause glyph | Manual mode stops scrolling here until Resume; Voice Follow ignores it |
| `[[mark: B-roll here]]` | A small flag | When it crosses the reading line, sends a Recorder mark (category from the label prefix, default Note) |
| `[[end]]` | A rule with "End" | Scrolling stops here; state becomes Ended |

**Word index** (`prompter-core`)

1. Parse Markdown with `pulldown-cmark` into blocks (paragraph, heading, list item, quote, cue).
2. Split spoken text into tokens. Each token gets an index, its display text, its block id, and one or more normalized forms.
3. Normalized forms: lowercase, punctuation stripped, contractions kept both ways ("don't", "do not"), digits and symbols expanded with alternatives ("2026" as "twenty twenty six" and "two thousand twenty six"; "%" as "percent"; "&" as "and").
4. Cue blocks and notes produce no spoken tokens.
5. The renderer lays out each block as an egui galley and keeps a map from token index to glyph range, so a token index maps to an exact on-screen position in every view.
6. The index rebuilds on every edit (under 50 ms for a 10,000-word script) and is cached with the script.

**Limits:** 50,000 words per script. Larger files import with a warning and are split at the nearest heading.

## Scrolling

Two modes share one transport: Manual scrolls at a set speed, Voice Follow parks the word you are saying on the reading line. You can switch modes while paused.

**Manual mode**

- Speed 20 to 400 px per second at a 40 px font on the Primary view, default 80, steps of 10. Speed scales with font size so pace in words stays the same when you change the font.
- Speed changes ease over 250 ms, never jump.
- Nudge moves 3 lines back or forward with a 150 ms ease.
- Mouse wheel scrolls while click-through is off. Scrolling by hand while running pauses for 2 seconds, then resumes.
- A `[[pause]]` cue stops scrolling at the reading line until Resume.

**Voice Follow mode**

- Each view moves so the last matched word sits on its reading line, with a smooth glide (critically damped, about 300 ms to settle).
- Silence holds the text still. There is no drift while you think.
- Skipping ahead: if you jump up to 40 words forward, it follows. Larger jumps need 4 matched words in a row.
- Going back: re-reading moves the text back only after 4 matched words in a row, so a repeated common word never yanks the page.
- Ad-libs: unscripted speech is ignored; the text waits where it is.
- Lost: after 6 seconds of speech with no match, the state becomes Lost and the status strip says "Lost: read on or nudge". Any match or a nudge re-anchors it.
- Nudge hotkeys work in Voice Follow too; a nudge moves the anchor, and tracking continues from there.
- Optional **Creep while unsure** (default off): while you are speaking but nothing matches, scroll forward slowly at 15 px per second.

**Countdown**

- 0, 3, 5 or 10 seconds, default 5.
- Runs on Start from the top or after Reset. Resume after Pause has no countdown.
- When the Recorder is linked, the Recorder's countdown setting wins and both apps show the same countdown (see Suite integration).
- In Voice Follow, the mic opens at the start of the countdown so the model is warm; matches before zero are ignored.

**Start position**

- Start runs from the current position. Reset returns to the top.
- Jump to section (menu, Outline and API) moves to a heading and sets the anchor there.
- The last position per script is remembered and offered on the next open.

## Voice Follow engine

Voice Follow runs fully offline: a streaming sherpa-onnx transducer turns mic audio into words, and a script-aware aligner in `prompter-core` turns those words into an anchor position. Audio never leaves the machine and is never written to disk.

**Why sherpa-onnx**

- Apache 2.0, actively maintained, with an official Rust API and static Windows libraries ([docs.rs](https://docs.rs/sherpa-onnx)).
- Streaming zipformer transducers give partial results as you speak, which is what scrolling needs.
- Transducer models support hotwords (contextual biasing), so script terms like product names get recognized ([sherpa-onnx hotwords](https://k2-fsa.github.io/sherpa/onnx/hotwords/index.html)). Hotwords require `modified_beam_search` decoding.
- Vosk was the runner-up (also Apache 2.0, reconfigurable vocabulary), but its development pace is slower and it has no hotword weighting. The `SpeechEngine` trait keeps it swappable.

**Pipeline** (one capture thread, one speech thread)

1. **Capture:** WASAPI shared-mode capture on the chosen mic, 10 ms buffers, downmixed to mono. The capture callback only copies into a lock-free ring buffer.
2. **Resample:** `rubato` to 16 kHz f32.
3. **Voice activity:** Silero VAD (through sherpa-onnx) marks speech and silence. It drives the Lost timer and Creep while unsure; it does not gate the recognizer.
4. **Recognize:** online transducer, `modified_beam_search`, 4 active paths, 2 threads, endpoint detection on. Partial results are polled after every decode chunk.
5. **Align:** new or revised words go to the aligner, which returns an anchor token index and a confidence from 0 to 1.
6. **Publish:** the speech thread sends `VoiceAnchor { index, confidence }` to the app over a channel, at most 30 times a second, and requests a repaint. Each view glides to it.

**Script context (hotwords)**

- Built on script load and on edit: words that are capitalized mid-sentence, contain digits or mixed case, or are 10+ letters long. Up to 500, score 1.5.
- Tokenized at runtime with the model's BPE vocabulary (`modeling_unit = "bpe"`), so no offline conversion step.

**Aligner** (`prompter-core`, pure Rust, no audio)

1. Keep the last 12 recognized words as the hypothesis tail.
2. Search window: 30 tokens behind to 120 tokens ahead of the anchor.
3. Score with local alignment (Smith-Waterman) against the window. Exact match on any normalized form +2, near match +1 (edit-distance ratio up to 0.34 or same phonetic key), mismatch -1, gap -1.
4. Position prior: forward moves beyond 10 tokens cost 0.05 per token; backward moves cost 0.1 per token.
5. Accept the end of the best alignment as the new anchor when it has 3 matched words, or 2 when it is within 8 tokens ahead.
6. Backward moves and jumps over 40 tokens need 4 consecutive exact matches.
7. Partial results can be revised by the recognizer; the anchor never moves back because of a revision alone.
8. In Lost, search the whole script through a trigram index, then resume windowed search.

**Engine trait**

```rust
pub trait SpeechEngine: Send {
    fn set_context(&mut self, hotwords: &[String]) -> Result<()>;
    fn accept(&mut self, samples_16k: &[f32]);
    fn poll(&mut self) -> Option<Hypothesis>; // partial or final
    fn reset(&mut self);
}

pub struct Hypothesis { pub words: Vec<String>, pub is_final: bool, pub t_ms: u64 }
```

**Models**

- v1 ships one English streaming zipformer model, int8, inside the installer. The exact model is picked in the phase 8 benchmark; target under 150 MB.
- Stored in `%LOCALAPPDATA%\Cutmark\models\speech\<model-name>` with a SHA-256 manifest checked on load.
- The model loads when Voice Follow is first armed, not at app start.

**Targets**

| Measure | Target |
| --- | --- |
| Speech to scroll latency, p95 | 300 ms or less |
| Anchor within 3 words of truth, clean read | 95% of speaking time |
| Backward jumps on a clean read | 0 |
| CPU while following | under 10% of an 8-core CPU |
| Model load time | under 2 s from SSD |

**Tuning and debug**

- Every number above is a setting with these defaults, hidden under Advanced.
- Debug HUD (Ctrl+Alt+Shift+D, configurable): recognized text, anchor index, best score, VAD state. Excluded from capture like everything else.

## Display and mirror

Mirror and typography are set per view, so you can read normally on screen while a glass rig gets a flipped image.

**Mirror modes**

| Mode | Use it for | What flips |
| --- | --- | --- |
| Off | Reading on screen | Nothing |
| Horizontal | Beam-splitter glass in front of a camera | Text left to right |
| Vertical | Rigs where the monitor is mounted upside down or reflected top to bottom | Text top to bottom; scrolling runs downward |
| Both | Combined rigs (same as a 180 degree turn) | Both axes |

Rules:

1. In Docked and Floating views, mirroring flips the painted text layer only (its mesh is transformed around the layer's center before it is drawn). Status strip, drag bar, widgets and countdown are never mirrored.
2. A Full screen view mirrors everything it shows, including the countdown and widgets, since it is read through glass.
3. Scroll math, the reading line and the voice anchor work in unmirrored coordinates; the flip is applied last.
4. With Vertical or Both, the reading line position is measured from the bottom of the glass view.
5. Mirror can be changed while running, with no jump in position.

**Typography** (per script, with app-wide defaults; size is per view)

| Setting | Range | Default |
| --- | --- | --- |
| Font | Any installed font, plus bundled OpenDyslexic | Segoe UI |
| Size, Docked | 16 to 96 px | 32 |
| Size, Floating | 20 to 160 px | 44 |
| Size, Full screen | 24 to 240 px | 72 |
| Line height | 1.2 to 2.0 | 1.45 |
| Letter spacing | 0 to 0.2 em | 0 |
| Word spacing | 0 to 0.5 em | 0 |
| Paragraph spacing | 0 to 2 lines | 0.5 |
| Side margins | 0 to 25% | 6% |
| Alignment | Left, Center | Left |
| Text and background color | Any | White on near-black |
| Read words (Voice Follow) | Dim to 40 to 100% | 60% |
| Current word highlight (Voice Follow) | On, Off | On, accent underline |

**Dyslexia-friendly preset**

One toggle that sets the values below and remembers your previous values for when you turn it off.

| Setting | Preset value |
| --- | --- |
| Font | OpenDyslexic (bundled) |
| Alignment | Left |
| Line height | 1.8 |
| Letter spacing | 0.05 em |
| Word spacing | 0.2 em |
| Paragraph spacing | 1 line |
| Colors | Cream text (#F4EDD8) on dark gray (#1E1E1E) |
| Italic | Rendered as bold |

Any value can still be changed by hand after the preset is on. Script text colors are user content, so they are not `ui-kit` tokens; everything around the text is.

**Glass rig setup**

- Full screen view on the monitor under the glass, mirror Horizontal. Close or hide the other views to keep the desktop clear.
- A test pattern button shows a mirrored arrow and the word READ, so you can confirm orientation before recording.

## Suite integration

The Prompter links to one recorder at a time (Cutmark Recorder or OBS), and the Recorder owns the clock: the Prompter reports what you are reading, and the Recorder timestamps it into the take folder next to the marks.

**Transport link setting:** Off, Cutmark Recorder (default when the Recorder is installed), or OBS.

**Cutmark Recorder link** (Recorder local API, `ws://127.0.0.1:4466`, token from Recorder settings)

| Prompter does | Recorder does |
| --- | --- |
| Start pressed in the Prompter | Receives `start`, runs its countdown, emits `countdownStarted`; the Prompter shows that same countdown |
| Shows Countdown, then Scrolling or Following | Emits `recordingStarted` at zero; the Prompter starts moving on that event |
| Pause or Resume (either app) | `pause` / `resume` commands and `paused` / `resumed` events keep both in step |
| Esc during countdown | `countdownCancelled`; the Prompter returns to Ready |
| Recording stops | `recordingStopped`; the Prompter goes to Paused, keeping its place |
| Sends `scriptPosition` when the reading line reaches a new line (max 10 per second) | Writes it with its own clock to `{name}_script.csv` and fills the Script column of every mark |
| Flub hotkey | Sends `mark` with category Retake and the current line as the label |
| `[[mark: label]]` cue crosses the reading line | Sends `mark` with that label |

Line numbers in `scriptPosition` are script lines (blocks in the Markdown), not wrapped lines in any one view.

Options (default off): Stop recording when the Prompter reaches the end; Stop the Prompter stops recording.

**Recorder side (already built).** The Recorder already has the events, the `scriptPosition` command, the take files and the Script column (`apps/recorder/src/api/protocol.rs`, `apps/recorder/src/engine/script.rs`). The Prompter's client follows that code as the source of truth; Phase 11 tests against it, and any mismatch is fixed on the Prompter side unless JD decides otherwise.

**OBS link** (obs-websocket v5, default `ws://127.0.0.1:4455`, password from OBS Tools > WebSocket Server Settings)

- The Prompter owns the countdown. At zero it sends `StartRecord` and starts moving.
- `RecordStateChanged` keeps state in step: Started, Paused, Resumed, Stopped. Starting or stopping from OBS itself also drives the Prompter.
- OBS has no marks, so the Prompter keeps its own log (flubs, mark cues, script positions, timed from the Started event) and writes `<recording name>_prompter_marks.md` and `_script.csv` next to the recording when `RecordStateChanged` reports Stopped with its output path. Same formats as the Recorder, so the editor imports them the same way.

**Stream Deck.** The suite's one Stream Deck plugin (Cutmark Remote Control v1 Spec) drives the Prompter through the local API below. The Prompter's actions get added to that spec before Phase 5.

**Video editor (cross-spec, to add to the Video Editor UI spec)**

- Import, recorder view: a take with `_script.csv` shows a Script stream card, on by default.
- Edit, Markers: a Script column on recorder markers.
- Edit, timeline: an optional Script lane under the ruler showing the line being read at each time; clicking a line moves the playhead there.

**Script sources**

- Local library (default).
- Rich paste from any app, FigNote first (see Scripts).
- Linked Markdown file: a script can follow an external `.md` file and re-import when it changes (one way, with a prompt if the script was edited here). Works with any notes app that saves Markdown.
- Notes target, shared suite setting (None, Markdown folder, FigNote): "Open from notes" lists pages from the chosen target. FigNote support waits on FigNote's local API.

## Hotkeys

Every Cutmark app reads one shared hotkey map, and each chord has exactly one owner, so the Prompter can never steal a Recorder chord again.

**Suite hotkey map** (`cutmark-hotkeys` crate, already in the workspace)

1. One file: `%APPDATA%\Cutmark\hotkeys.json`, listing chord, owner app, action.
2. Each app registers only the chords it owns, as global hotkeys.
3. Every app's settings show the whole map. Assigning a chord another app owns is refused with the owner's name.
4. A chord that Windows refuses (another program holds it) is flagged in settings and in the status strip.
5. A chord that types a character with AltGr on the active keyboard layout (Ctrl+Alt counts as AltGr) gets a warning.
6. Hotkeys are editable in the UI in v1.

**Prompter defaults**

| Action | Default | Notes |
| --- | --- | --- |
| Start / Pause / Resume | Ctrl+Alt+Space | Linked: also starts and pauses the recording |
| Reset to top | Ctrl+Alt+Home | Was Ctrl+Alt+R, which the Recorder owns |
| Speed up / down | Ctrl+Alt+= / Ctrl+Alt+- | Manual mode |
| Nudge back / forward | Ctrl+Alt+\[ / Ctrl+Alt+\] | Both modes; re-anchors Voice Follow |
| Previous / next section | Ctrl+Alt+Page Up / Page Down |  |
| Flub (Retake mark) | Ctrl+F9 | Sits beside the Recorder's F9 marks |
| Toggle Manual / Voice Follow | Ctrl+Alt+V | While paused or ready |
| Toggle click-through | Ctrl+Alt+C | All views |
| Show / hide all views | Ctrl+Alt+H |  |
| Show / hide Docked view | Ctrl+Alt+D |  |
| Toggle Outline | Ctrl+Alt+O | Floating and Full screen views |
| Debug HUD | Ctrl+Alt+Shift+D |  |

**Recorder chords in the same map:** Ctrl+Alt+R start/stop, Ctrl+Alt+P pause/resume, F9 silent mark, Shift+F9 mark with label, Ctrl+Alt+M mute mic. The map file is the source of truth if these have changed.

## Local control API

The Prompter exposes the same kind of local WebSocket API as the Recorder; every hotkey, button and API call goes through one internal command bus.

- **Bind:** 127.0.0.1 only, port 4467 (Recorder is 4466), configurable.
- **Auth:** token shown in settings, sent in the first message. Can be disabled entirely.
- **Clients:** the Cutmark Stream Deck plugin, scripts, other Cutmark apps.

**Commands:** `start`, `pause`, `resume`, `stop`, `reset`, `nudge` (lines, signed), `setSpeed`, `setMode` (manual, voice), `jumpToSection` (index or heading text), `loadScript` (id), `openView` (kind, monitor), `closeView` (view id), `showViews` (all or one view id, on or off), `setMirror` (view id, mode), `setWidget` (view id, widget, on or off), `setClickThrough`, `flub`, `getStatus`.

**Events:** `stateChanged`, `scriptLoaded`, `sectionChanged`, `positionChanged` (line, token, max 10 per second), `viewsChanged`, `voiceLost`, `voiceFound`, `linkChanged` (Recorder or OBS connected or dropped), `warning`.

```json
{ "op": "setMode", "id": 3, "data": { "mode": "voice" } }
{ "event": "stateChanged", "replyTo": 3, "data": { "state": "Ready", "mode": "voice" } }
```

## Data and settings

Everything is local: scripts are plain Markdown files you can back up or open anywhere, and secrets live in Windows Credential Manager, never in JSON.

```
%APPDATA%\Cutmark\prompter\
  appearance.json            the Prompter's own accent (ui-kit)
  scripts\{id}.md            script text
  scripts\.history\{id}\     snapshots, last 10 per script
  library.json               titles, folders, favorites, open tabs, linked files, last positions, per-script overrides
  settings.json              app defaults (below)
  views.json                 each view: kind, monitor, bounds, Primary flag, font size, mirror, opacity, widgets
  windows.json               Library size, position and monitor
  cache\{id}.idx             word index cache, safe to delete
%LOCALAPPDATA%\Cutmark\models\speech\{model}\   speech models
%APPDATA%\Cutmark\hotkeys.json                    suite hotkey map
```

Rules:

1. Script writes go to a temp file, then rename, so a crash never leaves half a script.
2. A snapshot of the script is taken before every Start; the last 10 are kept and restorable from the Library.
3. Mic devices are saved by Windows endpoint ID, with the device name as fallback.
4. Recorder token and OBS password are stored with Windows Credential Manager.
5. No network traffic except the localhost links to the Recorder, OBS and API clients. No telemetry.
6. A view whose saved monitor is missing opens on the primary monitor at its default size.
7. OpenDyslexic ships in the app resources and is added to egui's font definitions at startup; it is not installed into Windows.

**settings.json holds:** default mode, speed, countdown, typography defaults, dyslexia preset state, read-time pace, reading line position, per-kind view defaults, mic device, Voice Follow tunables, transport link and its host and port, API port and on/off, notes target.

## Build process

- Worktree: `cm_suite\cutmark-prompter`, branch `prompter`, its own VS Code window with Cline. Merged into `main` at milestone ends, the same way as S1 and AE 1.3.
- Memory bank: Prompter tasks read and update only `memory-bank/prompter.md` (kept under 8 KB). They never read or edit `activeContext.md` or `progress.md`, which belong to the other tracks and would conflict on merge.
- Prompts follow the Prompt Sizing Rules: one concern per prompt, S or M, `/clear` or a new task between prompts, only the touched crates tested.

## Test checklist

Capture invisibility and click-through are tested first; if any capture tool sees a view, stop and fix it before anything else.

**Capture and views** (Windows 11)

- [ ] Floating view is truly transparent (per-pixel alpha) on wgpu DX12, with no black or opaque background
- [ ] OBS Display Capture and Window Capture: Docked, Floating, Full screen, Library and HUD absent
- [ ] Cutmark Recorder full-monitor capture: all Prompter windows absent
- [ ] Zoom and Teams full-screen share: absent
- [ ] Windows Game Bar and Snipping Tool recording: absent
- [ ] Click-through on: click, type and drag in another app through every view
- [ ] Click-through off: Floating view drags and resizes
- [ ] Showing a view never takes focus from the active app
- [ ] Views stay above a fullscreen browser and a maximized VS Code
- [ ] On Windows 10 the app shows a clear message and exits; the installer refuses to run

**Views and widgets**

- [ ] Docked view pins top-center, honors width, offset and line count, and survives a resolution change
- [ ] Docked, Floating and Full screen open together with different font sizes and stay on the same word in both modes
- [ ] Fourth view opens; a fifth is refused with a clear message; a second Docked view on the same monitor is refused
- [ ] Unplug a monitor with a view on it: the view moves to the primary monitor
- [ ] Timer holds on Pause, clears on Reset, and matches the Recorder clock when linked
- [ ] Outline click jumps all views to that section
- [ ] Second app launch focuses the Library and starts no new process

**Editor and library**

- [ ] Rich paste from Google Docs, FigNote and Word keeps headings, bold, italic and lists; Ctrl+Shift+V pastes plain
- [ ] Import .txt, .md, .docx, .odt and .html; originals untouched
- [ ] Word count ignores notes and cues; read time follows the pace setting; selection shows selection stats
- [ ] 12 tabs open, restored after restart; Live badge on the running script
- [ ] Favorites filter and sort
- [ ] Library follows rev 3 (menu bar, centered title, no hamburger, Undo and Redo in the panel toolbar), passes the `ui-kit` hard-coded value lint and a screenshot review
- [ ] Changing the accent in the Prompter changes only the Prompter; Reset to default returns to Violet
- [ ] Help window, shortcut sheet and tour open from the Help menu; the shortcut sheet matches the hotkey map

**Scrolling and display**

- [ ] A 60-minute script scrolls in Manual with no stutter and no memory growth
- [ ] All four mirror modes on every view kind, changed mid-scroll with no jump
- [ ] Test pattern reads correctly through glass with a Full screen view set to Horizontal
- [ ] Dyslexia preset applies and restores the previous values when turned off
- [ ] Cues: note skipped, pause stops Manual, mark sends a Recorder mark, end stops

**Voice Follow** (automated harness in CI plus manual runs)

- [ ] Harness: 10 recorded reads (WAV plus script plus hand-labeled word times) replayed through engine and aligner; anchor within 3 words of truth for 95% of speaking time
- [ ] Clean reads: zero backward jumps
- [ ] Ad-lib for 20 seconds, then return to the script: re-anchors within 4 words
- [ ] Skip a paragraph: follows within 4 words
- [ ] Re-read a sentence: moves back only after 4 matched words
- [ ] Silence for 30 seconds: no drift
- [ ] Product names in hotwords recognized in a jargon-heavy script
- [ ] Latency p95 of 300 ms or less and CPU under 10% on the dev machine
- [ ] Mic shared with the Recorder: both capture cleanly at once

**Integration**

- [ ] Recorder link: one countdown, recording and scrolling start together, pause and resume both ways
- [ ] Take folder gets `_script.md` and `_script.csv`; marks carry the Script column
- [ ] Flub hotkey lands a Retake mark labeled with the current line
- [ ] OBS link: StartRecord at zero, state follows OBS, prompter marks file written beside the recording
- [ ] Every hotkey works while another app has focus; a chord owned by the Recorder cannot be assigned
- [ ] API test script loads a script, opens a Docked view, switches to Voice Follow, starts, nudges and stops

**Gaming**

- [ ] Docked and Floating views stay above a borderless-fullscreen game; game keeps focus and input while hotkeys fire
- [ ] Exclusive-fullscreen game with a Full screen view on a second monitor works
- [ ] Voice Follow running during a GPU-heavy game: average FPS within 1% of the same run with Voice Follow off
- [ ] Virtual mic from Wave Link selected and followed correctly

## Build phases

Thirteen phases, each ending in a test you can run yourself; each phase becomes one batch of Cline prompts, written one phase at a time per the Prompt Sizing Rules. S1 is closed, so Phase 1 can start. Phases 1 to 10 run alongside the Recorder and AE 1.3 tracks. The Recorder side of Phase 11 is already built.

| # | Phase | Delivers | Done when |
| --- | --- | --- | --- |
| 1 | View spike | `apps/prompter` (eframe on wgpu, `ui-kit`, Violet accent) with a rev 3 Library shell, `prompter-platform` window calls and Windows 11 check, one transparent Floating view, content protection, click-through, no-activate, topmost re-assert | The Capture and views checklist passes for the Floating view, including per-pixel transparency |
| 2 | Scripts | `prompter-core` parsing, cues, word index, normalization; Library panels with list, search, folders, editor, preview, import (.txt, .md), autosave, snapshots | Unit tests pass; a 10,000-word script indexes in under 50 ms |
| 3 | Editor extras | Styled editor (Elgato-style), formatting toolbar and / commands, rich paste (clipboard HTML through `prompter-platform`), import (.docx, .odt, .html), tabs, favorites, drag and drop in the Scripts panel, single instance, stats bar | Editor and library checklist passes |
| 4 | Manual prompter | Galley-based token renderer, Manual scroll, speed, nudge, reading line, countdown, states, status strip, progress, sections | A 60-minute script scrolls with no stutter; cues behave |
| 5 | Hotkeys and API | Prompter entries in the existing `cutmark-hotkeys` map, hotkey settings UI, shortcut sheet, local API, Prompter actions in the Remote Control spec | All hotkeys work with another app focused; the API test script passes |
| 6 | Views and widgets | Docked and Full screen views, Views list, multi-view sync on the Primary view, `views.json`, timer and outline widgets | Views and widgets checklist passes |
| 7 | Display and mirror | Four mirror modes per view, typography settings, dyslexia preset with bundled font, test pattern | Mirror and preset checks pass on screen and through glass |
| 8 | Speech spike | `prompter-voice`: WASAPI capture, resample, VAD, sherpa-onnx streaming; benchmark 2 to 3 English models; record and label 10 harness reads | A model is picked with measured latency, CPU and accuracy |
| 9 | Aligner | Aligner in `prompter-core`, trigram index, offline replay harness in CI | Harness targets met offline, zero backward jumps on clean reads |
| 10 | Voice Follow live | Engine and aligner wired to the views, hotwords, Lost state, Creep while unsure, read-word dimming, debug HUD | Voice Follow checklist passes live |
| 11 | Recorder link | Recorder client against the Recorder's existing API, shared countdown, `scriptPosition`, flub and cue marks, timer on the Recorder clock | Recorder integration checklist passes |
| 12 | OBS link | obs-websocket v5 client, state follow, prompter marks file | OBS checklist items pass |
| 13 | Package and help | Installer built the same way as the other suite apps, with the speech model and font; Help window content and first-run tour (mic, link, hotkeys, first view) | Full checklist passes on Windows 11 from a clean install |

## macOS readiness

macOS ships later, but v1 is built so a macOS backend is an addition, not a rewrite: every OS call lives in `prompter-platform`, and nothing else may touch the OS.

**Built now (v1)**

- `prompter-platform` with traits for view window tweaks, clipboard HTML, single instance, mic capture, credentials, OS check and fonts; Windows backend only.
- Per-OS default chords in `cutmark-hotkeys` (Windows Ctrl+Alt, macOS Cmd+Option), even though only Windows ships.
- No Windows-only assumptions in paths, file formats, API or UI. Paths come from `directories`; the UI never names Windows-only features.
- eframe and wgpu already run on macOS (Metal), so the UI itself needs no port.
- CI builds and tests `prompter-core` and `prompter-voice` on macOS too, so OS-neutral code stays OS-neutral.

**The known macOS blocker**

- Since macOS 15, windows marked as hidden from capture are still captured by ScreenCaptureKit (Zoom, QuickTime, system screenshots, current OBS), and Apple offers no public fix ([details](https://www.producthunt.com/p/jobpath/how-apps-hide-windows-from-screen-capture-and-what-broke-in-macos-15)).
- Plan for macOS: a Cutmark Recorder for Mac excludes Prompter windows through its own ScreenCaptureKit content filter; with OBS or Zoom, the Prompter shows a one-time tip to capture a window or app instead of the whole display, or to use a Full screen view on another monitor.

**macOS phases (later)**

| # | Phase | Delivers |
| --- | --- | --- |
| M1 | macOS platform backend | CoreAudio mic capture, Keychain, pasteboard HTML, window level and "join all spaces" so views float over fullscreen apps, click-through, mic permission prompt |
| M2 | Capture strategy | macOS version detection, capture tip, Full screen view guidance, Recorder-for-Mac exclusion hook |
| M3 | Mac packaging | .dmg, Developer ID signing, notarization, hardened runtime with the audio-input entitlement |

Needs: an Apple Developer account and an Apple Silicon Mac for manual testing. sherpa-onnx already ships macOS libraries for Apple Silicon and Intel.

## Deferred

- Different scripts running at the same time (a second session with its own transport).
- Docked view that reserves screen space so maximized windows sit below it.
- Languages other than English for Voice Follow (model download per language).
- Learned pace: Manual mode that adapts its speed to your measured words per minute.
- Multi-speaker scripts with a color per speaker.
- Phone remote app.
- Rehearsal stats (words per minute, flub count per section).
- macOS (see macOS readiness).

## Open questions

- [ ] Per-pixel transparent views on wgpu DX12: proven in Phase 1, or a fallback picked.
- [ ] Speech model license: confirm the model picked in Phase 8 allows commercial redistribution inside the installer.
- [ ] OpenDyslexic license: confirm it allows bundling in a commercial installer.
- [ ] Code signing certificate for the suite's installers (unsigned installers trigger SmartScreen warnings). Suite-wide decision.
- [ ] Add the Prompter's actions to the Cutmark Remote Control spec (before Phase 5).
- [ ] Add the Script stream, Script column and Script lane to the Video Editor UI spec, or keep them as a later editor phase?
- [ ] Flub default Ctrl+F9: good, or another chord?
- [ ] Second Voice Follow language, if any, for the first commercial release.
- [ ] FigNote local API: needed before "Open from notes" can list FigNote pages.

## Sources

- Elgato Studio 2.0 Virtual Prompter walkthrough (feature benchmark, summarized by JD): [youtu.be/wvWYGpT2Z-g](https://youtu.be/wvWYGpT2Z-g)
- [sherpa-onnx Rust API (docs.rs)](https://docs.rs/sherpa-onnx)
- [sherpa-onnx hotwords (contextual biasing)](https://k2-fsa.github.io/sherpa/onnx/hotwords/index.html)
- [Vosk API repository summary](https://awesome.ecosyste.ms/projects/github.com%2Falphacep%2Fvosk-api)
- [Electron setContentProtection: WDA\_EXCLUDEFROMCAPTURE behavior](https://jetbrains.github.io/kotlin-wrappers/kotlin-electron/electron.core/-browser-window/set-content-protection.html)
- [obs-websocket 5.x: default port 4455](https://github.com/obsproject/obs-websocket/wiki/Notable-changes-between-4.x-and-5.x)
- Desktop Recorder v1 Spec, Design Language rev 3, accent-per-app amendment, S1 Help spec, in this Project and docs/suite
