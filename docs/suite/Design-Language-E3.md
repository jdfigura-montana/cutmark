# Cutmark Design Language: E3 amendment (character pass)

Nov 25, 2026 · @JD · approved from the board "E3 · Recorder, Setup (proposal)" on the Cutmark Look canvas.

Applies on top of Design-Language.md, Design-Language-rev3.md and the accent-per-app amendment. Where they conflict, this file wins.

## Summary

Four changes give the suite its own character while the work area stays calm:

1. One signature: the cut-mark notch holding the app's primary transport button.
2. Neutrals tinted toward each app's accent.
3. New type: Instrument Sans for the UI, Martian Mono for numbers.
4. No generic tells: sentence-case labels, no dot-joined strings, radius by hierarchy.

## 1. The notch (replaces the floating transport deck)

- Each app's main content panel (the "stage": the timeline in the audio editor, the preview in the Recorder, the monitor in the video editor, the script in the Prompter) ends in a 64 tall bottom bar inside the panel, separated by `stroke.hairline`.
- A round notch is cut out of the stage panel's bottom edge, centered: radius 42.
- The primary transport button sits in the notch: a 68px circle centered horizontally, with its center 8px above the panel's bottom edge, so it breaks out of the frame. Editors: Play (accent fill, `accent.on` glyph, switches to Pause while playing). Recorder: Record (`sem.record` fill, white inner dot; while recording, a white rounded square). Soft colored shadow: 0, 8, blur 24, the button color at 25%.
- Bottom bar layout: three columns, the center 112 wide and empty for the notch. Left: the timecode (see Type) and the app's next most used control (Add mark, back to start). Right: the remaining transport controls and their small readouts. Secondary buttons are 40px circles, `bg.control`, hover `bg.hover`.
- The status bar under the panels is 30 tall and keeps its center 120 wide empty, so the button never overlaps status text.
- The floating glass deck is retired. The glass recipe stays for menus, popovers and toasts.
- The app mark (logo placeholder) keeps the notch motif; its dot uses the app's accent.

## 2. Tinted neutrals

Every neutral (surfaces, strokes, text) leans slightly toward the current accent's hue, so each app has its own temperature: the Recorder feels faintly teal, the audio editor faintly blue, the video editor faintly warm.

- Neutrals are defined as OKLCH lightness and chroma in `tokens.json`, with the hue taken at runtime from the app's accent (`Theme`). A gray accent pick (chroma below 0.02) gives untinted neutrals.
- Values (L, C), with the Teal results from the board. These are the final numbers, tuned in E3 phase 1: with the Teal preset every token in the table matches the board's hex value exactly.

| Token | L | C | Teal result |
| --- | --- | --- | --- |
| `bg.ground` | 0.141 | 0.006 | `#070A0A` |
| `bg.side` (new: side panels) | 0.185 | 0.006 | `#101413` |
| `bg.panel` (stage) | 0.199 | 0.007 | `#121716` |
| `bg.stage.inner` (new: area behind the preview or monitor) | 0.167 | 0.006 | `#0C100F` |
| `bg.group` | 0.235 | 0.009 | `#19201F` |
| `bg.control` | 0.259 | 0.011 | `#1E2625` |
| `bg.hover` | 0.302 | 0.0155 | `#263130` |
| `bg.selected` | 0.356 | 0.020 | `#30403E` |
| `text.primary` | 0.963 | 0.0075 | `#EDF4F3` |
| `text.secondary` | 0.809 | 0.0135 | `#B7C4C2` |
| `text.tertiary` | 0.630 | 0.0165 | `#7F8D8B` |
| `text.disabled` | 0.458 | 0.016 | `#4E5B59` |

- Strokes: `stroke.hairline` = `text.primary` at 7% alpha; `stroke.separator` at 5%; other strokes keep their alpha on the tinted base.
- The other track-area and row tokens from rev 3 derive the same way (keep their rev 3 lightness, chroma 0.008).
- Contrast rule unchanged: readable text 4.5:1. A test checks every preset and a gray pick.

**Meaning colors** (fixed, not tinted, no longer Apple's exact values): `sem.record` `#FF4D3D`, `sem.meter.ok` `#34D17A`, `sem.meter.warn` `#F2C94C`, `sem.meter.clip` `#FF4D3D`, `sem.success` `#34D17A`. Marker and pending colors unchanged.

## 3. Type

- **Instrument Sans** (SIL OFL) replaces Inter for all UI text: Regular, Medium, SemiBold.
- **Martian Mono** (SIL OFL) replaces JetBrains Mono: Regular and Medium. It carries every number a user reads as data: timecode, levels, dB, resolutions, frame rates, sizes, durations, percentages, hex colors, take names that are counters. Never used for labels or sentences.
- Number style: letter spacing -0.01em at 11 to 12px; the timecode in the bottom bar is 22px Medium, -0.03em, with the frames part in `text.disabled`.
- Type scale names stay; only the families change.

## 4. No generic tells

- **Labels:** sentence case only. Group labels are `type` 12 / 600 `text.secondary` with 16 above and 7 below. No uppercase, no letter-spaced labels anywhere.
- **No dot-joined strings.** Show separate facts as separate items: in rows, a name with a second line of values separated by spaces; in status bars, items 20px apart; for metrics, small stat blocks (value in Martian Mono 15, label in `type.caption` under it). The title bar caption shows one fact (for example the profile name), not a chain.
- **Radius by hierarchy:** stage panel 18 with the hairline rim; side panels 12, `bg.side`, no rim; groups 8; controls as before. Nested shapes still follow the concentric rule.
- **Selected tool in a tool strip:** `bg.control` fill, `text.primary`, and a 2px accent line along its bottom edge.
