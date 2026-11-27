//! The segmented control (spec "Components": row "Segmented control").

use std::sync::Arc;

use egui::{Color32, Galley, Id, Key, Pos2, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{color, radius, size, space, typography};
use crate::widgets::focus_ring;

/// Track padding around the segments (spec "Segmented control": padding 3;
/// not on the 2-based spacing scale, so a named constant of its own).
const TRACK_PADDING: f32 = 3.0;
/// Horizontal padding inside each segment (spec "Spacing and sizes":
/// segment 28 tall with 12 horizontal padding).
const SEGMENT_PADDING: f32 = space::SCALE_12;
/// The 1px shadow under the selected segment (spec "Segmented control").
const SEGMENT_SHADOW_OFFSET: f32 = 1.0;

/// The segmented control: a `bg.control` track with `radius.control`,
/// padding 3, the selected segment `bg.selected` with a 1px shadow and
/// `radius.segment`, 28 tall. Arrow keys move the selection when focused.
///
/// `id_salt` must be unique per control on the page: the segments' ids are
/// derived from it, and several controls with the same labels must never
/// share ids (egui reports reused ids as red "First use / Second use of
/// widget ID" overlays).
///
/// Usage:
///
/// ```ignore
/// let mut mode = 0;
/// ui_kit::segmented(ui, "modes", &mut mode, &["Edit", "Mix"]);
/// ```
pub fn segmented(ui: &mut Ui, id_salt: impl Into<Id>, selected: &mut usize, labels: &[&str]) -> Response {
    let font = fonts::medium(typography::BODY_SIZE);
    let galleys: Vec<_> = labels.iter().map(|label| ui.painter().layout_no_wrap((*label).to_owned(), font.clone(), Color32::PLACEHOLDER)).collect();
    let cell_widths: Vec<f32> = galleys.iter().map(|galley| galley.size().x + 2.0 * SEGMENT_PADDING).collect();
    segmented_impl(ui, id_salt, selected, labels, &galleys, &cell_widths, &[])
}

/// The segmented control filling the available width with equal cells, for a
/// control that must span its row (an inspector row's control area), with one
/// tooltip per segment (pass `&[]` for none).
///
/// `id_salt` must be unique per control on the page, as with [`segmented`].
///
/// Usage:
///
/// ```ignore
/// let mut view = 0;
/// ui_kit::segmented_filled(ui, "view", &mut view, &["Screen", "PiP", "Cam"], &["Screen only", "Picture in picture", "Cam only"]);
/// ```
pub fn segmented_filled(ui: &mut Ui, id_salt: impl Into<Id>, selected: &mut usize, labels: &[&str], tooltips: &[&str]) -> Response {
    let font = fonts::medium(typography::BODY_SIZE);
    let galleys: Vec<_> = labels.iter().map(|label| ui.painter().layout_no_wrap((*label).to_owned(), font.clone(), Color32::PLACEHOLDER)).collect();
    // The track padding comes off the width first, then the rest splits evenly,
    // so every cell is the same width and the track fills the available width.
    let cell_width = ((ui.available_width() - 2.0 * TRACK_PADDING) / labels.len().max(1) as f32).max(1.0);
    let cell_widths = vec![cell_width; labels.len()];
    segmented_impl(ui, id_salt, selected, labels, &galleys, &cell_widths, tooltips)
}

/// The drawing both segmented variants share: one width per cell, and an
/// optional tooltip per segment.
fn segmented_impl(
    ui: &mut Ui,
    id_salt: impl Into<Id>,
    selected: &mut usize,
    labels: &[&str],
    galleys: &[Arc<Galley>],
    cell_widths: &[f32],
    tooltips: &[&str],
) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let id_salt = id_salt.into();
    let height = size::SEGMENT_HEIGHT;
    let total_width: f32 = cell_widths.iter().sum::<f32>() + 2.0 * TRACK_PADDING;
    let (rect, track) = ui.allocate_exact_size(vec2(total_width, height + 2.0 * TRACK_PADDING), Sense::hover());
    track.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Segmented control"));

    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, radius::CONTROL, neutrals.bg_control);
    }

    let mut x = rect.left() + TRACK_PADDING;
    let mut focused: Option<Response> = None;
    for (index, galley) in galleys.iter().enumerate() {
        let segment_rect = Rect::from_min_size(Pos2::new(x, rect.top() + TRACK_PADDING), vec2(cell_widths[index], height));
        x += cell_widths[index];
        let is_selected = index == *selected;
        let segment = ui.interact(segment_rect, ui.id().with(id_salt).with("segmented_segment").with(index), Sense::click());
        segment.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, is_selected, labels[index]));
        let segment = match tooltips.get(index).copied().filter(|text| !text.is_empty()) {
            Some(text) => segment.on_hover_text(text),
            None => segment,
        };
        if segment.clicked() {
            *selected = index;
        }
        if segment.has_focus() {
            let (right, left) = ui.input(|input| (input.key_pressed(Key::ArrowRight), input.key_pressed(Key::ArrowLeft)));
            if right {
                *selected = (*selected + 1).min(labels.len().saturating_sub(1));
            }
            if left {
                *selected = selected.saturating_sub(1);
            }
            focused = Some(segment);
        }
        if ui.is_rect_visible(segment_rect) {
            if is_selected {
                // The 1px shadow: the glass shadow color, offset 1px down
                // under the selected segment (spec "Segmented control").
                ui.painter().rect_filled(segment_rect.translate(vec2(0.0, SEGMENT_SHADOW_OFFSET)), radius::SEGMENT, color::GLASS_SHADOW);
                ui.painter().rect_filled(segment_rect, radius::SEGMENT, neutrals.bg_selected);
            }
            let text = if is_selected { neutrals.text_primary } else { neutrals.text_secondary };
            ui.painter().galley(segment_rect.center() - galley.size() / 2.0, galley.clone(), text);
        }
    }
    if let Some(segment) = focused {
        focus_ring(ui, segment.rect, egui::CornerRadius::from(radius::SEGMENT));
    }
    track
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The filled control spans the width it is given and its cells are equal.
    #[test]
    fn filled_control_fills_the_width_with_equal_cells() {
        let ctx = egui::Context::default();
        crate::apply(&ctx);
        let mut selected = 0usize;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 60.0))));
            let track = segmented_filled(&mut child, "filled-cells", &mut selected, &["A", "B", "C"], &[]);
            assert_eq!(track.rect.width(), 300.0, "the track fills the available width");
            let cell_width = (300.0 - 2.0 * TRACK_PADDING) / 3.0;
            for index in 0..3 {
                // The id salt goes in as an `Id`, so it must be queried as one: `Id::with`
                // hashes the type of what it is given, and `Id::new("x")` != `"x"`.
                let id = child.id().with(egui::Id::new("filled-cells")).with("segmented_segment").with(index);
                let segment = ctx.read_response(id).expect("every segment registers a response");
                assert!((segment.rect.width() - cell_width).abs() < 0.01, "every cell is equally wide");
            }
        });
        output.textures_delta.clear();
        assert_eq!(selected, 0);
    }
}
