# Recorder Phase 9e: Composite mode (spec)

Nov 24, 2026 · @JD · draft 1

Exported from the Claude Project "Creator Suite" ("Recorder Phase 9e Composite Mode Spec"). The Project copy is the source.

## Overview

Today every take is Baked: the camera, its key, crop, border and the background are burned into the main video. Composite mode adds a second choice, Clean: the main video holds the screen only, the camera is recorded raw in its ISO file, and `session.json` gets a `composite` block that tells an editor how to rebuild the look. The user can then move, resize, re-key or drop the camera in the edit.

**Goals**

1. Baked stays the default and records exactly as today.
2. Clean gives an untouched screen file plus a raw camera file that line up within one frame (the existing cam offset and sync test still apply).
3. The `composite` block is complete enough for the CutMark video editor, and later the Final Cut export, to rebuild the Baked look without guessing.

**Non-goals**

- No editor work. The video editor reads the block in its own milestone (S5); the Final Cut export maps it in S2X.
- No change to keying, layouts or encoders.

## Behavior

| Layout | Baked (default) | Clean |
| --- | --- | --- |
| Screen only | Screen | Same as Baked |
| Picture in picture | Screen with the keyed, cropped, bordered camera | Screen only; camera in its ISO file |
| Cam only | Keyed camera over the background | Same as Baked |

Rules:

1. Composite mode is a profile setting (`composite_mode`: `baked` or `clean`), saved with the profile, switchable only while idle like every profile field.
2. Clean is offered only for Picture in picture. For the other layouts the control is disabled with the caption "Only for Picture in picture".
3. Clean needs the camera file: while Clean is on and the layout is Picture in picture, the Camera ISO switch is forced on and disabled, with the caption "Clean mode records the camera to its own file". Turning Clean off restores the switch to its previous state.
4. If the webcam is off or lost at start, a Clean take records the screen only and the `composite` block lists no camera layer. A camera lost mid-take behaves as today (last frame holds in the ISO, System mark).
5. The preview always shows the full composite, so the user sees the intended look. A "Clean" chip in the preview toolbar shows when the take will be Clean.
6. When the Capture Device Amendment lands, Clean also leaves the overlay source out of the main video; its layer goes in the same block. The block below is shaped for that now.

## The `composite` block in session.json

Written in every take, Baked or Clean, so an editor always knows what is in the main video.

```json
"composite": {
  "version": 1,
  "mode": "clean",
  "canvas": { "width": 2560, "height": 1440, "fps": 60 },
  "main": { "source": "screen", "file": "{name}.mkv" },
  "layers": [
    {
      "source": "cam",
      "file": "{name}_cam.mkv",
      "baked": false,
      "offset_ms": -50,
      "rect": { "x": 0.857, "y": 0.775, "w": 0.120, "h": 0.213 },
      "anchor": "bottom_right",
      "crop": { "left": 0.22, "right": 0.06, "top": 0.0, "bottom": 0.0 },
      "shape": "circle",
      "corner_radius_px": 0,
      "border": { "width_px": 5, "color": "#3D4BFF" },
      "key": { "enabled": false, "color": "#00B140", "similarity": 0.40, "smoothness": 0.08, "spill": 0.10,
               "garbage_crop": { "left": 0.0, "right": 0.0, "top": 0.0, "bottom": 0.0 },
               "mask": null }
    }
  ],
  "background": { "kind": "none" }
}
```

Field rules:

1. `rect` is the layer's final placed box on the canvas, as fractions of the canvas (0 to 1), after scale, offsets and crop. Editors use it directly; `anchor` is kept for editors that want to re-anchor.
2. `crop` and `garbage_crop` are fractions of the source frame. `mask` is null or `{ "x", "y", "w", "h" }` in fractions.
3. `baked: true` means the layer is already in the main video (Baked mode); editors must not add it again.
4. `offset_ms` is the cam offset that was applied (Baked) or must be applied (Clean).
5. `background.kind` is `none`, `color` (`"color": "#RRGGBB"`), `image` or `video` (`"path"`: absolute path at record time, plus `"fit"`: cover, contain or stretch).
6. Values come from the profile at the moment recording starts. Changes during a take are not possible (profile fields are locked while recording).
7. Readers ignore unknown fields. `version` bumps only on a breaking change.

## UI (Phase 13 layout)

- **Output tool:** new Composite group above Camera ISO file: segmented Baked | Clean, with one caption line under it: Baked "Camera and key are burned into the video." Clean "Screen only in the video; the camera goes to its own file for editing."
- **Camera ISO file group:** forced on and disabled under rule 3, with its caption.
- **Preview toolbar:** "Clean" chip (read-only) while Clean applies.
- **Review, File group:** row "Composite: Baked" or "Composite: Clean".
- **Control API:** the status event gains `compositeMode`. No new commands.

## Steps (one prompt each)

| Step | Work | Done when |
| --- | --- | --- |
| C1 | Profile field with migration (missing means Baked), compositor leaves the camera out of the main frame in Clean PiP, ISO forced on, preview still shows the full composite | A Clean take's main video has no camera; the ISO file exists; Baked takes are unchanged |
| C2 | `composite` block written at start in session.json for every take, geometry as fractions, docs/suite/recorder.md gets the schema | Unit tests for the rect math at three anchors and two scales; a real take's block matches its profile |
| C3 | UI: Composite group, ISO lock with caption, preview chip, Review row, status event field | JD records one Baked and one Clean take and checks both files |

## Acceptance

1. A Baked take is byte-for-byte the same kind of output as before (same streams, same look).
2. A Clean take: main video shows the screen with no camera; the cam ISO exists; placing the ISO by `rect` with `offset_ms` in any editor lines up with the Baked look within one frame and a few pixels.
3. Every take's session.json has a valid `composite` block.
4. Clean cannot start without the camera file (switch locked on), and is unavailable outside Picture in picture.
