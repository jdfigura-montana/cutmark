# Cutmark Design Language: accent per app (amendment)

Nov 24, 2026 · @JD · approved

Replaces the "Accent picker" rules on storage and sharing in docs/suite/Design-Language.md. Where they conflict, this file wins.

## Rules

1. Each app has its own accent. Changing it in one app changes only that app.
2. Storage: `%APPDATA%\Cutmark\<app>\appearance.json` on Windows (`~/Library/Application Support/Cutmark/<app>/appearance.json` on macOS), holding the raw pick and the clamped result, as before.
3. Apps no longer watch each other's appearance file. The shared `%APPDATA%\Cutmark\appearance.json` is no longer read or written. It is left on disk and not migrated: every app starts on its own default.
4. Every app has the accent picker in its own Appearance (the same ui-kit widget, same layout, same readability clamp). "Reset to Blue" becomes "Reset to default" and resets to that app's default.
5. Glass effects are always on; there is no glass switch (decision Nov 24, 2026).
6. Exported media and published TuteTool courses never use the accent (unchanged).

## Defaults

| App | `<app>` folder | Default accent |
| --- | --- | --- |
| Audio editor | audio | Blue |
| Recorder | recorder | Teal |
| Video editor | video | Orange |
| Prompter | prompter | Violet |
| Gallery | gallery | Blue |
| TuteTool | tutetool | Blue (its own repo, applied when TuteTool adopts the tokens) |

The four presets stay Blue, Violet, Teal, Orange, then Custom.
