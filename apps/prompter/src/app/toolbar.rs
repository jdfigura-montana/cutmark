//! The editor panel's formatting toolbar (spec "Library window", center panel). Every
//! control is a shortcut to a menu item.
//!
//! The panel is often too narrow for one row, so the toolbar wraps, a group at a time. The
//! Preview chip stays at the right end of the first row.

use egui::{Color32, Rect, Shape, pos2, vec2};
use ui_kit::icons::regular as icon;
use ui_kit::tokens::{color, radius, size, space, typography};

use super::{HAIRLINE, menu_rows};
use crate::formatting::{Action, ListKind};
use crate::markup::Inline;
use crate::menu::{CUES, Item, MenuState, STYLES};

/// Wide enough for the longest style name and the caret.
const STYLE_WIDTH: f32 = 104.0;
/// The room a separator takes, with its line in the middle.
const SEPARATOR: f32 = 9.0;
const SEPARATOR_HEIGHT: f32 = 16.0;
/// The Preview chip's padding at each side of its label, as ui-kit's chip has it.
const CHIP_PADDING: f32 = 11.0;
/// How strongly the accent color backs the button of a format that is on.
const ACTIVE_TINT: f32 = 0.35;
/// The gap between a button and the menu under it.
const MENU_GAP: f32 = space::SCALE_4;
/// The label of the cue menu's button.
pub const INSERT_CUE: &str = "Insert cue";

#[derive(Clone, Copy)]
enum Group {
    /// The paragraph style dropdown.
    Style,
    /// Buttons that each run their item.
    Buttons(&'static [(Item, &'static str)]),
    /// A rule's button, then the button that opens the cue menu.
    Insert,
}

const GROUPS: [Group; 7] = [
    Group::Style,
    Group::Buttons(&[
        (Item::Format(Action::Inline(Inline::Bold)), icon::TEXT_B),
        (Item::Format(Action::Inline(Inline::Italic)), icon::TEXT_ITALIC),
        (Item::Format(Action::Inline(Inline::Underline)), icon::TEXT_UNDERLINE),
        (Item::Format(Action::Inline(Inline::Strike)), icon::TEXT_STRIKETHROUGH),
    ]),
    Group::Buttons(&[
        (Item::Format(Action::List(ListKind::Bulleted)), icon::LIST_BULLETS),
        (Item::Format(Action::List(ListKind::Numbered)), icon::LIST_NUMBERS),
    ]),
    Group::Buttons(&[(Item::Format(Action::Outdent), icon::TEXT_OUTDENT), (Item::Format(Action::Indent), icon::TEXT_INDENT)]),
    Group::Insert,
    Group::Buttons(&[(Item::Undo, icon::ARROW_COUNTER_CLOCKWISE), (Item::Redo, icon::ARROW_CLOCKWISE)]),
    Group::Buttons(&[(Item::FindInScript, icon::MAGNIFYING_GLASS)]),
];

impl Group {
    fn width(self) -> f32 {
        match self {
            Self::Style => STYLE_WIDTH,
            Self::Buttons(buttons) => buttons.len() as f32 * size::ICON_BUTTON,
            Self::Insert => 2.0 * size::ICON_BUTTON,
        }
    }
}

/// Which row each group goes on: as many to a row as fit, in order. `first` is the room on
/// the first row and `rest` on the others.
fn rows(widths: &[f32], first: f32, rest: f32) -> Vec<usize> {
    let (mut row, mut used) = (0, 0.0);
    widths
        .iter()
        .map(|&width| {
            let room = if row == 0 { first } else { rest };
            if used > 0.0 && used + SEPARATOR + width > room {
                row += 1;
                used = 0.0;
            }
            used += if used > 0.0 { SEPARATOR + width } else { width };
            row
        })
        .collect()
}

/// Draws the toolbar. Returns the menu item of the control used. The Preview chip sets
/// `preview` itself.
pub(super) fn show(ui: &mut egui::Ui, state: MenuState, preview: &mut bool) -> Option<Item> {
    let width = ui.available_width();
    let chip_text = ui.painter().layout_no_wrap("Preview".to_owned(), ui_kit::fonts::medium(typography::BODY_SIZE), Color32::PLACEHOLDER);
    let chip = chip_text.size().x + 2.0 * CHIP_PADDING;
    let widths = GROUPS.map(Group::width);
    let placed = rows(&widths, width - chip - space::SCALE_4, width);
    let count = placed.last().map_or(1, |last| last + 1);
    // One row is as tall as any panel's toolbar. Each further one adds a button's height.
    let height = size::PANEL_TOOLBAR_HEIGHT + (count - 1) as f32 * size::ICON_BUTTON;
    let (area, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    let top = area.top() + (size::PANEL_TOOLBAR_HEIGHT - size::ICON_BUTTON) / 2.0;
    let row_rect = |row: usize| Rect::from_min_size(pos2(area.left(), top + row as f32 * size::ICON_BUTTON), vec2(width, size::ICON_BUTTON));

    // Each row is a child that leaves this ui's cursor where it is, under the whole toolbar.
    // A scope would move it to under the last one drawn, and the editor over the rows below.
    let mut picked = None;
    for row in 0..count {
        let layout = egui::UiBuilder::new().id_salt(("toolbar-row", row)).max_rect(row_rect(row)).layout(egui::Layout::left_to_right(egui::Align::Center));
        let mut ui = ui.new_child(layout);
        ui.spacing_mut().item_spacing.x = 0.0;
        let on_row = GROUPS.iter().zip(&placed).filter(|(_, placed)| **placed == row);
        for (index, (&group, _)) in on_row.enumerate() {
            if index > 0 {
                separator(&mut ui);
            }
            let used = match group {
                Group::Style => style_dropdown(&mut ui, state),
                Group::Buttons(buttons) => buttons.iter().fold(None, |used, &(item, glyph)| button(&mut ui, item, glyph, state).or(used)),
                Group::Insert => insert_buttons(&mut ui, state),
            };
            picked = picked.or(used);
        }
    }
    let right = egui::UiBuilder::new().id_salt("toolbar-preview").max_rect(row_rect(0)).layout(egui::Layout::right_to_left(egui::Align::Center));
    ui.new_child(right).add_enabled_ui(state.script_open, |ui| ui_kit::widgets::chip(ui, preview, "Preview"));
    ui.painter().hline(area.x_range().expand(space::PANEL_PADDING), area.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
    picked
}

fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(SEPARATOR, size::ICON_BUTTON), egui::Sense::hover());
    let half = SEPARATOR_HEIGHT / 2.0;
    ui.painter().vline(rect.center().x, rect.center().y - half..=rect.center().y + half, egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
}

/// The icon button of `item`, backed with the accent color while its format is on.
fn button(ui: &mut egui::Ui, item: Item, glyph: &str, state: MenuState) -> Option<Item> {
    let on = matches!(item, Item::Format(action) if state.editor.format.has(action));
    let tooltip = if item.chord().is_empty() { item.label().to_owned() } else { format!("{} ({})", item.label(), item.chord()) };
    ui.add_enabled_ui(item.enabled(state), |ui| {
        // ui-kit's icon button has no look for "on", so its back is painted under it.
        let back = ui.painter().add(Shape::Noop);
        let response = ui_kit::icon_button(ui, glyph, &tooltip);
        if on {
            let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
            ui.painter().set(back, Shape::circle_filled(response.rect.center(), size::ICON_BUTTON / 2.0, accent.gamma_multiply(ACTIVE_TINT)));
        }
        response.clicked().then_some(item)
    })
    .inner
}

/// The dropdown that shows the caret line's style and sets it.
fn style_dropdown(ui: &mut egui::Ui, state: MenuState) -> Option<Item> {
    let current = Action::Style(state.editor.format.style).label();
    let text = egui::TextFormat::simple(ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_PRIMARY);
    let caret = egui::TextFormat::simple(ui_kit::fonts::icon_regular(typography::CAPTION_SIZE), color::TEXT_SECONDARY);
    let mut job = egui::text::LayoutJob::default();
    job.append(current, 0.0, text);
    job.append(icon::CARET_DOWN, space::SCALE_6, caret);
    let response = ui
        .add_enabled_ui(state.script_open, |ui| ui.add_sized(vec2(STYLE_WIDTH, size::CHIP_HEIGHT), egui::Button::new(job)))
        .inner
        .on_hover_text("Paragraph style");
    menu_under(&response, |ui| menu_rows(ui, &[STYLES], state, false))
}

/// The horizontal rule button, and the button with the cue menu under it.
fn insert_buttons(ui: &mut egui::Ui, state: MenuState) -> Option<Item> {
    let rule = button(ui, Item::Format(Action::Rule), icon::MINUS, state);
    let response = ui.add_enabled_ui(state.script_open, |ui| ui_kit::icon_button(ui, icon::BRACKETS_SQUARE, INSERT_CUE)).inner;
    let cue = menu_under(&response, |ui| {
        let mut picked = None;
        // The menu names the cues alone.
        for (&item, name) in CUES.iter().zip(["Note", "Pause", "Mark", "End"]) {
            if ui.add_enabled(item.enabled(state), egui::Button::new(name)).clicked() {
                picked = Some(item);
                ui.close();
            }
        }
        picked
    });
    rule.or(cue)
}

/// A menu that a click on `button` opens under it, in the look of the menu bar's.
fn menu_under(button: &egui::Response, contents: impl FnOnce(&mut egui::Ui) -> Option<Item>) -> Option<Item> {
    let ctx = button.ctx.clone();
    let frame = ui_kit::glass_frame(&ctx, radius::GROUP).inner_margin(egui::Margin::same(space::SCALE_6 as i8));
    egui::Popup::menu(button).frame(frame).gap(MENU_GAP).show(contents).and_then(|shown| shown.inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_that_does_not_fit_starts_the_next_row() {
        let widths = [100.0, 100.0, 100.0, 100.0];
        assert_eq!(rows(&widths, 1000.0, 1000.0), [0, 0, 0, 0]);
        // Two groups and the separator between them need 209.
        assert_eq!(rows(&widths, 209.0, 209.0), [0, 0, 1, 1]);
        assert_eq!(rows(&widths, 208.0, 320.0), [0, 1, 1, 1]);
        // A group wider than the row still gets one.
        assert_eq!(rows(&widths, 50.0, 50.0), [0, 1, 2, 3]);
    }

    #[test]
    fn the_toolbar_has_menu_bar_items_only_each_once() {
        let listed: Vec<Item> = crate::menu::MENUS.iter().flat_map(|(_, groups)| groups.iter().copied().flatten().copied()).collect();
        let buttons = GROUPS.iter().filter_map(|group| match group {
            Group::Buttons(buttons) => Some(buttons.iter().map(|(item, _)| *item)),
            _ => None,
        });
        for item in buttons.flatten().chain(STYLES.iter().chain(CUES).copied()) {
            assert!(listed.contains(&item), "{item:?}");
        }
    }
}
