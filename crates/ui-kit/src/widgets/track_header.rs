//! The track header (spec "Components": row "Track header", "Layout rules" item 5).

use egui::{Color32, Pos2, Rect, Response, Sense, Ui, UiBuilder, WidgetInfo, WidgetType, vec2};

use crate::fonts;
use crate::meter;
use crate::tokens::{color, space, typography};

/// Side of a toggle square (spec "Track header": 24 x 24).
const TOGGLE_SIDE: f32 = 24.0;
/// Corner radius of a toggle square.
const TOGGLE_ROUNDING: f32 = 6.0;
/// Gap between neighbouring toggles.
const TOGGLE_GAP: f32 = 4.0;
/// Height of the horizontal level meter bar (spec "Meter": 6px bars).
const METER_HEIGHT: f32 = 6.0;

/// What a [`Toggle`] does, which decides its on color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleKind {
    /// A plain editing toggle: on is `bg.selected`.
    Normal,
    /// The record arm: on is `sem.record` with a white label.
    RecordArm,
}

/// One 24 x 24 toggle button in a track header.
pub struct Toggle<'a> {
    /// The letter drawn in the square, e.g. "M", "S" or "R".
    pub label: &'a str,
    /// The toggle state, flipped on click.
    pub on: &'a mut bool,
    /// The toggle's kind, which decides its on color.
    pub kind: ToggleKind,
    /// Show on every header when false; otherwise only when the header is
    /// selected or hovered ("Layout rules" item 5).
    pub always_visible: bool,
}

impl<'a> Toggle<'a> {
    /// A plain toggle that only shows when the header is selected or hovered.
    pub fn hidden(label: &'a str, on: &'a mut bool) -> Self {
        Self { label, on, kind: ToggleKind::Normal, always_visible: false }
    }

    /// A plain toggle that shows on every header.
    pub fn visible(label: &'a str, on: &'a mut bool) -> Self {
        Self { label, on, kind: ToggleKind::Normal, always_visible: true }
    }
}

/// The track header: the track number in `type.value` (`text.tertiary`,
/// `text.primary` when selected), the name in `type.headline`, the toggles as
/// 24 x 24 rounded squares, and an optional compact horizontal level meter
/// under the toggles. The whole header reports a click, which the caller
/// turns into a selection.
pub fn track_header(ui: &mut Ui, number: usize, name: &str, selected: bool, toggles: &mut [Toggle], meter: Option<(f32, bool)>) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ui.available_height()), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, name));
    // rect_contains_pointer rather than response.hovered(): the toggles below
    // take the pointer while visible, which must not hide them again.
    let hovered = ui.rect_contains_pointer(rect);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    let pad = space::SCALE_2;
    let number_color = if selected { color::TEXT_PRIMARY } else { color::TEXT_TERTIARY };
    let number_galley = painter.layout_no_wrap(number.to_string(), fonts::mono(typography::VALUE_SIZE), number_color);
    let name_galley = painter.layout_no_wrap(name.to_owned(), fonts::semibold(typography::HEADLINE_SIZE), color::TEXT_PRIMARY);
    let line_height = name_galley.size().y.max(number_galley.size().y);
    let mut y = rect.top() + pad;
    let name_x = rect.left() + pad + number_galley.size().x + space::SCALE_2;
    painter.galley(Pos2::new(rect.left() + pad, y + (line_height - number_galley.size().y) / 2.0), number_galley, number_color);
    painter.with_clip_rect(Rect::from_min_max(Pos2::new(rect.left() + pad, y), Pos2::new(rect.right() - pad, y + line_height))).galley(
        Pos2::new(name_x, y + (line_height - name_galley.size().y) / 2.0),
        name_galley,
        color::TEXT_PRIMARY,
    );
    y += line_height + space::SCALE_2;
    let mut x = rect.left() + pad;
    for (index, toggle) in toggles.iter_mut().enumerate() {
        let toggle_rect = Rect::from_min_size(Pos2::new(x, y), vec2(TOGGLE_SIDE, TOGGLE_SIDE));
        x = toggle_rect.right() + TOGGLE_GAP;
        if !(toggle.always_visible || selected || hovered) {
            continue;
        }
        let toggle_response = ui.interact(toggle_rect, response.id.with(("track_header_toggle", index)), Sense::click());
        toggle_response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, *toggle.on, toggle.label));
        if toggle_response.clicked() {
            *toggle.on = !*toggle.on;
        }
        let (fill, glyph_color) = match (toggle.kind, *toggle.on) {
            (ToggleKind::RecordArm, true) => (color::SEM_RECORD, Color32::WHITE),
            (_, true) => (color::BG_SELECTED, color::TEXT_PRIMARY),
            (_, false) => (color::BG_CONTROL, color::TEXT_TERTIARY),
        };
        painter.rect_filled(toggle_rect, TOGGLE_ROUNDING, fill);
        let glyph = painter.layout_no_wrap(toggle.label.to_owned(), fonts::semibold(typography::CAPTION_SIZE), glyph_color);
        let glyph_pos = Pos2::new(toggle_rect.center().x - glyph.size().x / 2.0, toggle_rect.center().y - glyph.size().y / 2.0);
        painter.with_clip_rect(toggle_rect).galley(glyph_pos, glyph, glyph_color);
    }
    y += TOGGLE_SIDE + space::SCALE_2;
    if let Some((peak_dbfs, clipped)) = meter {
        let meter_rect = Rect::from_min_max(Pos2::new(rect.left() + pad, y), Pos2::new(rect.right() - pad, y + METER_HEIGHT));
        let mut meter_ui = ui.new_child(UiBuilder::new().max_rect(meter_rect));
        meter::level_meter(&mut meter_ui, peak_dbfs, clipped, meter_rect.width());
    }
    response
}
