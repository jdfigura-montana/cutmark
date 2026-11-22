---
id: exporting
title: Exporting
section: Export
order: 600
keywords: export, wav, mp3, bit depth, bitrate, channels, mono, stereo, selection, overwrite, clip
context: dialog.export
---

Render the mix to a WAV or MP3 file.

## Export the project

1. Open the **Project** menu and click **Export...**, or press <kbd>Ctrl</kbd>+<kbd>E</kbd>.
2. Choose a **Scope**: **Whole project** or **Current selection**. Without a range selection only **Whole project** is available.
3. Turn on **Selected tracks only** to export just the tracks in the range selection.
4. Choose **Format**: **WAV** or **MP3**.
5. For WAV, choose a **Bit depth**: **16-bit**, **24-bit** or **32-bit float**. For MP3, choose a **Bitrate**: **128**, **192**, **256** or **320** kbps.
6. Choose **Channels**: **Stereo** or **Mono**.
7. Type a path in **File**, or click **Browse...** to pick one in the project's **exports** folder.
8. Click **Export**.

The export is the mix you hear: mute, solo, track volume and pan, clip gain and fades, and the master fader. WAV files export at the project rate. MP3 files export at 44.1 kHz for 44.1 and 88.2 kHz projects, and at 48 kHz for every other rate. Mono sums the left and right channels.

## Watch the export

The **Exporting** window shows **Checking levels...**, then **Writing file...**, with a progress bar and **Cancel**. Editing and playback are locked until the export finishes or you cancel it.

If the mix goes above 0 dBFS, the **Clipping** window says **The mix peaks at +<X.X> dBFS and will clip.** Click **Cancel** to stop and lower a level, or **Export Anyway** to continue.

If a path you typed already exists, **Overwrite?** asks **"<name>" already exists. Overwrite it?** Click **Overwrite** to replace the file.

When the export finishes, the status bar shows **Exported <name>** with a **Reveal** link for ten seconds.

> Tip: Select a range first, then choose **Current selection** to export only that part.

See [Master and levels](topic:master-and-levels).
