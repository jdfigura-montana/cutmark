//! The track header (spec "Components": row "Track header", "Layout rules" item 5).

use egui::{Color32, CornerRadius, Galley, Pos2, Rect, Response, Sense, Ui, UiBuilder, WidgetInfo, WidgetType, vec2};

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
    if selected {
        // The selected header shares its row's `bg.track.selected`, so the
        // selection spans the header column too (spec "Track row").
        painter.rect_filled(rect, CornerRadius::ZERO, color::BG_TRACK_SELECTED);
    }
    let number_color = if selected { color::TEXT_PRIMARY } else { color::TEXT_TERTIARY };
    let number_galley = painter.layout_no_wrap(number.to_string(), fonts::mono(typography::VALUE_SIZE), number_color);
    let name_galley = painter.layout_no_wrap(name.to_owned(), fonts::semibold(typography::HEADLINE_SIZE), color::TEXT_PRIMARY);
    let line_height = name_galley.size().y.max(number_galley.size().y);
    let mut y = rect.top() + pad;
    let name_x = rect.left() + pad + number_galley.size().x + space::SCALE_8;
    // One baseline (spec "Track header"): shift the name by the difference
    // between the two fonts' first baselines.
    let name_y = y + galley_baseline(&number_galley) - galley_baseline(&name_galley);
    painter.galley(Pos2::new(rect.left() + pad, y), number_galley, number_color);
    painter.with_clip_rect(Rect::from_min_max(Pos2::new(rect.left() + pad, y), Pos2::new(rect.right() - pad, y + line_height))).galley(
        Pos2::new(name_x, name_y),
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

/// The y of the first text baseline within a single-row galley, so the track
/// number (mono) and the name (Inter) can share one baseline (spec
/// "Track header"). `Glyph::pos` is the baseline position within its row.
fn galley_baseline(galley: &Galley) -> f32 {
    galley.rows.first().map_or(galley.size().y, |row| row.pos.y + row.glyphs.first().map_or(row.size.y, |glyph| glyph.pos.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number (mono) and name (Inter) galleys give usable baselines, and
    /// shifting the name by the baseline difference puts both on one line.
    #[test]
    fn the_number_and_name_galleys_share_one_baseline() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::fonts::definitions());
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let painter = egui::Painter::new(ctx, egui::LayerId::background(), Rect::EVERYTHING);
        let number = painter.layout_no_wrap("3".to_owned(), fonts::mono(typography::VALUE_SIZE), Color32::WHITE);
        let name = painter.layout_no_wrap("Voice".to_owned(), fonts::semibold(typography::HEADLINE_SIZE), Color32::WHITE);
        for (galley, what) in [(&number, "number"), (&name, "name")] {
            let baseline = galley_baseline(galley);
            assert!(baseline > 0.0 && baseline <= galley.size().y, "{what} baseline {baseline} must sit inside its galley");
        }
        let name_y = galley_baseline(&number) - galley_baseline(&name);
        assert!((name_y + galley_baseline(&name) - galley_baseline(&number)).abs() < 1e-4, "after the shift both baselines coincide");
    }
}
