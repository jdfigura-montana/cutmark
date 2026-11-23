//! The tool strip (spec "Components": row "Tool strip").

use std::sync::Arc;

use egui::{Color32, CornerRadius, Galley, Key, Pos2, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::fonts;
use crate::tokens::{color, radius, space, typography};
use crate::widgets::focus_ring;

/// The tool strip cell height (spec "Components": "Tool strip" 52 tall; no
/// size token of its own, so a named constant like the segmented padding).
const CELL_HEIGHT: f32 = 52.0;
/// Horizontal padding inside each cell (spec "Spacing and sizes": spacing 12).
const CELL_PADDING: f32 = space::SCALE_12;
/// Gap between neighboring cells (spec "Spacing and sizes": spacing 2).
const CELL_GAP: f32 = space::SCALE_2;
/// Gap between a cell's icon and its label (spec "Spacing and sizes": spacing 4).
const ICON_LABEL_GAP: f32 = space::SCALE_4;
/// The hover fill is `bg.hover` at half strength (spec row "Tool strip"
/// hover; not a token, so a named constant).
const HOVER_STRENGTH: f32 = 0.5;

/// The tool strip: equal-width cells, each a phosphor regular line icon over
/// a `type.micro` label, 52 tall, `radius.segment` rounding (the 9 standalone
/// cell radius the list row also uses). The selected cell fills `bg.hover`
/// and reads `text.primary`; the others read `text.secondary` and fill
/// `bg.hover` at half strength when hovered. Arrow keys move the selection
/// when a cell has the keyboard focus.
///
/// The cells' widget ids derive from their labels, so the labels must be
/// unique within one strip and two strips with identical labels must not
/// share a page.
///
/// Usage:
///
/// ```ignore
/// let mut tool = Tool::Razor;
/// ui_kit::tool_strip(ui, &mut tool, &[
///     (Tool::Select, ui_kit::icons::regular::CURSOR, "Select"),
///     (Tool::Razor, ui_kit::icons::regular::SCISSORS, "Razor"),
/// ]);
/// ```
pub fn tool_strip<T: PartialEq + Copy>(ui: &mut Ui, selected: &mut T, items: &[(T, &str /*icon*/, &str /*label*/)]) -> Response {
    let selected_index = items.iter().position(|(value, _, _)| *value == *selected);
    let icon_font = fonts::icon_regular(typography::HEADLINE_SIZE);
    let label_font = fonts::medium(typography::MICRO_SIZE);
    let galleys: Vec<(Arc<Galley>, Arc<Galley>)> = items
        .iter()
        .map(|(_, icon, label)| {
            (
                ui.painter().layout_no_wrap((*icon).to_owned(), icon_font.clone(), Color32::PLACEHOLDER),
                ui.painter().layout_no_wrap((*label).to_owned(), label_font.clone(), Color32::PLACEHOLDER),
            )
        })
        .collect();
    // Equal-width cells: every cell is as wide as the widest content, so the
    // strip reads as one tool group (spec row "Tool strip").
    let cell_width = galleys.iter().map(|(icon, label)| icon.size().x.max(label.size().x) + 2.0 * CELL_PADDING).fold(0.0, f32::max);
    let total_width = items.len() as f32 * cell_width + items.len().saturating_sub(1) as f32 * CELL_GAP;
    let (rect, strip) = ui.allocate_exact_size(vec2(total_width, CELL_HEIGHT), Sense::hover());
    strip.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Tool strip"));

    let mut focused: Option<Response> = None;
    let mut x = rect.left();
    for (index, ((value, _, label), (icon_galley, label_galley))) in items.iter().zip(&galleys).enumerate() {
        let cell_rect = Rect::from_min_size(Pos2::new(x, rect.top()), vec2(cell_width, CELL_HEIGHT));
        x += cell_width + CELL_GAP;
        let is_selected = Some(index) == selected_index;
        let cell = ui.interact(cell_rect, ui.id().with("tool_strip").with(*label), Sense::click());
        cell.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, is_selected, label));
        let hovered = cell.hovered();
        if cell.clicked() {
            *selected = *value;
        }
        if cell.has_focus() {
            let (right, left) = ui.input(|input| (input.key_pressed(Key::ArrowRight), input.key_pressed(Key::ArrowLeft)));
            if right {
                *selected = items[(index + 1).min(items.len() - 1)].0;
            }
            if left {
                *selected = items[index.saturating_sub(1)].0;
            }
            focused = Some(cell);
        }
        if ui.is_rect_visible(cell_rect) {
            if is_selected {
                ui.painter().rect_filled(cell_rect, radius::SEGMENT, color::BG_HOVER);
            } else if hovered {
                ui.painter().rect_filled(cell_rect, radius::SEGMENT, color::BG_HOVER.gamma_multiply(HOVER_STRENGTH));
            }
            let text = if is_selected { color::TEXT_PRIMARY } else { color::TEXT_SECONDARY };
            let content_height = icon_galley.size().y + ICON_LABEL_GAP + label_galley.size().y;
            let content_top = cell_rect.top() + (CELL_HEIGHT - content_height) / 2.0;
            let center_x = cell_rect.center().x;
            ui.painter().galley(Pos2::new(center_x - icon_galley.size().x / 2.0, content_top), icon_galley.clone(), text);
            ui.painter().galley(
                Pos2::new(center_x - label_galley.size().x / 2.0, content_top + icon_galley.size().y + ICON_LABEL_GAP),
                label_galley.clone(),
                text,
            );
        }
    }
    if let Some(cell) = focused {
        focus_ring(ui, cell.rect, CornerRadius::from(radius::SEGMENT));
    }
    strip
}
