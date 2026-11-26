//! The title bar's center block (spec: Design Language rev 3 "Title bar",
//! center zone): the document title on one line, or the app name.

use egui::text::LayoutJob;
use egui::{Painter, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{space, typography};

/// Diameter of the unsaved dot (spec rev 3: 6px).
const DOT_DIAMETER: f32 = 6.0;
/// Gap between the name, the dot and the caption.
const GAP: f32 = space::SCALE_8;
/// The narrowest the name is cut to before the caption is dropped.
const MIN_NAME_WIDTH: f32 = 96.0;
/// Slack when comparing a measured width with the room, so a block that was
/// measured to fit never truncates on the second layout.
const FIT_SLACK: f32 = 0.5;

/// What the title bar's center shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Title<'a> {
    /// No document is open: the app name in `text.secondary`.
    App(&'a str),
    /// An open document: the name in `type.body` 600 `text.primary`, a 6px
    /// `text.secondary` dot when it has unsaved changes, then a caption in
    /// `type.caption` `text.tertiary` (format details, for example
    /// "48 kHz \u{b7} 24-bit \u{b7} 4 tracks").
    Document {
        /// The document name.
        name: &'a str,
        /// True shows the unsaved dot.
        unsaved: bool,
        /// The caption after the name, if any.
        caption: Option<&'a str>,
    },
}

/// The title block: one line, vertically centered in its layout. Put it in
/// the center closure of [`crate::widgets::main_toolbar`], which centers the
/// zone on the window. A name that does not fit in the available width is cut
/// with an ellipsis; the caption goes first when even a short name would not
/// fit.
///
/// Usage:
///
/// ```ignore
/// ui_kit::title_block(ui, &ui_kit::Title::Document { name: "Take 3", unsaved: true, caption: Some("48 kHz") });
/// ```
pub fn title_block(ui: &mut Ui, title: &Title<'_>) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let available = ui.available_width();
    // An inactive window dims toolbar text (main_toolbar sets the override).
    let dim = ui.visuals().override_text_color == Some(neutrals.text_tertiary);
    let painter = ui.painter().clone();
    let (name, name_color, unsaved, caption) = match *title {
        Title::App(name) => (name, if dim { neutrals.text_tertiary } else { neutrals.text_secondary }, false, None),
        Title::Document { name, unsaved, caption } => (name, if dim { neutrals.text_tertiary } else { neutrals.text_primary }, unsaved, caption),
    };
    let name_font = fonts::semibold(typography::BODY_SIZE);
    let caption_galley =
        caption.map(|text| painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(typography::CAPTION_SIZE), neutrals.text_tertiary));
    let dot_width = if unsaved { GAP + DOT_DIAMETER } else { 0.0 };
    let caption_width = caption_galley.as_ref().map_or(0.0, |galley| GAP + galley.size().x);
    let natural_name = painter.layout_no_wrap(name.to_owned(), name_font.clone(), name_color);

    // Make it fit: first the name gets shorter, then the caption goes.
    let mut caption_galley = caption_galley;
    let mut caption_width = caption_width;
    let mut name_room = available - dot_width - caption_width;
    if natural_name.size().x + dot_width + caption_width > available + FIT_SLACK && name_room < MIN_NAME_WIDTH {
        caption_galley = None;
        caption_width = 0.0;
        name_room = available - dot_width;
    }
    let name_galley = if natural_name.size().x <= name_room + FIT_SLACK { natural_name } else { truncated(&painter, name, name_font, name_color, name_room) };

    let height = name_galley.size().y.max(caption_galley.as_ref().map_or(0.0, |galley| galley.size().y));
    let (rect, response) = ui.allocate_exact_size(vec2(name_galley.size().x + dot_width + caption_width, height), Sense::hover());
    // The toolbar measures this block in an invisible pass first; only the
    // real one belongs in the accessibility tree.
    if !ui.is_sizing_pass() {
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let mut x = rect.left();
    painter.galley_with_override_text_color(egui::pos2(x, rect.center().y - name_galley.size().y / 2.0), name_galley.clone(), name_color);
    x += name_galley.size().x;
    if unsaved {
        let center = egui::pos2(x + GAP + DOT_DIAMETER / 2.0, rect.center().y);
        painter.circle_filled(center, DOT_DIAMETER / 2.0, if dim { neutrals.text_tertiary } else { neutrals.text_secondary });
        x += dot_width;
    }
    if let Some(galley) = caption_galley {
        painter.galley_with_override_text_color(egui::pos2(x + GAP, rect.center().y - galley.size().y / 2.0), galley, neutrals.text_tertiary);
    }
    response
}

/// The name on one row, cut to `max_width` with an ellipsis.
fn truncated(painter: &Painter, name: &str, font: egui::FontId, text_color: egui::Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple(name.to_owned(), font, text_color, max_width.max(0.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    painter.layout_job(job)
}
