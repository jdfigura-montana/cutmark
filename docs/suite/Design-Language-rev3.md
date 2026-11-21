# Cutmark Design Language: rev 3 amendments (pro layout)

Nov 21, 2026 · @JD · approved from the boards "E2 · Audio editor, pro layout" and "E2 · Start screen" on the Cutmark Look canvas.

These rules replace the matching parts of docs/suite/Design-Language.md. Where they conflict, this file wins.

## Why

The first restyle hid the menus behind a menu button, put Undo and Redo in the title bar, and left the track area without structure. On a desktop pro tool that reads as unfinished. Rev 3 follows desktop pro-app conventions: a real menu bar, a centered document title, and a clearly built track area.

## Title bar (replaces "Main toolbar" and the 52px size)

- Height 44 (`size.toolbar.main` 44). Caption buttons 46 x 44. Hairline (`stroke.hairline`) along the bottom edge.
- Left zone: app mark (18px), then the menu bar: one text button per top-level menu (File, Edit, Track, Clip, View, Help, whatever the app has), `type.body` in `text.secondary`, 28 tall, 10 horizontal padding, radius 7, hover `bg.hover` with `text.primary`. Clicking opens that menu as a glass popover under it; hovering the next item while a menu is open switches menus (standard menu-bar behavior); Alt and the Alt+letter accelerators work.
- Center zone (centered on the window): document title on one line: name in `type.body` 600 `text.primary`, a 6px `text.secondary` dot when unsaved, then a caption in `type.caption` `text.tertiary` (format details, for example "48 kHz · 24-bit · 4 tracks"). With no document open the center shows the app name in `text.secondary`.
- Right zone: app actions (the primary button, for example Export, 30 tall), then a 12px gap, a 1px x 20 separator (`rgba(255,255,255,.10)`, token `stroke.separator.strong`), then the caption buttons. Nothing to the right of Close.
- No menu (hamburger) button anywhere. No Undo or Redo in the title bar.

## Panel toolbar

- Undo and Redo live at the left of the main content panel's toolbar (icon buttons, 30, radius 8, disabled state when there is nothing to undo or redo), followed by a separator, then that panel's toggles (chips). They also stay in the Edit menu with their shortcuts.
- Right side: view controls (waveform height, zoom slider with a magnifier icon, readout, Fit).

## Panel header

Side panels (Media, Master, inspectors) start with a 44 tall header row: title in `type.body` 600 `text.primary` at 14 left padding, icon buttons on the right, hairline along the bottom.

## Track area (new tokens)

| Token | Value | Use |
| --- | --- | --- |
| `bg.track.header` | `#1B1B1E` | Track header column, including the ruler row's corner cell ("Tracks" label in `type.caption` `text.tertiary`) and the Add track row |
| `bg.track.header.selected` | `#26262A` | Header of the selected track |
| `bg.track.alt` | `rgba(255,255,255,.025)` | Every other lane (was .015) |
| `bg.track.empty` | `#111113` | Area below the last track |
| `stroke.track` | `rgba(255,255,255,.07)` | 1px line under every track row and under the Add track row |
| `stroke.track.header` | `rgba(255,255,255,.08)` | 1px divider between the header column and the lanes, and under the ruler |

Rules: the header column is one continuous darker strip from the ruler down to the Add track row; every track row ends in a `stroke.track` line; the lanes of the selected track keep `bg.track.selected`; below the Add track row the area is `bg.track.empty`, so the end of the track list is obvious.

## Start screen

- Same title bar (menu bar with the menus the start screen supports, center shows the app name, no primary button).
- Centered content, max width 1120, two panels side by side (420 and the rest), 16 gap, about 640 tall.
- Left panel (hero): app mark 44, app name `type` 26 / 700 / -0.02em, version in `type.value` `text.tertiary`; behind the top part a decorative waveform band (two layered filled waveforms, `bg.hover` and accent at 55% opacity, whole band at 55% opacity; fixed seed, no animation). Below: action cards, full width, 14 radius, 14 x 16 padding, a 44 icon tile, title `type` 15 / 600, subtitle `type.caption`, shortcut on the right in `type.value`. The primary action card (New project) uses the accent fill with `accent.on` text; other cards use `bg.group` with a hairline rim, hover `bg.hover`. Then a drop zone: 84 tall, 14 radius, 1.5px dashed `rgba(255,255,255,.14)` border, upload icon and the drop hint in `type.caption` `text.tertiary`; highlights with the accent rim while a valid drop hovers.
- Right panel: "Recent projects" title (`type` 15 / 600) with the count on the right; rows 58 tall, radius 10: 40 icon tile (waveform glyph, accent for the selected row), name `type.body` 600, folder path in `type.value` 11 `text.tertiary` (middle-truncated when long), date in `type.caption` on the right, remove button (28) visible on hover or selection; selected row `bg.row.selected`. Footer hint line separated by a hairline: how to open and remove (only keys the app supports).
