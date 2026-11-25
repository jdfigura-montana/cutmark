//! Find in script: the matching, and the bar at the top of the editor panel.

use std::ops::Range;

use ui_kit::tokens::{color, size, space, typography};

use crate::editor::Highlight;

/// Room for the match count.
const COUNT_WIDTH: f32 = 72.0;
/// The least width of the bar's search field.
const MIN_FIELD_WIDTH: f32 = 60.0;

/// The byte ranges in `text` that match `query` without case, in order and without overlaps.
pub fn matches(text: &str, query: &str) -> Vec<Range<usize>> {
    let needle: String = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    // A letter's lowercase form can differ in length, so each byte of the lowercased text
    // remembers which character of `text` it came from.
    let mut lower = String::with_capacity(text.len());
    let mut origin: Vec<(usize, usize)> = Vec::with_capacity(text.len());
    for (at, original) in text.char_indices() {
        lower.extend(original.to_lowercase());
        origin.resize(lower.len(), (at, at + original.len_utf8()));
    }
    lower.match_indices(&needle).filter_map(|(start, found)| Some(origin.get(start)?.0..origin.get(start + found.len() - 1)?.1)).collect()
}

/// Gives a text field the keyboard with all of `text` selected.
pub fn focus_and_select(ui: &egui::Ui, id: egui::Id, text: &str) {
    ui.memory_mut(|memory| memory.request_focus(id));
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    let all = egui::text::CCursorRange::two(egui::text::CCursor::default(), egui::text::CCursor::new(egui::text::CharIndex(text.chars().count())));
    state.cursor.set_char_range(Some(all));
    state.store(ui.ctx(), id);
}

/// The find bar's state. The query is kept while the bar is closed.
#[derive(Default)]
pub struct FindBar {
    open: bool,
    query: String,
    /// The index of the current match.
    current: usize,
    /// The search field has to take the keyboard.
    focus: bool,
    /// The editor has to select the current match and scroll to it.
    reveal: bool,
    /// The search field, once drawn.
    field: Option<egui::Id>,
    /// The field or the editor had the keyboard at the end of the last frame. Esc takes it
    /// away before the frame starts, so it cannot be asked during one.
    had_keyboard: bool,
}

impl FindBar {
    pub fn is_open(&self) -> bool {
        self.open
    }

    #[cfg(test)]
    pub fn field(&self) -> Option<egui::Id> {
        self.field
    }

    /// Opens the bar, or goes back to it, with the query selected.
    pub fn open(&mut self) {
        self.open = true;
        self.focus = true;
        self.reveal = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.had_keyboard = false;
    }

    /// What the editor should paint, or `None` with the bar closed.
    pub fn highlight(&mut self) -> Option<Highlight<'_>> {
        let reveal = std::mem::take(&mut self.reveal);
        self.open.then(|| Highlight { query: &self.query, current: self.current, reveal })
    }

    /// Notes who has the keyboard. Call it after the editor is drawn.
    pub fn note_keyboard(&mut self, ui: &egui::Ui, editor: egui::Id) {
        let field = self.field;
        self.had_keyboard = ui.memory(|memory| memory.has_focus(editor) || field.is_some_and(|field| memory.has_focus(field)));
    }

    /// Draws the bar's contents in a row and handles its keys. Esc and Close give the
    /// keyboard back to `editor`.
    pub fn show(&mut self, ui: &mut egui::Ui, text: &str, editor: egui::Id) {
        let count = matches(text, &self.query).len();
        self.current = self.current.min(count.saturating_sub(1));
        let (mut step, mut close) = self.keys(ui);

        ui.spacing_mut().item_spacing.x = space::SCALE_2;
        let reserved = COUNT_WIDTH + 3.0 * size::ICON_BUTTON + 4.0 * space::SCALE_2;
        let field_size = egui::vec2((ui.available_width() - reserved).max(MIN_FIELD_WIDTH), size::ICON_BUTTON);
        let field = ui
            .allocate_ui_with_layout(field_size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(field_size.x);
                ui_kit::search_field(ui, &mut self.query, "Find")
            })
            .inner;
        self.field = Some(field.id);
        if std::mem::take(&mut self.focus) {
            focus_and_select(ui, field.id, &self.query);
        }
        if field.changed() {
            self.current = 0;
            self.reveal = true;
        }

        // The query may have just changed.
        let count = matches(text, &self.query).len();
        let label = match count {
            _ if self.query.is_empty() => String::new(),
            0 => "No matches".to_owned(),
            _ => format!("{} of {count}", self.current + 1),
        };
        let label = egui::RichText::new(label).font(ui_kit::fonts::mono(typography::VALUE_SIZE)).color(color::TEXT_SECONDARY);
        ui.add_sized(egui::vec2(COUNT_WIDTH, size::ICON_BUTTON), egui::Label::new(label).selectable(false).truncate());
        ui.add_enabled_ui(count > 0, |ui| {
            if ui_kit::icon_button(ui, ui_kit::icons::regular::CARET_UP, "Previous (Shift+F3)").clicked() {
                step = Some(false);
            }
            if ui_kit::icon_button(ui, ui_kit::icons::regular::CARET_DOWN, "Next (F3)").clicked() {
                step = Some(true);
            }
        });
        close |= ui_kit::icon_button(ui, ui_kit::icons::regular::X, "Close (Esc)").clicked();

        if let Some(forward) = step
            && count > 0
        {
            self.current = (self.current + if forward { 1 } else { count - 1 }) % count;
            self.reveal = true;
            // So that Enter goes on from here after a button was clicked.
            ui.memory_mut(|memory| memory.request_focus(field.id));
        }
        if close {
            self.close();
            ui.memory_mut(|memory| memory.request_focus(editor));
            ui.ctx().request_repaint();
        }
    }

    /// The step the keys ask for (true is forward), and whether Esc closes the bar. The keys
    /// are consumed, so that Enter does not take the keyboard from the field.
    fn keys(&self, ui: &egui::Ui) -> (Option<bool>, bool) {
        let in_field = self.field.is_some_and(|field| ui.memory(|memory| memory.has_focus(field)));
        // An open popup takes Esc for itself.
        let escapes = self.had_keyboard && !egui::Popup::is_any_open(ui.ctx());
        ui.input_mut(|input| {
            let mut pressed = |modifiers, key| input.consume_key(modifiers, key);
            // A pattern without Shift also matches with it, so the Shift keys go first.
            let back = pressed(egui::Modifiers::SHIFT, egui::Key::F3) || (in_field && pressed(egui::Modifiers::SHIFT, egui::Key::Enter));
            let forward = pressed(egui::Modifiers::NONE, egui::Key::F3) || (in_field && pressed(egui::Modifiers::NONE, egui::Key::Enter));
            let step = if back { Some(false) } else { forward.then_some(true) };
            (step, escapes && pressed(egui::Modifiers::NONE, egui::Key::Escape))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found<'a>(text: &'a str, query: &str) -> Vec<&'a str> {
        matches(text, query).into_iter().map(|range| &text[range]).collect()
    }

    #[test]
    fn matching_ignores_case_and_keeps_the_original_ranges() {
        assert_eq!(found("Take one, take TWO", "take"), ["Take", "take"]);
        assert_eq!(found("aaaa", "AA"), ["aa", "aa"]);
        // The dotted capital I lowercases to two characters, which moves what follows.
        assert_eq!(found("\u{130} Caf\u{c9} caf\u{e9}", "caf\u{e9}"), ["Caf\u{c9}", "caf\u{e9}"]);
        assert!(matches("words", "").is_empty());
        assert!(matches("", "words").is_empty());
    }
}
