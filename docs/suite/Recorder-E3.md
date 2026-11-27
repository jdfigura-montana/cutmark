# Recorder adopts E3

Nov 26, 2026. Applies Design-Language-E3.md to the Recorder. Layout and look only: no new features, no capture or encode changes. Reference board: "E3 · Recorder, Setup" on the Cutmark Look canvas.

## 1. Layout

```
title bar:  menus | Setup Record Review | project name   profile caption | caption buttons
+-----------+-------------------------------------+-------------+
| side      | stage panel (bg.panel, radius 18)   | side panel  |
| panel     |  toolbar: Matte view, Guides        | tool strip  |
| Sources   |  preview (bg.stage.inner)           | Camera Key  |
| Pre-flight|                                     | Layout ...  |
|           |  bottom bar 64 | notch | bottom bar | inspector   |
+-----------+-------------( REC )-----------------+-------------+
status bar: left items      [center 120 empty]      right items
```

- Left and right panels use `side_panel` (bg.side, radius 12, no rim). The R10b resizable dividers and remembered widths keep working; the gap between panels stays the current gutter.
- The preview area becomes `stage_panel`. The preview sits on `bg.stage.inner`.
- The old transport deck in the Recorder is removed. Every action it had moves to the bottom bar, with the same shortcut, menu item, Control API action and Stream Deck action.

## 2. The notch button by mode

| Mode | Notch button | Notes |
| --- | --- | --- |
| Setup | Record (sem.record, white dot) | Starts a take (with countdown if on), same as today. |
| Record | Record while idle; white rounded square while recording | Click stops the take. Disabled states follow today's rules (pre-flight failed, no sources). |
| Review | Play (accent fill) | Plays the selected take; Pause while playing. Disabled with no take selected. |

Decision: Review uses Play because recording from Review is not an action today. If Review has no player in the stage, keep Record there and note it.

## 3. Bottom bar

- Left: `timecode_large` (elapsed while recording or playing, 00:00:00 otherwise), then the next most used control (Add mark while recording; in Setup, the next take name in Martian Mono with its label "Next take").
- Right: the remaining deck controls (Countdown, Pause if present, Stop for playback in Review) as `bar_button`s, plus small readouts.
- Center 112 stays empty. When the bar gets narrow, hide right-side readouts first, then shrink the timecode (same rule as the audio editor's per-side hiding).

## 4. Status bar

- `status_bar` with the 120 center gap.
- Items are separate, 20 apart, no " · ": "Ready", "161.6 GB free", "About N at this profile", warnings (for example "Camera falls back to HEVC on this GPU"), "Control API: no clients".

## 5. Labels and strings

- Sentence case everywhere; no uppercase or letter-spaced labels (panel headers, group labels, tool strip).
- No " · " joined strings. Resolution and rate show as separate items ("2560×1440" and "60 fps"), stat blocks in Pre-flight (value in Martian Mono 15, label in caption under it).
- Title caption: one fact, the profile name.
- Tool strip selected state: bg.control, text.primary, 2px accent bottom line.
- Every number a user reads as data uses Martian Mono (levels, dB, resolutions, rates, sizes, durations, offsets, percentages, hex).

## 6. Help

Recorder help text that mentions the deck or transport bar is updated to describe the bottom bar and the notch button.

## 7. Done when

- Setup, Record and Review match the board's structure with tinted Teal neutrals and the new fonts.
- Every former deck action works from the bar, its shortcut, the menu, the Control API and Stream Deck.
- No clipping at the 640 minimum window width (the clipping fix tests still pass).
- `.\scripts\check.ps1` is green.
