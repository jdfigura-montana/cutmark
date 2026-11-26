//! The script editor: a monospace text field that keeps its selection while a menu or the
//! find bar has the keyboard, and the Edit menu's commands for it.

use std::ops::Range;

use egui::text::{CCursor, CCursorRange, CharIndex, LayoutJob, TextFormat};
use egui::text_edit::TextEditState;
use ui_kit::tokens::{color, typography};

use crate::clipboard::{self, Clipboard};
use crate::find;
use crate::menu::Item;

/// How strongly the accent color backs a find match, and the current one.
const MATCH_TINT: f32 = 0.3;
const CURRENT_TINT: f32 = 0.7;

/// The editor's id for one opening of a script. Its cursor and undo history are kept under it.
pub fn id(serial: u64) -> egui::Id {
    egui::Id::new(("script-editor", serial))
}

/// Puts what the clipboard holds in place of the text of each paste the window reported, if
/// the editor has the keyboard: its HTML as Markdown, or with Ctrl+Shift held its plain text.
/// With nothing to paste, the event is dropped. Call it before [`Editor::show`] and before
/// [`command`], whose pastes are ready as they are. `held` is the modifiers as of the end of
/// the last frame.
pub fn rich_paste(ui: &egui::Ui, id: egui::Id, clipboard: &dyn Clipboard, mut held: egui::Modifiers) {
    if !ui.memory(|memory| memory.has_focus(id)) {
        return;
    }
    ui.input_mut(|input| {
        input.events.retain_mut(|event| match event {
            // The modifiers may change again in the same frame, so they are followed in order.
            egui::Event::ModifiersChanged(modifiers) => {
                held = *modifiers;
                true
            }
            egui::Event::Paste(text) => match clipboard::paste_from(clipboard, held.command && held.shift) {
                Some(pasted) => {
                    *text = pasted;
                    true
                }
                None => false,
            },
            _ => true,
        });
    });
}

/// Gives the editor the keyboard and has it do what an Edit menu item asks, as if the keys had
/// been pressed. Call it before [`Editor::show`], and not in the frame of the click that asked:
/// the editor drops the keyboard on a click elsewhere.
pub fn command(ui: &egui::Ui, id: egui::Id, item: Item, clipboard: &dyn Clipboard) {
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers { ctrl: true, command: true, ..Default::default() },
    };
    let event = match item {
        Item::Undo => Some(key(egui::Key::Z)),
        Item::Redo => Some(key(egui::Key::Y)),
        Item::Cut => Some(egui::Event::Cut),
        Item::Copy => Some(egui::Event::Copy),
        Item::Paste | Item::PastePlain => clipboard::paste_from(clipboard, item == Item::PastePlain).map(egui::Event::Paste),
        Item::SelectAll => Some(key(egui::Key::A)),
        _ => return,
    };
    ui.memory_mut(|memory| memory.request_focus(id));
    if let Some(event) = event {
        ui.input_mut(|input| input.events.push(event));
    }
}

/// What the Edit menu's enable rules need to know about the editor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Status {
    pub has_text: bool,
    pub has_selection: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// The find matches to paint.
#[derive(Debug, Clone, Copy)]
pub struct Highlight<'a> {
    pub query: &'a str,
    /// The index of the current match.
    pub current: usize,
    /// Select the current match and scroll to it.
    pub reveal: bool,
}

/// What [`Editor::show`] did.
pub struct Shown {
    pub changed: bool,
    pub response: egui::Response,
}

/// What is remembered about the open script's editor between frames.
#[derive(Default)]
pub struct Editor {
    serial: u64,
    /// The text has changed since the script was opened.
    edited: bool,
    /// The selection the undo flags were last read at.
    seen: Option<CCursorRange>,
    status: Status,
    /// The 1-based line of the text the cursor is on.
    cursor_line: u32,
}

impl Editor {
    pub fn status(&self) -> Status {
        self.status
    }

    /// The cursor's 1-based line as of the last [`Editor::show`].
    pub fn cursor_line(&self) -> u32 {
        self.cursor_line
    }

    /// The editor, filling the space left.
    pub fn show(&mut self, ui: &mut egui::Ui, serial: u64, text: &mut String, highlight: Option<Highlight<'_>>) -> Shown {
        if self.serial != serial {
            *self = Self { serial, ..Default::default() };
        }
        let id = id(serial);
        let shown = egui::ScrollArea::vertical().id_salt(id).auto_shrink(false).show(ui, |ui| field(ui, id, text, highlight)).inner;

        self.edited |= shown.changed;
        let state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        let range = state.cursor.char_range().unwrap_or_default();
        self.cursor_line = line_of(text, range.primary.index);
        self.status.has_text = !text.is_empty();
        self.status.has_selection = !range.is_empty();
        // Reading the undo history copies it, so it is read only when it can have changed.
        if shown.changed || self.seen != Some(range) {
            self.seen = Some(range);
            let undoer = state.undoer();
            let now = (range, text.clone());
            // Before the first change, the history holds only where the cursor was.
            self.status.can_undo = self.edited && undoer.has_undo(&now);
            self.status.can_redo = undoer.has_redo(&now);
        }
        shown
    }
}

/// The 1-based line of the character at `index`.
fn line_of(text: &str, index: CharIndex) -> u32 {
    let breaks = text.chars().take(index.0).filter(|&c| c == '\n').count();
    u32::try_from(breaks).map_or(u32::MAX, |breaks| breaks.saturating_add(1))
}

/// The text field inside its scroll area.
fn field(ui: &mut egui::Ui, id: egui::Id, text: &mut String, highlight: Option<Highlight<'_>>) -> Shown {
    // egui drops the selection of a field without the keyboard, and a right-click moves the
    // cursor. The selection is put back below, so that menus and the find bar can act on it.
    let mut kept = TextEditState::load(ui.ctx(), id).and_then(|state| state.cursor.char_range());
    let revealed = highlight.filter(|highlight| highlight.reveal).and_then(|highlight| {
        let found = find::matches(text, highlight.query).get(highlight.current)?.clone();
        Some(char_range(text, found))
    });
    if let Some(range) = revealed {
        select(ui.ctx(), id, range);
        kept = Some(range);
    }

    let font = ui_kit::fonts::mono(typography::BODY_SIZE);
    let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
    // Enough empty rows that a click anywhere in the panel lands in the field.
    let rows = (ui.available_height() / line).floor().max(1.0) as usize;
    let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
    let mut plain = TextFormat::simple(font.clone(), color::TEXT_PRIMARY);
    plain.line_height = Some(line + ui.spacing().extra_text_line_spacing);
    // egui's own layout for a multiline field, with the matches backed.
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let text = buffer.as_str();
        let mut job = LayoutJob { keep_trailing_whitespace: true, ..Default::default() };
        job.wrap.max_width = wrap_width;
        let mut end = 0;
        if let Some(highlight) = highlight {
            for (index, found) in find::matches(text, highlight.query).into_iter().enumerate() {
                let tint = if index == highlight.current { CURRENT_TINT } else { MATCH_TINT };
                job.append(&text[end..found.start], 0.0, plain.clone());
                job.append(&text[found.clone()], 0.0, TextFormat { background: accent.gamma_multiply(tint), ..plain.clone() });
                end = found.end;
            }
        }
        job.append(&text[end..], 0.0, plain.clone());
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };

    // The undo history takes points when the text has rested, so a paste gets one on each side
    // to be exactly one step.
    let pasting = ui.memory(|memory| memory.has_focus(id)) && ui.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::Paste(_))));
    if pasting {
        add_undo_point(ui.ctx(), id, text);
    }
    let right_button = ui.input(|i| i.pointer.button_down(egui::PointerButton::Secondary) || i.pointer.button_released(egui::PointerButton::Secondary));
    let output = egui::TextEdit::multiline(text)
        .id(id)
        .font(font)
        .text_color(color::TEXT_PRIMARY)
        .frame(egui::Frame::NONE)
        .desired_width(f32::INFINITY)
        .desired_rows(rows)
        .layouter(&mut layouter)
        .show(ui);
    let response = output.response.response;
    if pasting {
        add_undo_point(ui.ctx(), id, text);
    }

    if let Some(range) = kept {
        // A right-click inside the selection keeps it. Anywhere else, it moves the cursor.
        let on_selection = right_button
            && !range.is_empty()
            && response.contains_pointer()
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|pointer| {
                let at = output.galley.cursor_from_pos(pointer - output.galley_pos).index;
                let [first, last] = range.sorted_cursors();
                (first.index..=last.index).contains(&at)
            });
        if on_selection || !ui.memory(|memory| memory.has_focus(id)) {
            select(ui.ctx(), id, range);
        }
    }
    if let Some(range) = revealed {
        let [first, _] = range.sorted_cursors();
        let at = output.galley.pos_from_cursor(first).translate(output.galley_pos.to_vec2());
        ui.scroll_to_rect(at, Some(egui::Align::Center));
    }
    Shown { changed: response.changed(), response }
}

fn select(ctx: &egui::Context, id: egui::Id, range: CCursorRange) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

/// Makes the text and cursor as they are now a step that Undo can return to.
fn add_undo_point(ctx: &egui::Context, id: egui::Id, text: &str) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    // A field that has had no cursor yet has it at the end.
    let end = || CCursorRange::one(CCursor::new(CharIndex(text.chars().count())));
    let range = state.cursor.char_range().unwrap_or_else(end);
    let mut undoer = state.undoer();
    undoer.add_undo(&(range, text.to_owned()));
    state.set_undoer(undoer);
    state.store(ctx, id);
}

/// A byte range of `text` as the editor counts it, in characters.
fn char_range(text: &str, bytes: Range<usize>) -> CCursorRange {
    let cursor = |end: usize| CCursor::new(CharIndex(text[..end].chars().count()));
    CCursorRange::two(cursor(bytes.start), cursor(bytes.end))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clipboard with HTML and the text that goes with it.
    struct Copied;

    impl Clipboard for Copied {
        fn read_html(&self) -> Option<String> {
            Some("<b>back</b>".to_owned())
        }

        fn read_text(&self) -> Option<String> {
            Some("back".to_owned())
        }

        fn write_text(&self, _text: &str) -> prompter_platform::Result<()> {
            Ok(())
        }
    }

    /// One frame of the editor, with the command given first as the Library does.
    fn frame(ctx: &egui::Context, time: f64, text: &mut String, item: Option<Item>, events: Vec<egui::Event>) -> egui::FullOutput {
        let input = egui::RawInput { time: Some(time), events, ..Default::default() };
        let mut editor = Editor::default();
        let mut output = ctx.run_ui(input, |ui| {
            if let Some(item) = item {
                command(ui, id(1), item, &Copied);
            }
            editor.show(ui, 1, text, None);
        });
        output.textures_delta.clear();
        output
    }

    #[test]
    fn undo_and_redo_from_the_menu_reach_the_editor() {
        let ctx = egui::Context::default();
        ctx.set_fonts(ui_kit::fonts::definitions());
        let mut text = "one".to_owned();
        // The undo history takes a point once the text has rested for a second.
        ctx.memory_mut(|memory| memory.request_focus(id(1)));
        frame(&ctx, 0.0, &mut text, None, Vec::new());
        frame(&ctx, 2.0, &mut text, None, Vec::new());
        frame(&ctx, 3.0, &mut text, None, vec![egui::Event::Text(" two".to_owned())]);
        frame(&ctx, 5.0, &mut text, None, Vec::new());
        assert_eq!(text, "one two");
        // A menu click takes the keyboard away from the editor.
        ctx.memory_mut(|memory| memory.surrender_focus(id(1)));
        frame(&ctx, 6.0, &mut text, Some(Item::Undo), Vec::new());
        assert_eq!(text, "one");
        frame(&ctx, 7.0, &mut text, Some(Item::Redo), Vec::new());
        assert_eq!(text, "one two");
    }

    #[test]
    fn copy_cut_and_paste_reach_the_editor_after_frames_without_the_keyboard() {
        let ctx = egui::Context::default();
        ctx.set_fonts(ui_kit::fonts::definitions());
        let mut text = "words".to_owned();
        ctx.memory_mut(|memory| memory.request_focus(id(1)));
        frame(&ctx, 0.0, &mut text, None, Vec::new());
        frame(&ctx, 1.0, &mut text, Some(Item::SelectAll), Vec::new());
        let copied = |output: egui::FullOutput| {
            output.platform_output.commands.into_iter().find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text),
                _ => None,
            })
        };
        // An open menu has the keyboard for some frames. The selection has to last through them.
        ctx.memory_mut(|memory| memory.surrender_focus(id(1)));
        frame(&ctx, 1.5, &mut text, None, Vec::new());
        frame(&ctx, 1.6, &mut text, None, Vec::new());
        assert_eq!(copied(frame(&ctx, 2.0, &mut text, Some(Item::Copy), Vec::new())).as_deref(), Some("words"));
        assert_eq!(copied(frame(&ctx, 3.0, &mut text, Some(Item::Cut), Vec::new())).as_deref(), Some("words"));
        assert_eq!(text, "");
        frame(&ctx, 4.0, &mut text, Some(Item::Paste), Vec::new());
        assert_eq!(text, "**back**");
        frame(&ctx, 5.0, &mut text, Some(Item::PastePlain), Vec::new());
        assert_eq!(text, "**back**back");
    }
}
