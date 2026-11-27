//! The inspector: header, group and row (spec "Components": rows "Inspector
//! header", "Inspector group").

use egui::{Align, InnerResponse, Layout, Pos2, Response, Sense, Stroke, Ui, UiBuilder, vec2};

use crate::HAIRLINE;
use crate::Theme;
use crate::fonts;
use crate::tokens::{radius, size, space, typography};

/// The temp-data id counting the rows drawn so far inside the current group.
fn row_counter_id(ui: &Ui) -> egui::Id {
    ui.id().with("inspector_row_index")
}

/// The inspector header: the `type.title` name over a `type.caption`
/// subtitle (track, source file, state).
///
/// Usage:
///
/// ```ignore
/// ui_kit::inspector_header(ui, "Interview.mp4", "Source file");
/// ```
pub fn inspector_header(ui: &mut Ui, title: &str, subtitle: &str) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(title).font(fonts::semibold(typography::TITLE_SIZE)).color(neutrals.text_primary));
        ui.label(egui::RichText::new(subtitle).font(fonts::medium(typography::CAPTION_SIZE)).color(neutrals.text_tertiary));
    })
    .response
}

/// The inspector group: `bg.group`, `radius.group`, rows separated by a 1px
/// `stroke.separator` line. Wrap [`inspector_row`] calls in `add_rows`.
///
/// Usage:
///
/// ```ignore
/// ui_kit::inspector_group(ui, |ui| {
///     ui_kit::inspector_row(ui, "Gain", |ui| ui.label("0.0 dB"), None);
/// });
/// ```
pub fn inspector_group<R>(ui: &mut Ui, add_rows: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    egui::Frame::new().fill(neutrals.bg_group).corner_radius(radius::GROUP).inner_margin(egui::Margin::same(space::SCALE_4 as i8)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.data_mut(|data| data.insert_temp(row_counter_id(ui), 0u32));
        add_rows(ui)
    })
}

/// One inspector row: 42 tall, the label, a control between label and value,
/// and the value in `type.value` `text.primary`. Rows after the first draw a
/// 1px `stroke.separator` line at their top (spec "Inspector group").
///
/// Usage:
///
/// ```ignore
/// ui_kit::inspector_row(ui, "Gain", |ui| ui.label("0.0 dB"), Some("0.0 dB"));
/// ```
pub fn inspector_row(ui: &mut Ui, label: &str, add_control: impl FnOnce(&mut Ui), value: Option<&str>) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let counter = row_counter_id(ui);
    let index = ui.data_mut(|data| {
        let counter = data.get_temp_mut_or_insert_with(counter, || 0u32);
        let index = *counter;
        *counter += 1;
        index
    });
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::INSPECTOR_ROW_HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if index > 0 {
            painter.line_segment([Pos2::new(rect.left(), rect.top()), Pos2::new(rect.right(), rect.top())], Stroke::new(HAIRLINE, neutrals.stroke_separator));
        }
        let inset = space::SCALE_4;
        let label_galley = painter.layout_no_wrap(label.to_owned(), fonts::medium(typography::BODY_SIZE), neutrals.text_secondary);
        let label_pos = Pos2::new(rect.left() + inset, rect.center().y - label_galley.size().y / 2.0);
        let value_galley = value
            .map(|value| painter.layout_job(fonts::tracked(value, fonts::mono(typography::VALUE_SIZE), neutrals.text_primary, fonts::MONO_LETTER_SPACING_EM)));
        let value_width = value_galley.as_ref().map_or(0.0, |galley| galley.size().x);
        let control_left = label_pos.x + label_galley.size().x + space::SCALE_8;
        let control_right = (rect.right() - inset - value_width - space::SCALE_8).max(control_left);
        let control_rect = egui::Rect::from_min_max(Pos2::new(control_left, rect.top()), Pos2::new(control_right, rect.bottom()));
        let mut control_ui = ui.new_child(UiBuilder::new().max_rect(control_rect).layout(Layout::left_to_right(Align::Center)));
        add_control(&mut control_ui);
        let painter = ui.painter();
        painter.galley(label_pos, label_galley, neutrals.text_secondary);
        if let Some(galley) = value_galley {
            let pos = Pos2::new(rect.right() - inset - galley.size().x, rect.center().y - galley.size().y / 2.0);
            painter.galley(pos, galley, neutrals.text_primary);
        }
    }
    response
}
