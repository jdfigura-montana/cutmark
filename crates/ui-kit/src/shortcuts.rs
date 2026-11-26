//! The shortcut table (spec: S1 Help "Shortcut sheet"): each app defines its
//! keys once, and the input handling, the menus and the generated "Keyboard
//! shortcuts" page all read that one table, so they can never drift apart.
//! Styling comes only from ui-kit tokens.

use egui::{Context, KeyboardShortcut, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::HAIRLINE;
use crate::Theme;
use crate::fonts;
use crate::tokens::{space, typography};

/// Horizontal gap around the "+" between two key caps.
const CAP_GAP: f32 = space::SCALE_2;
/// Horizontal padding inside a key cap, each side (the Markdown `<kbd>` cap).
const KEY_PAD_X: f32 = 6.0;
/// Vertical padding inside a key cap, each side.
const KEY_PAD_Y: f32 = 2.0;
/// Corner radius of a key cap (smaller than `radius.group`).
const KEY_RADIUS: f32 = 4.0;
/// The heading of the generated page.
const SHEET_TITLE: &str = "Keyboard shortcuts";
/// The hint in the sheet's filter field.
const FILTER_HINT: &str = "Filter shortcuts";
/// Gap between a group's heading and its rows.
const GROUP_GAP: f32 = space::SCALE_8;
/// Gap between two groups on the sheet.
const SHEET_GAP: f32 = space::SCALE_16;
/// The order the sheet lists groups in (spec "Shortcut sheet": File, Edit,
/// Playback, Timeline, Tracks, View, Help). A group outside this list is
/// listed after these, in table order.
const GROUP_ORDER: [&str; 7] = ["File", "Edit", "Playback", "Timeline", "Tracks", "View", "Help"];

/// One app shortcut: its id, the menu label, its group, its keys (the first is
/// shown, the rest are alternates) and the context it applies in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// The id input handling and menus look the shortcut up by.
    pub id: &'static str,
    /// The action name shown in menus and on the sheet.
    pub label: &'static str,
    /// The group the sheet lists the shortcut under.
    pub group: &'static str,
    /// The keys; the first is shown, the rest are alternates.
    pub keys: Vec<KeyboardShortcut>,
    /// The context the shortcut only applies in, for the clash check.
    pub context: &'static str,
}

/// An app's shortcuts, in table order.
#[derive(Debug, Clone, Default)]
pub struct ShortcutTable {
    /// The shortcuts, in the order the app declared them.
    entries: Vec<Shortcut>,
}

impl ShortcutTable {
    /// A table over `entries`, in the given order.
    pub fn new(entries: Vec<Shortcut>) -> Self {
        Self { entries }
    }

    /// The table's entries, in table order.
    pub fn entries(&self) -> &[Shortcut] {
        &self.entries
    }

    /// The entry with `id`, or None when the table has no such shortcut.
    pub fn get(&self, id: &str) -> Option<&Shortcut> {
        self.entries.iter().find(|shortcut| shortcut.id == id)
    }

    /// True when one of `id`'s keys was pressed this frame. The key is
    /// consumed, so the shortcut fires once and nothing behind it sees the
    /// press; an unknown id is always false.
    pub fn pressed(&self, ctx: &Context, id: &str) -> bool {
        let Some(shortcut) = self.get(id) else {
            return false;
        };
        shortcut.keys.iter().any(|key| ctx.input_mut(|input| input.consume_shortcut(key)))
    }

    /// The text a menu shows next to an item, in egui's shortcut formatting
    /// (the first key only). An unknown id or a shortcut without keys is empty.
    pub fn text(&self, ctx: &Context, id: &str) -> String {
        self.get(id).and_then(|shortcut| shortcut.keys.first()).map(|key| ctx.format_shortcut(key)).unwrap_or_default()
    }

    /// Every clash where two entries share a key in the same context, as one
    /// message each, in table order.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (index, first) in self.entries.iter().enumerate() {
            for second in self.entries.iter().skip(index + 1) {
                if first.context != second.context || !shares_a_key(first, second) {
                    continue;
                }
                problems.push(format!("{} and {} both use {} in context {}", first.label, second.label, key_text(first.keys.first()), first.context));
            }
        }
        problems
    }
}

/// True when two shortcuts hold a common key.
fn shares_a_key(first: &Shortcut, second: &Shortcut) -> bool {
    first.keys.iter().any(|key| second.keys.contains(key))
}

/// A shortcut's first key as text like `Ctrl+Z`, for messages.
fn key_text(key: Option<&KeyboardShortcut>) -> String {
    key.map(|key| key.format(&egui::ModifierNames::NAMES, false)).unwrap_or_default()
}

/// One item of a shortcut's key caps: a cap or the "+" between two caps.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CapItem {
    /// A key cap.
    Cap(String),
    /// The "+" between two caps.
    Plus,
}

/// The key caps of a shortcut: each modifier and the key as its own cap, with
/// a "+" between them, exactly as the Markdown `<kbd>` keys are drawn.
pub fn key_caps(ui: &mut Ui, shortcut: &KeyboardShortcut) {
    draw_caps(ui, &cap_items(ui.ctx(), shortcut));
}

/// The caps and "+" items of a shortcut, in order.
fn cap_items(ctx: &Context, shortcut: &KeyboardShortcut) -> Vec<CapItem> {
    let mut items = Vec::new();
    for (index, part) in ctx.format_shortcut(shortcut).split('+').enumerate() {
        if index > 0 {
            items.push(CapItem::Plus);
        }
        items.push(CapItem::Cap(part.to_owned()));
    }
    items
}

/// Draws key-cap items left to right in the current layout.
fn draw_caps(ui: &mut Ui, items: &[CapItem]) {
    ui.scope(|ui| {
        // The gaps are part of each item, so there is no layout spacing
        // between them.
        ui.spacing_mut().item_spacing.x = 0.0;
        for item in items {
            match item {
                CapItem::Cap(text) => {
                    cap(ui, text);
                }
                CapItem::Plus => {
                    plus(ui);
                }
            }
        }
    });
}

/// The width of key-cap items laid out left to right.
fn caps_width(ui: &Ui, items: &[CapItem]) -> f32 {
    let mut width = 0.0;
    for item in items {
        match item {
            CapItem::Cap(text) => {
                width += cap_galley(ui, text).size().x + KEY_PAD_X * 2.0;
            }
            CapItem::Plus => {
                width += plus_galley(ui).size().x + CAP_GAP * 2.0;
            }
        }
    }
    width
}

/// Lays out the text of one key cap in `type.value` `text.primary`.
fn cap_galley(ui: &Ui, text: &str) -> std::sync::Arc<egui::Galley> {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    ui.painter().layout_no_wrap(text.to_owned(), fonts::mono(typography::VALUE_SIZE), neutrals.text_primary)
}

/// Lays out the "+" between two key caps in `type.value` `text.tertiary`.
fn plus_galley(ui: &Ui) -> std::sync::Arc<egui::Galley> {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    ui.painter().layout_no_wrap("+".to_owned(), fonts::mono(typography::VALUE_SIZE), neutrals.text_tertiary)
}

/// Draws one key cap: a small rounded key with a hairline rim, the same recipe
/// as the Markdown `<kbd>` keys.
fn cap(ui: &mut Ui, text: &str) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let galley = cap_galley(ui, text);
    let size = galley.size() + Vec2::new(KEY_PAD_X * 2.0, KEY_PAD_Y * 2.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, KEY_RADIUS, neutrals.bg_control);
        painter.rect_stroke(rect, KEY_RADIUS, Stroke::new(HAIRLINE, neutrals.stroke_hairline), StrokeKind::Inside);
        painter.galley(rect.center() - galley.size() / 2.0, galley, neutrals.text_primary);
    }
    response
}

/// Draws the "+" between two key caps.
fn plus(ui: &mut Ui) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let galley = plus_galley(ui);
    let size = Vec2::new(galley.size().x + CAP_GAP * 2.0, galley.size().y);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, neutrals.text_tertiary);
}

/// Draws the generated "Keyboard shortcuts" page: the heading, a filter field
/// and then one table per group, in the spec's group order.
pub fn shortcut_sheet(ui: &mut Ui, table: &ShortcutTable, filter: &mut String) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    ui.label(RichText::new(SHEET_TITLE).font(fonts::semibold(typography::TITLE_SIZE)).color(neutrals.text_primary));
    ui.add_space(space::SCALE_8);
    crate::search_field(ui, filter, FILTER_HINT);
    ui.add_space(space::SCALE_12);
    let query = filter.trim().to_lowercase();
    for group in sheet_groups(table) {
        let rows: Vec<&Shortcut> = table
            .entries()
            .iter()
            .filter(|shortcut| shortcut.group == group)
            .filter(|shortcut| row_matches(shortcut, &key_text(shortcut.keys.first()), &query))
            .collect();
        if rows.is_empty() {
            continue;
        }
        ui.label(RichText::new(group).font(fonts::semibold(typography::HEADLINE_SIZE)).color(neutrals.text_primary));
        ui.add_space(GROUP_GAP);
        for shortcut in rows {
            sheet_row(ui, shortcut);
        }
        ui.add_space(SHEET_GAP);
    }
}

/// The groups the sheet shows: the spec's order first, then any other group in
/// table order.
fn sheet_groups(table: &ShortcutTable) -> Vec<&str> {
    let mut groups: Vec<&str> = GROUP_ORDER.into_iter().filter(|group| table.entries().iter().any(|shortcut| shortcut.group == *group)).collect();
    for shortcut in table.entries() {
        if !groups.contains(&shortcut.group) {
            groups.push(shortcut.group);
        }
    }
    groups
}

/// True when a row is shown for a lowercased `query`: the label, the group or
/// the keys hold it; an empty query shows every row.
fn row_matches(shortcut: &Shortcut, keys_text: &str, query: &str) -> bool {
    query.is_empty()
        || shortcut.label.to_lowercase().contains(query)
        || shortcut.group.to_lowercase().contains(query)
        || keys_text.to_lowercase().contains(query)
}

/// One sheet row: the action name on the left, its key caps on the right, laid
/// out left to right so the caps read in order.
fn sheet_row(ui: &mut Ui, shortcut: &Shortcut) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let Some(key) = shortcut.keys.first() else {
        return;
    };
    let width = caps_width(ui, &cap_items(ui.ctx(), key));
    ui.horizontal(|ui| {
        ui.label(RichText::new(shortcut.label).font(fonts::medium(typography::BODY_SIZE)).color(neutrals.text_secondary));
        let space_left = (ui.available_width() - width).max(0.0);
        ui.add_space(space_left);
        key_caps(ui, key);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers};

    /// A small table, for the tests.
    fn table() -> ShortcutTable {
        ShortcutTable::new(vec![
            Shortcut {
                id: "play",
                label: "Play",
                group: "Playback",
                keys: vec![KeyboardShortcut::new(Modifiers::NONE, Key::Space), KeyboardShortcut::new(Modifiers::NONE, Key::K)],
                context: "playback",
            },
            Shortcut { id: "undo", label: "Undo", group: "Edit", keys: vec![KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)], context: "global" },
        ])
    }

    /// Runs one frame with `keys` pressed, calling `body` with that frame's
    /// context so `pressed` sees the key events.
    fn with_keys(keys: &[KeyboardShortcut], body: impl FnOnce(&Context)) {
        let ctx = Context::default();
        let mut input = egui::RawInput::default();
        for key in keys {
            input.events.push(egui::Event::Key { key: key.logical_key, physical_key: None, pressed: true, repeat: false, modifiers: key.modifiers });
        }
        let body = std::cell::RefCell::new(Some(body));
        let mut output = ctx.run_ui(input, |ui| {
            if let Some(body) = body.borrow_mut().take() {
                body(ui.ctx());
            }
        });
        output.textures_delta.clear();
    }

    /// pressed() matches the first key and an alternate, and consumes them.
    #[test]
    fn pressed_matches_alternates_and_consumes() {
        let table = table();
        with_keys(&[KeyboardShortcut::new(Modifiers::NONE, Key::K)], |ctx| {
            assert!(table.pressed(ctx, "play"), "the alternate key must match");
            assert!(!table.pressed(ctx, "play"), "the key must be consumed, so the second check is false");
            assert!(!table.pressed(ctx, "missing"), "an unknown id is never pressed");
        });
        with_keys(&[KeyboardShortcut::new(Modifiers::NONE, Key::Space)], |ctx| {
            assert!(table.pressed(ctx, "play"), "the first key must match");
        });
    }

    /// validate() flags two entries that share a key in the same context only.
    #[test]
    fn validate_flags_a_clash() {
        let clashing = ShortcutTable::new(vec![
            Shortcut { id: "a", label: "Alpha", group: "Edit", keys: vec![KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)], context: "global" },
            Shortcut { id: "b", label: "Beta", group: "Edit", keys: vec![KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)], context: "global" },
            Shortcut { id: "c", label: "Gamma", group: "Edit", keys: vec![KeyboardShortcut::new(Modifiers::COMMAND, Key::Y)], context: "global" },
        ]);
        let problems = clashing.validate();
        assert_eq!(problems.len(), 1, "exactly the one clash is flagged: {problems:?}");
        assert!(problems[0].contains("Alpha") && problems[0].contains("Beta"), "the message names both entries: {}", problems[0]);
        assert!(problems[0].contains("Ctrl+Z"), "the message names the key: {}", problems[0]);

        // The same keys in two different contexts are not a clash.
        let separate = ShortcutTable::new(vec![
            Shortcut { id: "a", label: "Alpha", group: "Edit", keys: vec![KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)], context: "global" },
            Shortcut { id: "b", label: "Beta", group: "Edit", keys: vec![KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)], context: "record" },
        ]);
        assert!(separate.validate().is_empty());
    }

    /// The sheet filter hides a row whose label, group and keys miss the query.
    #[test]
    fn sheet_filter_hides_non_matching_rows() {
        let table = table();
        let undo = table.get("undo").expect("undo exists");
        let play = table.get("play").expect("play exists");
        assert!(row_matches(play, "Space, K", ""), "an empty query shows every row");
        assert!(row_matches(play, "Space, K", "play"), "the label matches");
        assert!(row_matches(play, "Space, K", "playback"), "the group matches");
        assert!(!row_matches(undo, "Ctrl+Z", "play"), "a row with no match is hidden");
    }

    /// The sheet lists the spec's groups first and any other group after.
    #[test]
    fn sheet_groups_follow_the_spec_order() {
        let table = ShortcutTable::new(vec![
            Shortcut { id: "x", label: "X", group: "Custom", keys: vec![KeyboardShortcut::new(Modifiers::NONE, Key::X)], context: "global" },
            Shortcut { id: "y", label: "Y", group: "Playback", keys: vec![KeyboardShortcut::new(Modifiers::NONE, Key::Y)], context: "global" },
            Shortcut { id: "z", label: "Z", group: "Edit", keys: vec![KeyboardShortcut::new(Modifiers::NONE, Key::Z)], context: "global" },
        ]);
        assert_eq!(sheet_groups(&table), ["Edit", "Playback", "Custom"]);
    }

    /// The key caps split a shortcut into one cap per part.
    #[test]
    fn key_caps_split_a_shortcut_into_parts() {
        let ctx = Context::default();
        let items = cap_items(&ctx, &KeyboardShortcut::new(Modifiers::COMMAND, Key::Z));
        assert_eq!(items, [CapItem::Cap("Ctrl".to_owned()), CapItem::Plus, CapItem::Cap("Z".to_owned())]);
    }

    /// The sheet draws without panicking on a fresh context.
    #[test]
    fn shortcut_sheet_draws() {
        let ctx = Context::default();
        crate::apply(&ctx);
        let mut filter = String::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            shortcut_sheet(ui, &table(), &mut filter);
        });
        output.textures_delta.clear();
    }
}
