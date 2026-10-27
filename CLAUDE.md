# Audio Editor

Windows multitrack audio editor in Rust: eframe/egui 0.36, egui-phosphor 0.14.

## Workflow

- At the start of every task, read `memory-bank/activeContext.md` and `memory-bank/progress.md`. At the end, update both.
- No behavior changes unless the task says so.

## UI rules

- Visual target is `docs/ui-target.png`.
- Every color, size and font comes from `src/ui/theme.rs`. No hardcoded `Color32` or pixel values elsewhere in `src/ui`.
- Any right-aligned group must still read left to right on screen.

## egui 0.36 gotchas

- `horizontal()` and `with_layout()` rows using `Align::Center` stretch to the parent's full available height. Inside a `Modal`, always bound each row with an explicit `max_rect`.
- Panels use `exact_size`, not `exact_height`.
- The menu bar uses `MenuBar::new().ui(...)`.

## Source encoding

- All `.rs` files are UTF-8 without a BOM.
- Write any non-ASCII character in a string literal as a `\u{...}` escape.
- Draw dots and icons with the painter or egui-phosphor, never as Unicode text glyphs.

## Definition of done

A task is done only when all of these pass:

1. `cargo fmt`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`
4. `cargo build --release`

The user runs `target\release\audio_editor.exe`, so the release build must be fresh.

## Final report

End every task with:

- Files changed
- The root cause of any bug
- A hand-check list
