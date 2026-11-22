---
id: troubleshooting
title: Troubleshooting
section: Help
order: 810
keywords: no sound, device, recording, input, missing media, relink, autosave, recovery, save error, problem
context:
---

Problems appear in a **Recording**, **Media** or **Project** dialog. When several kinds arrive together the title is **Messages**. In the quotes below, the text in angle brackets stands for the value the app fills in.

## Input device

- **Input device <name> not found. Using the default input.** The saved device was missing when a track was armed, so the Windows default opened instead. Pick the device again in **Audio Settings**; a missing device shows as **<name> (not found)**.
- **Could not arm the track: <reason>** The track stays disarmed. Check the device and **Record channels**.
- **Input device lost.** The track was disarmed; arm it again. During a take the message is **Recording stopped: input device lost.** and the take is kept.
- **Could not reopen the input: <reason>. The track was disarmed.** Shows in **Audio Settings**; arm the track again.

## Recording

- **Arm a track first** Click **R** on a track and press Record again.
- **The input is not ready yet** Try Record again.
- **Recording too short.** The take was deleted; record a longer one.
- **The recording dropped <N> ms of audio.** The take was kept with a gap.
- **The recording failed: <reason>** No take was added.

## Media

- **<N> media files are missing. Their clips are shown in red and play silence. Right-click an item in the bin and choose Relink to fix it.**
- **The replacement is too short. The clips using this item need at least <timecode>.** Pick a longer file.
- A failed import shows **unsupported audio file: <reason>** or **could not decode audio: <reason>**. Files that are not WAV or MP3 are skipped and their names are listed.

## Projects

- When a session did not close properly, the start screen offers **Recover autosave**. **Recover autosave** opens the autosave; **Open last saved** sets it aside and opens **project.json**.
- **The autosave could not be recovered. This project file is invalid: <reason>. The last saved project was opened instead.**
- **Autosave failed: <error>** Save the project with <kbd>Ctrl</kbd>+<kbd>S</kbd>.
- A project that will not open shows **This project file is invalid: <reason>** on the start screen.
- A failed save opens **Save Error** with the reason; the project stays open, so fix the cause and save again.

> Tip: Press <kbd>F1</kbd> over the panel that is giving trouble to open its topic.

See [Recording a take](topic:recording-a-take), [Importing audio](topic:importing-audio) and [Projects](topic:projects).
