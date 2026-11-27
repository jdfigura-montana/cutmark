//! The script editor: a styled Markdown text field that keeps its selection while a menu or
//! the find bar has the keyboard, and the Edit and Format menus' commands for it.

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use egui::text::{CCursor, CCursorRange, CharIndex, LayoutJob, TextFormat};
use egui::text_edit::TextEditState;
use ui_kit::tokens::{color, space};

use crate::caret::{Bias, Doc};
use crate::clipboard::{self, Clipboard};
use crate::find;
use crate::formatting::{self, Action};
use crate::keys::Keys;
use crate::markup::Markup;
use crate::menu::Item;
use crate::slash;
use crate::styled::{self, ParseCache};

/// How strongly the accent color backs a find match, and the current one.
const MATCH_TINT: f32 = 0.3;
const CURRENT_TINT: f32 = 0.7;

/// The widest the column of text gets.
pub const COLUMN_WIDTH: f32 = 760.0;
/// The room above the first line, and below the last.
pub const TOP_PAD: f32 = 48.0;
const BOTTOM_PAD: f32 = space::SCALE_8;
/// The caret, in the accent color. It is what shows that typing goes to the editor.
pub const CARET_WIDTH: f32 = 1.5;
/// What an empty script shows.
pub const PLACEHOLDER: &str = "Enter text or type '/' for commands";
/// How long the caret stays solid after a key the editor handled itself. The field only
/// restarts the blink for the keys it handles.
const CARET_HOLD: f64 = 0.5;

/// The editor's id for one opening of a script. Its cursor and undo history are kept under it.
pub fn id(serial: u64) -> egui::Id {
    egui::Id::new(("script-editor", serial))
}

/// Watches for a Ctrl+V the window sends no paste for. With no text on the clipboard, such as
/// with HTML alone, the window sends nothing at all for the press of V, only its release.
#[derive(Debug, Default)]
pub struct PasteKeys {
    /// The modifiers as of the last time Ctrl was held, since V was last released.
    chord: Option<egui::Modifiers>,
    /// The press of V has arrived, so it was no paste.
    typed: bool,
    /// A paste has arrived since Ctrl went down.
    pasted: bool,
}

impl PasteKeys {
    /// Takes in one event, with the modifiers held before it. For the release of a V whose
    /// press sent nothing, returns the modifiers the paste was asked with. Ctrl may have been
    /// let go first.
    fn missed(&mut self, event: &egui::Event, held: egui::Modifiers) -> Option<egui::Modifiers> {
        match event {
            egui::Event::ModifiersChanged(modifiers) if modifiers.command => {
                if !held.command {
                    self.pasted = false;
                }
                self.chord = Some(*modifiers);
            }
            egui::Event::Paste(_) => self.pasted = true,
            egui::Event::Key { key: egui::Key::V, pressed: true, .. } => self.typed = true,
            egui::Event::Key { key: egui::Key::V, pressed: false, .. } => {
                let chord = if held.command { Some(held) } else { self.chord.take() };
                let typed = std::mem::take(&mut self.typed);
                let pasted = std::mem::take(&mut self.pasted);
                return chord.filter(|_| !typed && !pasted);
            }
            // What happens in another window is not heard.
            egui::Event::WindowFocused(false) => *self = Self::default(),
            _ => {}
        }
        None
    }
}

/// Puts what the clipboard holds in place of the text of each paste the window reported, if
/// the editor has the keyboard: its HTML as Markdown, or with Ctrl+Shift held its plain text.
/// With nothing to paste, the event is dropped. A Ctrl+V the window reported no paste for is
/// pasted as the Edit menu would. Call it every frame, with `id` `None` with no script open,
/// before [`Editor::show`] and before [`command`], whose pastes are ready as they are. `held`
/// is the modifiers as of the end of the last frame.
pub fn rich_paste(ui: &egui::Ui, id: Option<egui::Id>, clipboard: &dyn Clipboard, keys: &mut PasteKeys, mut held: egui::Modifiers) {
    let focused = id.filter(|&id| ui.memory(|memory| memory.has_focus(id)));
    let mut missed = None;
    ui.input_mut(|input| {
        input.events.retain_mut(|event| {
            missed = keys.missed(event, held).or(missed);
            match event {
                // The modifiers may change again in the same frame, so they are followed in order.
                egui::Event::ModifiersChanged(modifiers) => {
                    held = *modifiers;
                    true
                }
                egui::Event::Paste(text) if focused.is_some() => match clipboard::paste_from(clipboard, held.command && held.shift) {
                    Some(pasted) => {
                        *text = pasted;
                        true
                    }
                    None => false,
                },
                _ => true,
            }
        });
    });
    if let (Some(id), Some(chord)) = (focused, missed) {
        command(ui, id, if chord.shift { Item::PastePlain } else { Item::Paste }, clipboard);
    }
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
    /// The formatting at the caret.
    pub format: formatting::Active,
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

/// What is remembered about the editors between frames. The status and cursor are the shown
/// one's. egui keeps each one's cursor, selection, scroll and undo history under its ids.
#[derive(Default)]
pub struct Editor {
    serial: u64,
    /// The openings that have been shown, whose state egui holds.
    shown: HashSet<u64>,
    /// The openings whose text has changed since they were opened.
    edited: HashSet<u64>,
    /// The selection the undo flags were last read at.
    seen: Option<CCursorRange>,
    status: Status,
    /// The 1-based line of the text the cursor is on.
    cursor_line: u32,
    look: Look,
    keys: Keys,
    /// The selection as of the end of the last frame. One that has changed since may be
    /// somewhere the caret must not rest.
    placed: Option<CCursorRange>,
    /// The parse of the text as of the end of the last frame, to tell a change made since.
    left: Option<Rc<Markup>>,
    /// When the editor last handled a key itself.
    used_at: Option<f64>,
}

/// What the styled layout needs besides the text.
#[derive(Default)]
struct Look {
    cache: ParseCache,
    /// The View menu's Show Markdown.
    show_markdown: bool,
}

impl Editor {
    pub fn status(&self) -> Status {
        self.status
    }

    /// The cursor's 1-based line as of the last [`Editor::show`].
    pub fn cursor_line(&self) -> u32 {
        self.cursor_line
    }

    /// Drops the cursor and undo history of each opening that is no longer open.
    pub fn forget_closed(&mut self, ctx: &egui::Context, open: &[u64]) {
        self.edited.retain(|serial| open.contains(serial));
        self.shown.retain(|&serial| {
            let keep = open.contains(&serial);
            if !keep {
                ctx.data_mut(|data| data.remove::<TextEditState>(id(serial)));
            }
            keep
        });
    }

    /// True while the "/" menu is open, which Esc closes.
    pub fn menu_open(&self) -> bool {
        self.keys.menu.is_some()
    }

    /// Shows the Markdown syntax, dimmed, or hides all of it.
    pub fn show_markdown(&mut self, show: bool) {
        // With its syntax hidden again, the caret may be somewhere it must not rest.
        if self.look.show_markdown && !show {
            self.placed = None;
        }
        self.look.show_markdown = show;
    }

    /// Does a formatting action to the selection or the caret's line, as one step of the
    /// undo history, and gives the editor the keyboard. True if the text changed. Call it
    /// before [`Editor::show`], and not in the frame of a click that asked for it.
    pub fn format(&mut self, ctx: &egui::Context, serial: u64, text: &mut String, action: Action) -> bool {
        let id = id(serial);
        ctx.memory_mut(|memory| memory.request_focus(id));
        let end = || CCursorRange::one(CCursor::new(CharIndex(text.chars().count())));
        let range = TextEditState::load(ctx, id).and_then(|state| state.cursor.char_range()).unwrap_or_else(end);
        let [first, last] = range.sorted_cursors();
        let selection = byte_of(text, first.index)..byte_of(text, last.index);
        let markup = self.look.cache.get(text);
        let Some(change) = formatting::apply(text, &markup, selection, action) else {
            return false;
        };
        // A point on each side, so that the change is exactly one step.
        add_undo_point(ctx, id, text);
        *text = change.text;
        let selected = char_range(text, change.selection);
        select(ctx, id, selected);
        add_undo_point(ctx, id, text);
        (self.keys, self.placed) = (Keys::default(), Some(selected));
        self.edited.insert(serial);
        self.left = None;
        // The status is read again, though the caret may not have moved.
        self.seen = None;
        true
    }

    /// The editor, filling the space left: a column of text in the middle of it. The first
    /// time an opening is shown, the caret goes to the top and the editor takes the keyboard:
    /// with its syntax hidden and no caret, a script looks like a page that cannot be typed
    /// in.
    pub fn show(&mut self, ui: &mut egui::Ui, serial: u64, text: &mut String, highlight: Option<Highlight<'_>>) -> Shown {
        if self.serial != serial {
            (self.serial, self.seen, self.status) = (serial, None, Status::default());
            (self.keys, self.placed, self.left, self.used_at) = (Keys::default(), None, None, None);
        }
        let id = id(serial);
        if self.shown.insert(serial) {
            select(ui.ctx(), id, CCursorRange::one(CCursor::new(CharIndex(0))));
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        // With the caret in a list, Tab is the editor's. Elsewhere it moves the keyboard on.
        let in_list = self.status.format.list.is_some();
        let area = egui::UiBuilder::new().max_rect(ui.available_rect_before_wrap());
        let shown = ui
            .scope_builder(area, |ui| {
                let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
                ui.visuals_mut().text_cursor.stroke = egui::Stroke::new(CARET_WIDTH, accent);
                let scroll = egui::ScrollArea::vertical().id_salt(id).auto_shrink(false);
                scroll
                    .show(ui, |ui| {
                        let column = column(ui.available_rect_before_wrap());
                        ui.scope_builder(egui::UiBuilder::new().max_rect(column), |ui| self.field(ui, id, text, highlight, in_list)).inner
                    })
                    .inner
            })
            .inner;

        if shown.changed {
            self.edited.insert(serial);
        }
        let state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        let range = state.cursor.char_range().unwrap_or_default();
        self.cursor_line = line_of(text, range.primary.index);
        self.status.has_text = !text.is_empty();
        self.status.has_selection = !range.is_empty();
        // Reading the undo history copies it, so it is read only when it can have changed, and
        // so is the formatting at the caret, which reads the whole text.
        if shown.changed || self.seen != Some(range) {
            self.seen = Some(range);
            let [first, last] = range.sorted_cursors();
            let selection = byte_of(text, first.index)..byte_of(text, last.index);
            let caret = byte_of(text, range.primary.index);
            self.status.format = formatting::active(text, &self.look.cache.get(text), selection, caret);
            let undoer = state.undoer();
            let now = (range, text.clone());
            // Before the first change, the history holds only where the cursor was.
            self.status.can_undo = self.edited.contains(&serial) && undoer.has_undo(&now);
            self.status.can_redo = undoer.has_redo(&now);
        }
        shown
    }

    /// The text field inside its scroll area.
    fn field(&mut self, ui: &mut egui::Ui, id: egui::Id, text: &mut String, highlight: Option<Highlight<'_>>, in_list: bool) -> Shown {
        let revealed = highlight.filter(|highlight| highlight.reveal).and_then(|highlight| {
            let found = find::matches(text, highlight.query).get(highlight.current)?.clone();
            Some(char_range(text, found))
        });
        if let Some(range) = revealed {
            select(ui.ctx(), id, range);
        }

        // The undo history takes points when the text has rested, so a paste or cut gets one on
        // each side to be exactly one step.
        // Text put there from outside, as a restored snapshot is, may leave the caret in
        // syntax that is now hidden.
        if self.left.take().is_some_and(|left| !Rc::ptr_eq(&left, &self.look.cache.get(text))) {
            self.placed = None;
        }
        let focused = ui.memory(|memory| memory.has_focus(id));
        let pasting = focused && ui.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::Paste(_) | egui::Event::Cut)));
        if pasting {
            add_undo_point(ui.ctx(), id, text);
        }
        let now = ui.input(|i| i.time);
        let taken = if focused { self.keys.take(ui, id, text, &mut self.look.cache, self.look.show_markdown) } else { Default::default() };
        if taken.used {
            self.used_at = Some(now);
        }
        if let Some(left) = self.used_at.map(|used| used + CARET_HOLD - now).filter(|&left| left > 0.0) {
            ui.visuals_mut().text_cursor.blink = false;
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left));
        }
        // egui drops the selection of a field without the keyboard, and a right-click moves the
        // cursor. The selection is put back below, so that menus and the find bar can act on it.
        let kept = TextEditState::load(ui.ctx(), id).and_then(|state| state.cursor.char_range());

        let font = styled::body_font();
        let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
        // Enough empty rows that a click anywhere down the column lands in the field.
        let rows = ((ui.available_height() - TOP_PAD - BOTTOM_PAD) / line).floor().max(1.0) as usize;
        let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
        let show_markdown = self.look.show_markdown;
        let cache = &mut self.look.cache;
        let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
            let text = buffer.as_str();
            let found = highlight.map_or_else(Vec::new, |highlight| find::matches(text, highlight.query));
            let tinted: Vec<_> = found
                .into_iter()
                .enumerate()
                .map(|(index, found)| {
                    let current = highlight.is_some_and(|highlight| index == highlight.current);
                    (found, accent.gamma_multiply(if current { CURRENT_TINT } else { MATCH_TINT }))
                })
                .collect();
            let markup = cache.get(text);
            let job = styled::layout_job(ui, text, &markup, show_markdown, &tinted, wrap_width);
            styled::layout(ui, &markup, job)
        };

        let right_button = ui.input(|i| i.pointer.button_down(egui::PointerButton::Secondary) || i.pointer.button_released(egui::PointerButton::Secondary));
        // A place under the text, for what is only known once the text is laid out.
        let behind = ui.painter().add(egui::Shape::Noop);
        let gutter = styled::GUTTER as i8;
        let margin = egui::Margin { left: gutter, right: gutter, top: TOP_PAD as i8, bottom: BOTTOM_PAD as i8 };
        let output = egui::TextEdit::multiline(text)
            .id(id)
            .lock_focus(in_list)
            .font(font)
            .text_color(color::TEXT_PRIMARY)
            // The margin goes in the frame: egui ignores TextEdit::margin once a frame is given.
            .frame(egui::Frame::NONE.inner_margin(margin))
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .layouter(&mut layouter)
            .show(ui);
        let response = output.response.response;
        if pasting {
            add_undo_point(ui.ctx(), id, text);
        }
        let markup = self.look.cache.get(text);
        let decorations = styled::decorations(ui, &markup, &output.galley, output.galley_pos.to_vec2(), show_markdown);
        ui.painter().set(behind, egui::Shape::Vec(decorations.behind));
        ui.painter().extend(decorations.front);
        if text.is_empty() {
            placeholder(ui, &output.galley, output.galley_pos);
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
        // The field moves the cursor with no care for what is hidden. A new place for it is
        // moved to one it may rest at.
        let field_changed = response.changed();
        if !show_markdown && !field_changed && !taken.changed && revealed.is_none() {
            self.snap(ui, id, text, &output.galley, output.galley_pos, response.interact_pointer_pos());
        }
        if let Some(range) = revealed {
            let [first, _] = range.sorted_cursors();
            let at = output.galley.pos_from_cursor(first).translate(output.galley_pos.to_vec2());
            ui.scroll_to_rect(at, Some(egui::Align::Center));
        }

        let picked = self.slash_menu(ui, id, text, &output.galley, output.galley_pos);
        self.placed = TextEditState::load(ui.ctx(), id).and_then(|state| state.cursor.char_range());
        self.left = Some(self.look.cache.get(text));
        if taken.used
            && let Some(range) = self.placed
        {
            let at = output.galley.pos_from_cursor(range.primary).translate(output.galley_pos.to_vec2());
            ui.scroll_to_rect(at, None);
        }
        Shown { changed: field_changed || taken.changed || picked, response }
    }

    /// Moves a selection that has changed to places the caret may rest. The places on either
    /// side of hidden syntax look the same: a click takes the one on the side of the
    /// character it was nearer. `pointer` is where the pointer is, if it is clicking or
    /// dragging in the text.
    fn snap(&mut self, ui: &egui::Ui, id: egui::Id, text: &str, galley: &egui::Galley, at: egui::Pos2, pointer: Option<egui::Pos2>) {
        let Some(mut state) = TextEditState::load(ui.ctx(), id) else {
            return;
        };
        let Some(range) = state.cursor.char_range() else {
            return;
        };
        let same = |a: CCursor, b: CCursor| a.index == b.index;
        if self.placed.is_some_and(|placed| same(placed.primary, range.primary) && same(placed.secondary, range.secondary)) {
            return;
        }
        let [first, last] = range.sorted_cursors();
        // Select all takes the marks at the very start too, so that all of the script can go.
        if !range.is_empty() && first.index.0 == 0 && last.index.0 == text.chars().count() {
            return;
        }
        let anchor_stayed = self.placed.is_some_and(|placed| same(placed.secondary, range.secondary));
        // A double or triple click picks both ends at once, and they draw in to its words.
        let picked = pointer.is_some() && !range.is_empty() && !anchor_stayed;
        let markup = self.look.cache.get(text);
        let doc = Doc::new(text, &markup);
        let index = |byte: usize| CharIndex(text[..byte].chars().count());
        let place = |cursor: CCursor, pointer: Option<egui::Pos2>| {
            let bias = if picked && cursor.index == first.index { Bias::Forward } else { Bias::Back };
            let rest = doc.rest(byte_of(text, cursor.index), bias);
            let (before, after) = doc.cluster(rest);
            let byte = match pointer {
                Some(pointer) if before < after => {
                    let edge = galley.pos_from_cursor(CCursor::new(index(before))).left() + at.x;
                    if pointer.x <= edge + 0.5 { before } else { after }
                }
                _ => rest,
            };
            CCursor { index: index(byte), ..cursor }
        };
        let primary = place(range.primary, pointer.filter(|_| !picked));
        let secondary = match () {
            () if range.is_empty() => primary,
            () if anchor_stayed => range.secondary,
            () => place(range.secondary, None),
        };
        if !same(primary, range.primary) || !same(secondary, range.secondary) {
            state.cursor.set_char_range(Some(CCursorRange { primary, secondary, h_pos: range.h_pos }));
            state.store(ui.ctx(), id);
            ui.ctx().request_repaint();
        }
    }

    /// Draws the "/" menu at the caret and does a command clicked in it. True if that
    /// changed the text.
    fn slash_menu(&mut self, ui: &egui::Ui, id: egui::Id, text: &mut String, galley: &egui::Galley, at: egui::Pos2) -> bool {
        let Some(range) = TextEditState::load(ui.ctx(), id).and_then(|state| state.cursor.char_range()) else {
            return false;
        };
        let [first, last] = range.sorted_cursors();
        let selection = byte_of(text, first.index)..byte_of(text, last.index);
        self.keys.check_menu(text, &selection);
        let Some(menu) = &mut self.keys.menu else {
            return false;
        };
        let listed = menu.query(text, &selection).map(slash::matching).unwrap_or_default();
        let caret = galley.pos_from_cursor(range.primary).translate(at.to_vec2());
        let shown = slash::show(ui, id, menu, &listed, caret);
        if let Some(index) = shown.clicked {
            menu.selected = index;
            ui.memory_mut(|memory| memory.request_focus(id));
            ui.ctx().request_repaint();
            return self.keys.pick(ui.ctx(), id, text, &mut self.look.cache);
        }
        let focused = ui.memory(|memory| memory.has_focus(id));
        let pressed = ui.input(|i| i.pointer.any_pressed());
        if !shown.pointed && (pressed || !focused) {
            self.keys.menu = None;
        } else if focused {
            // Esc closes the menu, and must not take the keyboard from the editor.
            let filter = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, tab: self.status.format.list.is_some(), escape: true };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(id, filter));
        }
        false
    }
}

/// The 1-based line of the character at `index`.
fn line_of(text: &str, index: CharIndex) -> u32 {
    let breaks = text.chars().take(index.0).filter(|&c| c == '\n').count();
    u32::try_from(breaks).map_or(u32::MAX, |breaks| breaks.saturating_add(1))
}

/// The byte at which the character at `index` starts.
fn byte_of(text: &str, index: CharIndex) -> usize {
    text.char_indices().nth(index.0).map_or(text.len(), |(at, _)| at)
}

/// Where the field goes in `area`: the text's column in the middle, with room at its left for
/// quote bars and as much at its right.
fn column(area: egui::Rect) -> egui::Rect {
    let width = area.width().min(COLUMN_WIDTH + 2.0 * styled::GUTTER);
    let left = area.left() + ((area.width() - width) / 2.0).floor();
    egui::Rect::from_min_size(egui::pos2(left, area.top()), egui::vec2(width, area.height()))
}

/// The hint in an empty script, where its first line would be.
fn placeholder(ui: &egui::Ui, galley: &egui::Galley, at: egui::Pos2) {
    let format = TextFormat { italics: true, ..TextFormat::simple(styled::body_font(), color::TEXT_TERTIARY) };
    let mut job = LayoutJob::single_section(PLACEHOLDER.to_owned(), format);
    job.wrap.max_width = galley.job.wrap.max_width;
    let hint = ui.fonts_mut(|fonts| fonts.layout_job(job));
    ui.painter().galley(at, hint, color::TEXT_TERTIARY);
}

fn select(ctx: &egui::Context, id: egui::Id, range: CCursorRange) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

/// Makes the text and cursor as they are now a step that Undo can return to.
pub(crate) fn add_undo_point(ctx: &egui::Context, id: egui::Id, text: &str) {
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
        // A new editor each frame, for an opening already shown: a first showing moves the caret.
        let mut editor = Editor { shown: HashSet::from([1]), ..Editor::default() };
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
