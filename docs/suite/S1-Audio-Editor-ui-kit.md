# S1 Audio Editor and ui-kit: v1 Spec

Nov 18, 2026 · @JD · rev 2: builds on the existing `ui-kit`

Exported from the Claude Project "Creator Suite". The Project copy is the source; this file is replaced when the spec changes.

## Overview

S1 gives the suite its look and proves it on the first app. It grows the existing `ui-kit` crate (made during Recorder phases 1 and 3) into the full "Cutmark Design Language v1 Spec" (milestones G1 to G4 there), moves the audio editor from glow to wgpu, and restyles the audio editor with `ui-kit` (G5). It is done when the restyled audio editor has been used by JD for one week, every carried-over test passes, the Recorder still builds and passes its tests, and `s1` is tagged.

**What S1 delivers**

1. The audio editor on wgpu, on the workspace's egui and eframe version.
2. `crates/ui-kit` extended: tokens, runtime accent, Inter, window chrome, and every component in the Design Language spec. Its current public API keeps working.
3. `apps/gallery` (package `cutmark-gallery`): every `ui-kit` component in every state, with the accent picker.
4. The audio editor restyled to the Glass Deck look, version 1.2.0, with a fresh installer.
5. As a side effect, the Recorder picks up the new look through `ui_kit::apply`.

**Goals**

1. Same features, new look. Every screen, control and shortcut the audio editor has today still exists and works the same way.
2. No breaking change to the Recorder. Old `ui-kit` names keep compiling until the Recorder moves off them.
3. The look is approved in the gallery before any audio editor code changes.

**Non-goals**

- No new audio editor features and no feature removals. Elements in the mockup that the app does not have today (Edit and Mix switch, effects chain, pan) are not built.
- No bug fixing beyond what the wgpu move and the restyle break.
- No Recorder restyle beyond what `apply` and the shared widgets change. Its layout is the Recorder track's job.
- No macOS window chrome. Windows 11 only in S1.
- No real backdrop blur.

## Inputs

| Input | Where |
| --- | --- |
| Look, tokens, components, window chrome, accent picker | Cutmark Design Language v1 Spec (added to docs/suite before Phase 3) |
| Visual reference | Design canvas "Cutmark Look", board "E · Audio editor, blue" |
| Audio editor architecture and layer rules | docs/audio-editor/spec.md |
| Workspace rules, crate map, CI, checks | docs/suite/S0-Workspace.md |
| Existing `ui-kit` | `crates/ui-kit` (see Starting point) |
| Audio editor starting point | Tag `s0` on `main`, version 1.1.0, 656 passing tests (Nov 14) |

## Starting point: ui-kit today

Read from `crates/ui-kit` on Nov 18. Used by the Recorder.

| File | Holds today | What S1 does with it |
| --- | --- | --- |
| `Cargo.toml` | egui, eframe, egui-phosphor from the workspace; strict lints with `unsafe_code = "forbid"` | Adds `windows` (Windows only) and `egui_kittest` (dev). `unsafe_code` goes from `forbid` to `deny`, with one allowed module (see Amendments) |
| `src/palette.rs` | 25 color constants copied from the audio editor's old theme (`PANEL_BG`, `ACCENT`, `METER_GREEN` and so on), plus `CONTROL_ROUNDING` | Becomes a compatibility layer: every old name stays, now pointing at the nearest new token, marked `#[deprecated]` with the new name in the note. Deleted once no app uses it |
| `src/fonts.rs` | IBM Plex Sans Regular, Medium, SemiBold; JetBrains Mono Regular, Medium; phosphor icons as fallback in every family; `mono()`, `medium()`, `semibold()` helpers; `MEDIUM_FAMILY`, `SEMIBOLD_FAMILY` | Plex is replaced by Inter (same three weights), JetBrains Mono gains SemiBold. Function and constant names stay, so callers do not change. Plex files and license are removed |
| `src/lib.rs` | `apply(ctx)`: installs fonts, `Visuals` and spacing from the palette | Same signature. Rebuilt on tokens and the runtime `Theme`. Every app calling it gets the new look |
| `src/meter.rs` | `level_meter(ui, peak_dbfs, clipped, width)`: horizontal bar, -60 to 0 dBFS, green to -12, yellow to -3, red above, clip light | Same signature and thresholds (they already match the Design Language spec). Restyled to the Meter component (6px bar, 3px radius, `sem.*` colors). A vertical variant is added |

The thresholds already in `meter.rs` become the tokens `sem.meter.ok` (below -12 dBFS), `sem.meter.warn` (-12 to -3) and `sem.meter.clip` (above -3).

## Amendments

### To S0-Workspace

1. **Crate map, `ui-kit` row:** "Created at: S1" becomes "Created at: Recorder phase 1, extended in S1". "Holds" becomes "Design Language tokens, theme, fonts, icons, window chrome, all shared widgets (meters and faders included), dialog frame, accent picker".
2. **Dependency rule 5:** `ui-kit` joins `capture`, `media-io` and `prompter-platform` as a crate allowed to hold Windows-only code, behind `cfg(windows)`, for the window chrome only.
3. **Unsafe code:** the Win32 hit test needs `unsafe`. In `ui-kit`'s `Cargo.toml`, `unsafe_code` changes from `forbid` to `deny` (a crate-level `forbid` cannot be lifted for one module). Only `src/chrome/windows.rs` carries `#[allow(unsafe_code)]`, with a safety comment on every block.
4. **CI:** if the Linux job does not already cover `ui-kit`, it adds `fmt`, `clippy` and `test` for it (non-Windows code). About one extra minute per run. Approving this spec approves that change.

### To the Cutmark Design Language v1 Spec

1. **Icons:** the suite's icon set is Phosphor (already in `ui-kit` through `egui-phosphor`, MIT). Line icons use the Regular weight; Play, Stop, Record and frame step use Fill. This replaces "line icons on a 16 or 18px grid, 1.6px stroke, drawn in `ui-kit`".

### To the Video Editor UI v1 Spec

- Open question "Fonts": answered. Inter and JetBrains Mono, no IBM Plex.
- Open question "Window frame": answered. Custom frame, once `ui-kit` has it.

## Working alongside the Recorder track

S1 runs in the git worktree `cm_suite\cutmark-s1` on branch `s1`; the Recorder runs in `cm_suite\cutmark` on `main`. Both tracks change `ui-kit`. To keep them from colliding:

1. One owner at a time. An S1 phase that touches `ui-kit` does not start while a Recorder phase that touches `ui-kit` is open, and the other way round.
2. Every S1 phase that touches `ui-kit` also builds the Recorder and runs its tests (`scripts\check.ps1` already covers every package).
3. New widgets the Recorder needs during S1 go into `ui-kit` following this spec's tokens, not into the Recorder.
4. The Recorder's look changes when Phase 6 lands. That is expected; the Recorder's own screens move to the new components on its own track.

## Technical decisions

| Topic | Decision |
| --- | --- |
| egui and eframe | The workspace version `ui-kit` already uses. Phase 1 confirms the audio editor is on it; Phase 2 moves it there if not. |
| Renderer | eframe `wgpu` feature in the audio editor, `glow` removed. `ui-kit` stays renderer-neutral, as today. |
| Custom painting | Glow paint callbacks found in Phase 1 move to egui `Shape`s or a wgpu `PaintCallback`. |
| Fonts | Inter Regular, Medium, SemiBold and JetBrains Mono Regular, Medium, SemiBold, static TTFs in `crates/ui-kit/assets/fonts/` with their OFL files, embedded with `include_bytes!` as today. |
| Icons | Phosphor through `egui-phosphor`, Regular and Fill variants. |
| Tokens | `design/tokens.json` at the repo root. `ui-kit/build.rs` generates `src/tokens.rs` and writes `design/tokens.css`. |
| Runtime theme | `ui_kit::Theme` (accent tokens, glass switch) stored in the egui context. Widgets read accent colors from `Theme::get(ctx)`, never from a constant. `palette::ACCENT` stays as the default Blue for old callers. |
| Color math | OKLCH conversion and the accent clamp written by hand in `ui-kit` (about 80 lines), tested against reference values. No color crate. |
| Appearance file | `%APPDATA%\Cutmark\appearance.json`, polled once a second on a timer. No file-watcher crate. |
| Window chrome | eframe viewport without decorations; Win32 subclass for `WM_NCHITTEST` (drag area, caption buttons, `HTMAXBUTTON` for Snap Layouts, resize edges) and `DwmSetWindowAttribute` for round corners, through the `windows` crate, in `src/chrome/windows.rs`. Exposed as `ui_kit::chrome::main_toolbar(...)` plus a viewport builder helper. |
| Chrome fallback | If Phase 7 cannot get dragging, resizing and Snap Layouts working reliably within the phase, the window keeps the native title bar and the 52px toolbar sits under it. Recorded in this spec at the end of Phase 7. |
| Style lint | `crates/ui-kit/tests/no_hardcoded_style.rs` scans the `src` folders of apps on an opt-in list for hex colors, `Color32::from_rgb`, literal corner radii and font sizes, and for any use of `ui_kit::palette`. The audio editor joins in Phase 13. The Recorder joins on its own track. |
| Tests | Unit tests for tokens, color math, clamp and appearance file. Widget behavior tests with `egui_kittest` where they run headless. Visual approval is JD looking at the gallery and the app. |

## Audio editor mapping

The restyle maps what the app has today onto Design Language components. Phase 1's inventory confirms the list; anything it finds that is not here gets a row before Phase 10.

| Today (`ui/`) | After S1 |
| --- | --- |
| Native title bar, `menu.rs` | 52px main toolbar with custom chrome. Left: app mark, project name with save state and format caption, Undo, Redo. Right: menu button (opens the existing menus as a glass popover), Export as the primary button, caption buttons. Center: empty in S1 |
| `toolbar.rs` transport buttons | Glass transport deck over the timeline: timecode, back to start, Play (accent capsule), Stop, Record, Loop if present, total length |
| `toolbar.rs` edit tools | Timeline panel toolbar: tools as a segmented control, toggles as chips, zoom slider and Fit if present |
| `timeline/ruler.rs` | Ruler token styles, markers as 7px dots |
| `timeline/track_header.rs` | Track header component: number, name, existing toggles as 24px rounded squares, record arm in `sem.record` |
| `timeline/lane.rs`, `clip_view.rs`, `waveform.rs` | Track rows and clips per the Clip component, waveform in `text.tertiary`, selected clip in accent tint |
| Playhead (wherever drawn) | Playhead component |
| `bin_panel.rs` | Left panel, list rows and search field if present |
| `master_strip.rs`, `widgets/meter.rs`, `widgets/fader.rs` | Right panel: Master inspector group with `ui-kit`'s fader and vertical meter. The app's own meter and fader widgets are deleted |
| The app's own theme module | Deleted. `ui_kit::apply` replaces it |
| `status_bar.rs` | Status bar component |
| `start_screen.rs` | Panel-style start screen in the same window chrome: recent projects as list rows, New and Open as buttons |
| `dialogs/*` | `ui-kit` dialog frame; buttons, fields, sliders and switches from `ui-kit` |
| New: View > Appearance | Dialog holding the `ui-kit` accent picker and the glass switch. The only new UI in S1, required by the Design Language spec |

Layer rule 5 of the audio editor still holds: only `ui` and `app` import `egui` and `ui-kit`.

## Build phases

Fourteen phases, in order. Phase 0 is JD by hand. Each later phase gets one or more Cline prompts, written one phase at a time (one concern, one layer, size S or M, its own Cline task). After every phase: run `scripts\check.ps1`, look at the running app (and the Recorder, when `ui-kit` changed), read the diff, commit, then start the next. Never stack phases.

### Phase 0: Preflight (JD, no Cline)

1. Worktree `cutmark-s1` on branch `s1`, created from a clean `main` at or after `s0`, with the Recorder's latest phase committed.
2. Run `scripts\check.ps1` and write down the passing test counts for the audio editor, the Recorder and `ui-kit`.
3. Screenshots of the current audio editor (start screen, main window with a project open, each dialog) and of the Recorder's main window.

**Done when:** the counts and screenshots are saved in `docs/audio-editor/s1-before/`.

### Phase 1: Inventory (Cline, read-only)

Cline writes `docs/audio-editor/ui-inventory.md`: the egui and eframe versions in the audio editor and the workspace; every use of glow; every file in `ui/` with the controls and colors it draws; the app's own theme module; every hard-coded color, size and font; every keyboard shortcut; and every `ui_kit` item the Recorder uses today.

**Done when:** the file exists and no code changed.

### Phase 2: wgpu move

Align the audio editor with the workspace egui and eframe if needed, switch eframe to `wgpu`, remove `glow`, port any glow drawing.

**Done when:** the app looks and behaves as in the "before" screenshots, all tests pass, playback and scrubbing stay smooth with a 4-track, 10-minute project.

### Phase 3: Tokens and palette compatibility (G1)

`design/tokens.json` with every fixed value from the Design Language spec, `ui-kit/build.rs`, generated `src/tokens.rs`, written `design/tokens.css`. `palette.rs` turns into deprecated aliases onto the new tokens.

**Done when:** a test checks that every token in the spec's tables exists, the Recorder builds (deprecation warnings allowed in the Recorder only), and all counts from Phase 0 hold.

### Phase 4: Runtime theme, accent and appearance (G1)

`Theme` in the egui context, OKLCH conversion, derivation of the six accent tokens, the readability clamp, the four presets, `appearance.json` read, write and poll.

**Done when:** tests pass for the presets, edge picks (black, white, mid gray, pure red, neon green), the record-hue warning, and a missing or corrupt appearance file (falls back to Blue).

### Phase 5: Style lint (G1)

`no_hardcoded_style.rs` with an empty opt-in list, plus a self-test that catches a planted hex color and a planted `ui_kit::palette` use.

**Done when:** the test passes and `scripts\check.ps1` runs it.

### Phase 6: Fonts, apply and core widgets (G2)

Inter in, Plex out, JetBrains Mono SemiBold added, same font API. `apply` rebuilt on tokens and `Theme`. Core widgets: panel, main toolbar layout (without chrome), icon button, primary button, segmented control, chip, switch, slider, text field, search field, list row, inspector group and row, focus ring, states.

**Done when:** widget behavior tests pass where `egui_kittest` allows, the Recorder runs with the new look and no broken screens, clippy is clean.

### Phase 7: Window chrome, Windows (G2)

Lint change to `deny`, `src/chrome/windows.rs`, undecorated window, caption buttons, drag area, double-click to maximize, Snap Layouts on maximize hover, resize edges, DWM round corners, inactive window state.

**Done when:** on Windows 11 the window drags, snaps (with the Snap Layouts flyout), resizes from every edge, maximizes and restores, and works at 100%, 150% and 200% scaling across two monitors with different scaling. Otherwise the fallback is recorded and the phase closes on it.

### Phase 8: Media widgets (G3)

Glass painter, transport deck, `level_meter` restyle plus a vertical meter, fader, track header, clip, waveform styling helper, playhead, marker, pending cut band, menus and popovers, toasts, dialog frame, accent picker.

**Done when:** behavior tests pass where `egui_kittest` allows, the accent picker's clamp note appears for a dark pick, and the Recorder's meters still read correctly.

### Phase 9: Gallery (G4)

`apps/gallery`: one scrolling window in the real chrome, every component in every state, accent picker and glass switch at the top, accent changes applied live.

**Done when:** JD approves the gallery on screen. **This is a gate.** Requested changes go into `ui-kit` and are re-reviewed before Phase 10.

### Phase 10: Restyle, frame (G5)

Audio editor onto `ui_kit::apply` (its own theme module deleted), window chrome, main toolbar, status bar, start screen, menus as a popover.

### Phase 11: Restyle, timeline (G5)

Panel toolbar, ruler, track headers, lanes, clips, waveforms, playhead, markers, transport deck.

### Phase 12: Restyle, side panels (G5)

Bin panel, master strip with `ui-kit`'s fader and vertical meter. The app's own `widgets/meter.rs` and `widgets/fader.rs` are deleted.

### Phase 13: Restyle, dialogs and lint (G5)

Every dialog in the `ui-kit` frame, View > Appearance, then the audio editor joins the style lint's opt-in list.

**Done when (Phases 10 to 13 together):** every row of the mapping table is in place, every "before" screenshot has an "after" match with nothing missing, all shortcuts still work, the lint passes, and the test count is at least the Phase 0 count.

### Phase 14: Close-out

Version 1.2.0, installer rebuilt with the Inter and JetBrains Mono license files, `ui-inventory.md` updated, memory bank updated, one week of JD's own use with issues fixed as they come up, one manual CI run, tag `s1`.

## Acceptance

1. Every feature in the Phase 0 screenshots still works.
2. The audio editor matches the "E · Audio editor, blue" board in layout and look, minus elements the app does not have.
3. Changing the accent in View > Appearance recolors the audio editor, the gallery and the Recorder within a second, with all three open.
4. No hard-coded style values in the audio editor (lint passes).
5. Window chrome works per Phase 7, or the fallback is recorded.
6. The Recorder builds and passes its tests at every phase.
7. Test counts at least the Phase 0 counts; `scripts\check.ps1` green; one CI run green.

## Risks

| Risk | Mitigation |
| --- | --- |
| egui version gap in the audio editor | Phase 1 measures it. More than one minor version: Phase 2 splits into one prompt per minor version |
| Glow-specific waveform drawing is slow as shapes | wgpu `PaintCallback` for waveforms if shapes drop below 60 fps on the Phase 2 test project |
| Win32 chrome edge cases (multi-monitor DPI, snap) | Timeboxed Phase 7 with a recorded fallback |
| Restyle loses a control | Mapping table, inventory, before and after screenshots |
| S1 and the Recorder track edit `ui-kit` at the same time | One owner at a time, Recorder built and tested in every `ui-kit` phase |
| Font swap shifts the Recorder's layouts (Inter is slightly wider than Plex) | Phase 6 checks the Recorder's screens; overflow fixes go into the Recorder's track |

## Open questions

1. **App mark.** The placeholder circle with a notch is used until a real logo exists.
