# S1 Phase 14: Help system (spec)

Nov 22, 2026 · @JD · draft for approval

Phase 14 of S1 becomes Help. The close-out (version 1.2.0, installer, week of use, CI, tag) moves to Phase 15.

## Overview

People need to learn what every part of Cutmark Audio does without leaving the app. Phase 14 adds three things:

1. **Help window**: searchable topics, offline, opened from the Help menu or F1, with context help for the area you are in.
2. **Shortcut sheet**: every keyboard shortcut, generated from the app's real key bindings so it can never go stale.
3. **First-run tour**: a short spotlight tour the first time the app opens, repeatable from the Help menu.

The viewer, the shortcut sheet and the tour engine live in `ui-kit`, so the Recorder, Prompter and video editor get them by supplying only their own topics, shortcuts and tour steps.

**Goals**

- A new user can find how to do any task in under a minute.
- Help text matches the app exactly: same labels, same shortcuts.
- Shortcuts are defined once and used for input, menus and the sheet.

**Non-goals (v1)**

- Online docs, a website or video links.
- A tooltip pass over every control (existing tooltips stay).
- Languages other than English.
- Images or screenshots inside topics.

## Help window

- Opened by Help > Help topics (F1). F1 opens the topic for the area that has focus or the pointer (timeline, media panel, master panel, a dialog); anywhere else it opens "Getting started".
- An egui window inside the app, 760 x 560 default, resizable, remembers size and position, closes with Esc. Panel look (`bg.panel`, radius 18, hairline rim).
- Left column (240): search field on top, then topics grouped by section, as list rows. Typing filters live; Enter opens the first match; Up and Down move through results.
- Right side: the topic, rendered from Markdown, scrollable. Links to other topics work (`[Recording](topic:recording)`). Back and Forward icon buttons above the content (Alt+Left, Alt+Right).
- Search: matches title, keywords and body text, case-insensitive, titles and keywords ranked first. Matching words are highlighted in the result list.

### Markdown subset

Rendered by a small renderer in `ui-kit` (no new crate), styled only from tokens: headings (#, ##, ###), paragraphs, bullet and numbered lists, **bold**, *italic*, `inline code`, keyboard keys as `<kbd>Ctrl</kbd>` (drawn as small rounded key caps), links to topics, a tip block (`> Tip: ...`, drawn as an inspector group), and horizontal rules. Anything else renders as plain text.

### Topic files

- Markdown files in `apps/<app>/help/`, embedded at build time (no files to ship).
- Front matter on each file:

```
---
id: recording
title: Recording
section: Basics
order: 30
keywords: record, arm, microphone, input, take
context: timeline.record
---
```

- `context` ties a topic to F1 areas. Apps tag areas with `ui_kit::help::context(ui, "timeline")` (more specific tags win, for example `timeline.record`).
- A build check (test) fails if two topics share an id, a link points to a missing topic, or a context tag used in the app has no topic.

## Shortcut sheet

- Help > Keyboard shortcuts (Ctrl+/) opens the help window on a generated "Keyboard shortcuts" page: a filter field, then one table per group (File, Edit, Playback, Timeline, Tracks, View, Help) with the action name and its keys as key caps.
- **One source of truth:** each app defines its shortcuts in one table (`ui_kit::shortcuts::Shortcut { id, label, group, keys, context }`). The app's input handling, its menus (the shortcut text shown next to menu items) and the sheet all read that table.
- For the audio editor this means moving today's scattered key handling (inventory section 7) into `apps/audio-editor/src/shortcuts.rs`. Same keys, same actions, same conditions (for example "not while recording").
- Tests: every shortcut in inventory section 7 exists in the table; no two entries share keys in the same context; every menu item with a shortcut shows the table's keys.

## First-run tour

- Shown once, the first time the app opens a project (not on the start screen). Re-run from Help > Show the tour.
- Spotlight: the window dims (`bg.ground` at 70%) except a rounded cutout around the step's target area; a glass callout card next to it with a title (`type.headline`), one or two sentences, "Step 2 of 5", and Back, Next (primary) and Skip tour. Esc skips; Enter or Right is Next; Left is Back. Respects reduce motion (no slide animation).
- Targets are registered by id: `ui_kit::tour::anchor(ui, "transport", rect)`. A step whose target is not on screen is skipped.
- "Seen" flag stored per app in `%APPDATA%\Cutmark\<app>\help.json` (same folder rules as `appearance.json`), with the tour version, so a new tour version can show again later.

**Audio editor tour (5 steps)**

1. Media panel: "Import audio files here, then drag them onto a track."
2. Tracks: "Each track has Mute, Solo and Record arm. Arm a track, then press Record to record onto it."
3. Transport: "Play, stop and record from here. Drag the bar sideways if it covers something."
4. Timeline toolbar: "Undo, Redo, Ripple and Snap on the left; waveform height and zoom on the right."
5. Menu bar: "Everything else lives in the menus. Press F1 any time for help on what you are looking at."

## Help menu (audio editor)

Help topics (F1), Keyboard shortcuts (Ctrl+/), Show the tour, separator, then the items the Help menu has today (for example About). No item is removed.

## Content

**Who writes:** Cline drafts every topic from the code (so names and shortcuts are exact), following the outline and style guide below; JD edits the Markdown files; a final pass fixes anything the edits break.

**Style guide**

- Second person, plain words, short sentences. No em dashes.
- Name controls exactly as labeled in the app, in bold (**Export**, **Snap**).
- Tasks as numbered steps; each step one action.
- Shortcuts as key caps (`<kbd>Ctrl</kbd>+<kbd>Z</kbd>`), always taken from the shortcut table.
- Each topic: one-line summary, then "How to" sections, then a Tip block if useful. Under about 400 words.
- Only describe what the app does today.

**Topic outline (audio editor)**

| Section | Topics |
| --- | --- |
| Basics | Getting started; Projects (new, open, save, the project folder); The window (menu bar, panels, status bar) |
| Recording | Recording a take; Choosing the input and output device |
| Editing | Importing audio; Working with clips (select, move, trim, split, fades, gain); Ripple and Snap; Markers; Undo and Redo |
| Tracks | Tracks (add, reorder, rename, height, Mute, Solo, Record arm, volume, pan) |
| Playback | Playback and the transport; Zoom and navigation |
| Mixing | Master and levels (meters, clip lights) |
| Export | Exporting (WAV and MP3, settings) |
| Settings | Appearance (accent color, glass effects) |
| Help | Keyboard shortcuts (generated page); Troubleshooting (no sound, device changes, recovery) |

## Build phases

| Phase | Delivers | Layer |
| --- | --- | --- |
| 14a | `ui_kit::help`: topic model, front matter parser, Markdown subset renderer, search, help window, context tags, link and id checks | ui-kit |
| 14b | `ui_kit::shortcuts` (table type, key-cap widget, generated sheet page) and `ui_kit::tour` (anchors, spotlight, callout, seen flag) | ui-kit |
| 14c | Audio editor `shortcuts.rs`: all key handling moved onto the table, menus read shortcut text from it | audio editor |
| 14d | Audio editor wiring: Help menu items, F1 with context tags on each area, the 5 tour steps, `help.json` | audio editor |
| 14e | Topic drafts in `apps/audio-editor/help/` (Cline, from the code) | content |
| 14f | JD edits the topics; Cline fixes broken links, checks and anything the review finds | content |

Each phase gets its own Cline prompt (or two if it is too big), written after the previous one is committed. The gallery gets a small Help demo page in 14a and 14b for testing.

## Acceptance

1. F1 in the timeline, media panel, master panel and an open dialog each opens the right topic.
2. Searching "record", "mp3", "snap" and "undo" finds the right topic first.
3. The shortcut sheet lists every shortcut the app responds to, and pressing each listed key does what the sheet says.
4. A fresh profile shows the tour once; Skip and finishing both mark it seen; Help > Show the tour shows it again.
5. Every topic link works; no topic describes a feature the app does not have.
6. Test count at or above the Phase 13 count; `scripts\check.ps1` green.

## Open questions

1. About box: does the Help menu have one today? If not, add a simple About (app mark, version, licenses link) in 14d, or leave it for Phase 15.
