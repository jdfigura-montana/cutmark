//! The tab bar over the editor: one tab per open script (spec "Library window").

use ui_kit::tokens::{color, radius, size, space, typography};

use crate::scripts_panel::elided;
use crate::workspace::{OpenScript, Workspace};

/// A tab's width. The tabs share the bar between these two, and scroll past the narrower one.
const MIN_WIDTH: f32 = 112.0;
const MAX_WIDTH: f32 = 180.0;
/// A tab's inset at each end, and the room between its parts.
const INSET: f32 = space::SCALE_8;
const PART_GAP: f32 = space::SCALE_4;
/// Side of the slot at a tab's right end, which holds the unsaved dot or the close button.
const SLOT: f32 = 18.0;
/// Diameter of the unsaved dot (rev 3: 6px).
const DOT_DIAMETER: f32 = 6.0;
/// How strongly the accent color backs the Live badge.
const BADGE_TINT: f32 = 0.18;

/// What a click in the bar asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Activate(String),
    Close(String),
}

#[derive(Default)]
pub struct TabBar {
    /// The active tab, tab count and bar width that were last scrolled into view. Until one of
    /// them changes, the bar can be scrolled by hand.
    revealed: Option<(u64, usize, f32)>,
    /// Where the last frame drew each tab.
    #[cfg(test)]
    pub rects: Vec<egui::Rect>,
}

impl TabBar {
    /// Draws the tabs, filling a row. Nothing is drawn with none open.
    pub fn show(&mut self, ui: &mut egui::Ui, workspace: &Workspace) -> Option<Action> {
        #[cfg(test)]
        self.rects.clear();
        let tabs = workspace.tabs();
        if tabs.is_empty() {
            self.revealed = None;
            return None;
        }
        let active = workspace.open_script().map(|open| open.serial);
        let mut action = None;
        crate::app::panel_row(ui, |ui| {
            // The wheel scrolls the bar without Shift.
            ui.style_mut().always_scroll_the_only_direction = true;
            ui.spacing_mut().item_spacing.x = space::SCALE_2;
            let room = ui.available_width();
            let count = tabs.len() as f32;
            let width = ((room - (count - 1.0) * space::SCALE_2) / count).floor().clamp(MIN_WIDTH, MAX_WIDTH);
            let key = active.map(|serial| (serial, tabs.len(), room));
            let reveal = self.revealed != key;
            self.revealed = key;
            let bar = egui::ScrollArea::horizontal().id_salt("tab-bar").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
            bar.show(ui, |ui| {
                for script in tabs {
                    let shown = active == Some(script.serial);
                    let (response, picked) = tab(ui, script, workspace.title(&script.id), shown, width);
                    if reveal && shown {
                        response.scroll_to_me(None);
                    }
                    #[cfg(test)]
                    self.rects.push(response.rect);
                    action = action.take().or(picked);
                }
            });
        });
        action
    }
}

/// One tab: its title, the Live badge if it is live, and at its right end the close button
/// while the pointer is over the tab, or else the unsaved dot.
fn tab(ui: &mut egui::Ui, script: &OpenScript, title: &str, active: bool, width: f32) -> (egui::Response, Option<Action>) {
    let (_, rect) = ui.allocate_space(egui::vec2(width, size::ICON_BUTTON));
    let id = egui::Id::new(("script-tab", script.serial));
    let response = ui.interact(rect, id, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, title));
    // Not the response's hover, which the close button takes over.
    let hovered = ui.rect_contains_pointer(rect);
    let slot = egui::Rect::from_center_size(egui::pos2(rect.right() - INSET / 2.0 - SLOT / 2.0, rect.center().y), egui::Vec2::splat(SLOT));
    let close = hovered.then(|| ui.interact(slot, id.with("close"), egui::Sense::click()).on_hover_text("Close (Ctrl+W)"));

    let fill = if active {
        Some(color::BG_ROW_SELECTED)
    } else if hovered {
        Some(color::BG_GROUP)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, radius::SEGMENT, fill);
    }

    let mut end = slot.left() - PART_GAP;
    if script.live {
        end = live_badge(ui, end, rect.center().y) - PART_GAP;
    }
    let text_color = if active { color::TEXT_PRIMARY } else { color::TEXT_SECONDARY };
    let label = elided(ui, title, end - rect.left() - INSET);
    let cut = label != title;
    let galley = ui.painter().layout_no_wrap(label, ui_kit::fonts::medium(typography::BODY_SIZE), text_color);
    ui.painter().galley(egui::pos2(rect.left() + INSET, rect.center().y - galley.size().y / 2.0), galley, text_color);

    match &close {
        Some(close) => {
            if close.hovered() {
                ui.painter().circle_filled(slot.center(), SLOT / 2.0, color::BG_HOVER);
            }
            let icon_color = if close.hovered() { color::TEXT_PRIMARY } else { color::TEXT_SECONDARY };
            let font = ui_kit::fonts::icon_regular(typography::BODY_SIZE);
            ui.painter().text(slot.center(), egui::Align2::CENTER_CENTER, ui_kit::icons::regular::X, font, icon_color);
        }
        None if script.unsaved() => {
            ui.painter().circle_filled(slot.center(), DOT_DIAMETER / 2.0, color::TEXT_SECONDARY);
        }
        None => {}
    }

    let closed = close.as_ref().is_some_and(|close| close.clicked() || close.middle_clicked()) || response.middle_clicked();
    let action = if closed { Some(Action::Close(script.id.clone())) } else { response.clicked().then(|| Action::Activate(script.id.clone())) };
    let response = if cut { response.on_hover_text(title) } else { response };
    (response, action)
}

/// The badge of the tab a session is running on, ending at `right`. Returns its left edge.
fn live_badge(ui: &egui::Ui, right: f32, middle: f32) -> f32 {
    let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
    let galley = ui.painter().layout_no_wrap("Live".to_owned(), ui_kit::fonts::semibold(typography::CAPTION_SIZE), accent);
    let area = galley.size() + egui::vec2(2.0 * PART_GAP, space::SCALE_2);
    let rect = egui::Rect::from_min_size(egui::pos2(right - area.x, middle - area.y / 2.0), area);
    ui.painter().rect_filled(rect, radius::SEGMENT, accent.gamma_multiply(BADGE_TINT));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, accent);
    rect.left()
}
