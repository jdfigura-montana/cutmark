# Cutmark mark and app colors

Nov 26, 2026. Decided by JD from the "Cutmark marks" boards on the Cutmark Look canvas. Supersedes the app-to-accent mapping in the accent-per-app amendment, and the "app mark keeps the notch motif" line in Design-Language-E3.md.

## 1. App colors

| App | Default accent (was) |
| --- | --- |
| Audio editor | Orange (Blue) |
| Recorder | Teal (Teal) |
| Video editor | Violet (Orange) |
| Prompter | Blue (Violet) |

- Use the existing ui-kit presets; no new color values.
- These are defaults. A user's own accent pick is kept.
- One-time migration: an appearance.json with no `scheme` field whose accent equals that app's old default moves to the new default. Every appearance.json written from now on has `"scheme": 2`, and the migration never runs again for it.
- Tinted neutrals follow the accent automatically, so the audio editor's neutrals turn faintly warm and the video editor's faintly violet.

## 2. The mark: Splice

A block cut on a slant, with the two pieces nudged apart.

Geometry on a 64 by 64 grid:
- Left piece: rounded rectangle x 6, y 6, w 52, h 48, radius 13, clipped to the polygon (0,0) (34,0) (26,64) (0,64). Ink.
- Right piece: rounded rectangle x 6, y 10, w 52, h 48, radius 13, clipped to the polygon (41,0) (64,0) (64,64) (33,64). The app's accent.
- Ink: `text.primary` on dark surfaces, #141A19 on light ones.
- Smallest size 16 px. Clear space around it: a quarter of its size.
- Never rotate it, outline it, add gradients, change the cut angle or swap the two colors.

## 3. The wordmark

- Bricolage Grotesque ExtraBold (SIL OFL), lowercase `cutmark`, letter spacing -0.045em.
- `cut` in ink, `mark` in the app's accent, with a 0.05em gap; `mark` sits 0.08em lower. This echoes the mark's cut.
- App name after it: Bricolage Grotesque Medium at 0.6 of the size, `text.tertiary`, 0.28em gap, title case ("Audio", "Recorder", "Video", "Prompter").
- Lockup: mark height equals the font size, 0.33em gap between mark and word.
- Shipped as outlined SVG, so the apps don't load the font at runtime.

## 4. Assets

Location: `crates/ui-kit/assets/brand/`. Claude makes these and places them; Cline wires them in.

- `mark-{app}.svg` and `mark-{app}-light.svg`
- `icon-{app}.svg`: rounded tile (radius 23% of its size, fill the app's tinted `bg.side`, 1px inner rim at 6% white), mark at 62% of the tile.
- `icon-{app}.ico`: 16, 20, 24, 32, 40, 48, 64 and 256 px.
- `wordmark-{app}.svg` and `wordmark-{app}-light.svg`: the full lockup with the app name.

`{app}` is `audio`, `recorder`, `video` or `prompter`.

## 5. Where it shows

- Title bar: the 18 px mark replaces the current notch placeholder.
- Start screen and About box: the wordmark lockup.
- Windows exe icon, taskbar and installer: `icon-{app}.ico`.

## 6. Order

1. M1: app color defaults and migration (Cline).
2. Asset pack (Claude).
3. M2: ui-kit brand module, plus the audio editor and Recorder wired to the assets (Cline).
4. The video editor and Prompter pick up the colors and assets in their own tracks.
