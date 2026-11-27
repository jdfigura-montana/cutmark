//! The keys the editor handles itself, ahead of its text field: those that have to go by
//! what can be seen, and those of the "/" menu. The rest are left to the field.

use std::ops::Range;

use egui::text::{CCursor, CCursorRange, CharIndex};
use egui::text_edit::TextEditState;
use egui::{Event, Key, Modifiers};

use crate::caret::Doc;
use crate::copy;
use crate::editor::add_undo_point;
use crate::formatting::Change;
use crate::slash::{self, Menu};
use crate::styled::ParseCache;
use crate::typing::{self, Carry, Typed};

/// What is remembered between keys.
#[derive(Debug, Default)]
pub struct Keys {
    /// Formatting that what is typed next goes back inside.
    carry: Option<Carry>,
    /// The open "/" menu.
    pub menu: Option<Menu>,
}

/// What [`Keys::take`] did.
#[derive(Debug, Clone, Copy, Default)]
pub struct Taken {
    /// The text changed.
    pub changed: bool,
    /// A key was handled.
    pub used: bool,
}

/// The editor's text and where its caret is, as the keys change them.
struct Field<'a> {
    text: &'a mut String,
    /// In bytes, from the anchor to the caret, which may come first.
    anchor: usize,
    caret: usize,
}

impl<'a> Field<'a> {
    /// With the caret at the end, until one is loaded.
    fn new(text: &'a mut String) -> Self {
        let end = text.len();
        Self { text, anchor: end, caret: end }
    }

    /// Takes the field's cursor, if it has had one.
    fn load(&mut self, ctx: &egui::Context, id: egui::Id) {
        if let Some(range) = TextEditState::load(ctx, id).and_then(|state| state.cursor.char_range()) {
            let byte = |at: CharIndex| self.text.char_indices().nth(at.0).map_or(self.text.len(), |(at, _)| at);
            (self.anchor, self.caret) = (byte(range.secondary.index), byte(range.primary.index));
        }
    }

    fn store(&self, ctx: &egui::Context, id: egui::Id) {
        let mut state = TextEditState::load(ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(self.cursor()));
        state.store(ctx, id);
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn apply(&mut self, change: Change) -> bool {
        let changed = *self.text != change.text;
        (*self.text, self.anchor, self.caret) = (change.text, change.selection.start, change.selection.end);
        changed
    }

    fn cursor(&self) -> CCursorRange {
        let cursor = |at: usize| CCursor::new(CharIndex(self.text[..at].chars().count()));
        CCursorRange::two(cursor(self.anchor), cursor(self.caret))
    }
}

/// True for an event that changes the text or moves the cursor if the editor leaves it to
/// the field. What comes after it has to be the field's too.
fn changes_if_left(event: &Event) -> bool {
    match event {
        Event::Key { key, pressed: true, .. } => matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::Backspace | Key::Delete),
        Event::Paste(_) | Event::Cut => true,
        _ => false,
    }
}

/// True for an event that the text field moves the cursor or changes the text for. What
/// comes after it has to be the field's too, to be done in order.
fn is_the_fields(event: &Event) -> bool {
    match event {
        Event::Key { key, pressed: true, modifiers, .. } => match key {
            Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End | Key::PageUp | Key::PageDown | Key::Tab => true,
            Key::A | Key::Z | Key::Y => modifiers.command,
            _ => false,
        },
        Event::PointerButton { button: egui::PointerButton::Primary, pressed: true, .. } | Event::Ime(_) => true,
        _ => false,
    }
}

impl Keys {
    /// Does and removes this frame's events that are the editor's, in order, up to the first
    /// that is the text field's. With `raw`, syntax shows, and the field edits it as the
    /// plain text it looks like, but for Enter and the menu. Call it with the editor
    /// holding the keyboard, before its field is shown.
    pub fn take(&mut self, ui: &egui::Ui, id: egui::Id, text: &mut String, cache: &mut ParseCache, raw: bool) -> Taken {
        let mut taken = Taken::default();
        let mut field = Field::new(text);
        field.load(ui.ctx(), id);
        let mut events = ui.input_mut(|input| std::mem::take(&mut input.events));
        let mut stopped = false;
        events.retain(|event| {
            stopped |= !self.in_menu(event) && is_the_fields(event);
            if stopped {
                return true;
            }
            // A copy moves nothing, so it does not count as a key that scrolls to the caret.
            if matches!(event, Event::Copy) && !raw && !field.selection().is_empty() {
                let markup = cache.get(field.text);
                copy::put(ui.ctx(), &Doc::new(field.text, &markup), field.selection());
                return false;
            }
            let Some(changed) = self.one(ui, id, &mut field, event, cache, raw) else {
                stopped = changes_if_left(event);
                return true;
            };
            taken.used = true;
            taken.changed |= changed;
            self.check_menu(field.text, &field.selection());
            false
        });
        ui.input_mut(|input| input.events = events);
        if taken.used {
            field.store(ui.ctx(), id);
        }
        taken
    }

    /// True for a key the open menu takes.
    fn in_menu(&self, event: &Event) -> bool {
        let Event::Key { key, pressed: true, modifiers, .. } = event else {
            return false;
        };
        self.menu.is_some() && *modifiers == Modifiers::NONE && matches!(key, Key::ArrowUp | Key::ArrowDown | Key::Enter | Key::Escape)
    }

    /// Closes the menu if what has been typed is no longer a command.
    pub fn check_menu(&mut self, text: &str, selection: &Range<usize>) {
        if let Some(menu) = &mut self.menu {
            match menu.query(text, selection) {
                Some(query) => menu.selected = menu.selected.min(slash::matching(query).len().saturating_sub(1)),
                None => self.menu = None,
            }
        }
    }

    /// Does one event. `None` if it is not the editor's, else whether the text changed.
    fn one(&mut self, ui: &egui::Ui, id: egui::Id, field: &mut Field<'_>, event: &Event, cache: &mut ParseCache, raw: bool) -> Option<bool> {
        if self.in_menu(event)
            && let Event::Key { key, .. } = event
        {
            return Some(self.menu_key(ui.ctx(), id, *key, field, cache));
        }
        let markup = cache.get(field.text);
        let selection = field.selection();
        let doc = || Doc::new(field.text, &markup);
        let done: Option<Typed> = match event {
            Event::Text(typed) if !typed.is_empty() && typed != "\n" && typed != "\r" => {
                if raw {
                    let mut new = field.text.clone();
                    new.replace_range(selection.clone(), typed);
                    let caret = selection.start + typed.len();
                    Some(Typed { change: Change { text: new, selection: caret..caret }, carry: None })
                } else {
                    Some(typing::type_text(&doc(), selection, typed, self.carry.as_ref()))
                }
            }
            Event::Paste(pasted) if !raw && !pasted.is_empty() => Some(typing::insert(&doc(), selection, pasted, self.carry.as_ref())),
            Event::Cut if !raw && !selection.is_empty() => {
                copy::put(ui.ctx(), &doc(), selection.clone());
                Some(typing::replace(&doc(), selection, ""))
            }
            Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.ctrl => {
                // With Shift, only a line break.
                Some(if modifiers.shift && !raw { typing::insert(&doc(), selection, "\n", None) } else { typing::enter(&doc(), selection) })
            }
            Event::Key { key, pressed: true, modifiers, .. } if !raw => {
                let word = modifiers.ctrl || modifiers.alt;
                match key {
                    Key::ArrowLeft | Key::ArrowRight => {
                        let forward = *key == Key::ArrowRight;
                        let doc = doc();
                        let step = |at| match (forward, word) {
                            (false, false) => doc.left(at),
                            (false, true) => doc.word_left(at),
                            (true, false) => doc.right(at),
                            (true, true) => doc.word_right(at),
                        };
                        if modifiers.shift {
                            field.caret = step(field.caret);
                        } else {
                            // Without Shift, a selection closes to the side the arrow points to.
                            let to = match () {
                                () if selection.is_empty() || word => step(field.caret),
                                () if forward => selection.end,
                                () => selection.start,
                            };
                            (field.anchor, field.caret) = (to, to);
                        }
                        (self.carry, self.menu) = (None, None);
                        return Some(false);
                    }
                    // Shift+Delete is the field's cut.
                    Key::Backspace => {
                        let done = typing::backspace(&doc(), selection, word, self.carry.as_ref());
                        return Some(self.finish(field, done));
                    }
                    Key::Delete if !modifiers.shift => {
                        let done = typing::delete(&doc(), selection, word);
                        return Some(self.finish(field, done));
                    }
                    // The field's other delete keys, which would take hidden syntax.
                    Key::H if modifiers.ctrl => {
                        let done = typing::backspace(&doc(), selection, false, self.carry.as_ref());
                        return Some(self.finish(field, done));
                    }
                    Key::K if modifiers.ctrl => {
                        let doc = doc();
                        let end = doc.bounds[doc.line_of(selection.start)].1;
                        let to = if selection.is_empty() { selection.start..end.max(selection.start) } else { selection };
                        let done = (!to.is_empty()).then(|| typing::replace(&doc, to, ""));
                        return Some(self.finish(field, done));
                    }
                    _ => return None,
                }
            }
            _ => return None,
        };
        let changed = self.finish(field, done);
        if let Event::Text(typed) = event
            && typed == "/"
            && let Some(at) = field.caret.checked_sub(1)
            && field.text.as_bytes().get(at) == Some(&b'/')
        {
            let markup = cache.get(field.text);
            if slash::opens(&Doc::new(field.text, &markup), at) {
                self.menu = Some(Menu::new(at));
            }
        }
        Some(changed)
    }

    fn finish(&mut self, field: &mut Field<'_>, done: Option<Typed>) -> bool {
        match done {
            Some(done) => {
                self.carry = done.carry;
                field.apply(done.change)
            }
            None => false,
        }
    }

    fn menu_key(&mut self, ctx: &egui::Context, id: egui::Id, key: Key, field: &mut Field<'_>, cache: &mut ParseCache) -> bool {
        let selection = field.selection();
        let Some(menu) = &mut self.menu else {
            return false;
        };
        match key {
            Key::ArrowUp | Key::ArrowDown => {
                let count = menu.query(field.text, &selection).map_or(0, |query| slash::matching(query).len());
                menu.step(key == Key::ArrowDown, count);
            }
            Key::Enter => {
                field.store(ctx, id);
                let changed = self.pick(ctx, id, field.text, cache);
                // The command has stored where the caret is now.
                field.load(ctx, id);
                return changed;
            }
            _ => self.menu = None,
        }
        false
    }

    /// Does the menu's highlighted command, as one step of the undo history, and closes the
    /// menu. True if the text changed. The caret is the one the field has stored.
    pub fn pick(&mut self, ctx: &egui::Context, id: egui::Id, text: &mut String, cache: &mut ParseCache) -> bool {
        let Some(menu) = self.menu.take() else {
            return false;
        };
        let mut field = Field::new(text);
        field.load(ctx, id);
        let selection = field.selection();
        let listed = menu.query(field.text, &selection).map(slash::matching).unwrap_or_default();
        let Some(&(_, action)) = listed.get(menu.selected) else {
            return false;
        };
        let markup = cache.get(field.text);
        let change = slash::run(&Doc::new(field.text, &markup), menu.at..selection.start, action);
        add_undo_point(ctx, id, field.text);
        let changed = field.apply(change);
        field.store(ctx, id);
        add_undo_point(ctx, id, field.text);
        self.carry = None;
        changed
    }
}
