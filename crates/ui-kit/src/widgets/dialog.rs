//! The dialog (spec "Components": row "Dialogs"): `bg.panel`, `radius.panel`,
//! 24 px padding, centered over a dimmed backdrop, the primary button at the
//! bottom right. Esc and a backdrop click close it.

use egui::{Align, Context, Id, Layout, Margin, Pos2, Rect, Stroke, Ui, UiBuilder};

use crate::HAIRLINE;
use crate::fonts;
use crate::tokens::{color, radius, space, typography};

/// The margin every dialog keeps to the screen edges, on every side.
const SCREEN_MARGIN: f32 = 48.0;

/// The tallest the dialog footer row can get (one row of buttons).
const FOOTER_MAX_HEIGHT: f32 = 40.0;

/// The dialog: a modal (egui [`egui::Modal`]) with a `type.headline` title,
/// `add_body`, and a right-aligned `add_footer` row laid out left to right,
/// so the primary button added last lands at the bottom right. Closing the
/// dialog sets `*open` to false: Esc on the topmost modal, a click on the
/// backdrop, or the app's own buttons.
///
/// Usage:
///
/// ```ignore
/// ui_kit::dialog(ctx, egui::Id::new("delete-confirm"), "Delete clip?", &mut open,
///     |ui| { ui.label("This cannot be undone."); },
///     |ui| {
///         if ui.button("Cancel").clicked() { open = false; }
///         if ui.button("Delete").clicked() { delete = true; }
///     });
/// ```
pub fn dialog(ctx: &Context, id: Id, title: &str, open: &mut bool, add_body: impl FnOnce(&mut Ui), add_footer: impl FnOnce(&mut Ui)) {
    if !*open {
        return;
    }
    // Dialogs are panels: `bg.panel`, `radius.panel`, hairline rim, flat.
    let frame = egui::Frame::new()
        .fill(color::BG_PANEL)
        .corner_radius(radius::PANEL)
        .inner_margin(Margin::same(space::SCALE_24 as i8))
        .stroke(Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
    let response = egui::Modal::new(id).frame(frame).show(ctx, |ui| {
        // A modal has no width limit of its own, so a body that asks for more
        // than the screen has would grow past the window edges; clamp the
        // whole dialog to the screen rect minus the margin on every side.
        let screen = ctx.input(|input| input.content_rect());
        let max_width = (screen.width() - 2.0 * SCREEN_MARGIN).max(0.0);
        let max_height = (screen.height() - 2.0 * SCREEN_MARGIN).max(0.0);
        ui.set_max_width(max_width);
        ui.set_max_height(max_height);
        ui.add(egui::Label::new(egui::RichText::new(title).font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY)));
        ui.add_space(space::SCALE_8);
        // egui never refuses an overflowing body: it expands the max_rect to
        // keep min_rect inside it, which would drag the dialog past the clamp.
        // So the body runs in a scroll area, which on every axis reports
        // min(viewport, content): a small body still hugs its content, while a
        // body asking for more than the clamp scrolls inside the clamp.
        let body_max_height = (max_height - ui.min_rect().height() - space::SCALE_8 - FOOTER_MAX_HEIGHT).max(0.0);
        egui::ScrollArea::both().auto_shrink(true).max_width(max_width).max_height(body_max_height).show(ui, |ui| add_body(ui));
        ui.add_space(space::SCALE_8);
        // Right-aligned footer row, laid out left to right, so the primary
        // button added last sits at the bottom right (spec "Dialogs"). egui
        // has no right-align for rows, so the row is placed against the
        // right edge using its measured width from the previous frame (a
        // footer's width is stable while a dialog is open).
        let footer_id = id.with("footer");
        let row = ui.available_rect_before_wrap();
        let measured = ctx.data(|data| data.get_temp::<f32>(footer_id));
        let max_rect = match measured {
            Some(width) if width < row.width() => Rect::from_min_max(Pos2::new(row.right() - width, row.top()), row.max),
            _ => row,
        };
        let footer = ui.scope_builder(UiBuilder::new().id_salt(footer_id).max_rect(max_rect).layout(Layout::left_to_right(Align::Center)), add_footer);
        ctx.data_mut(|data| data.insert_temp(footer_id, footer.response.rect.width()));
    });
    if response.should_close() {
        *open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dialog whose body asks for 5000 px of width stays inside the screen:
    /// the dialog clamps itself to the screen rect minus the margin per side.
    #[test]
    fn a_body_asking_for_5000px_stays_inside_the_screen() {
        let ctx = egui::Context::default();
        crate::apply(&ctx);
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut open = true;
        let id = Id::new("clamped-dialog");
        let mut painted: Option<Rect> = None;
        for _ in 0..3 {
            let mut output = ctx.run_ui(raw.clone(), |ui| {
                dialog(
                    ui.ctx(),
                    id,
                    "Clamped",
                    &mut open,
                    |ui| {
                        ui.set_min_width(5000.0);
                        ui.label("A body that asks for far more width than the screen has.");
                    },
                    |_ui| {},
                );
            });
            // The dialog is what it painted: the modal centers itself on the
            // screen, so the painted geometry is the real verdict. The backdrop
            // is the one shape that covers (nearly) the whole screen; skip it.
            let shapes = std::mem::take(&mut output.shapes);
            let mut rects: Vec<Rect> = Vec::new();
            for clipped in &shapes {
                let rect = clipped.shape.visual_bounding_rect();
                let nearly = screen.shrink(8.0);
                let covers_screen = rect.min.x <= nearly.min.x && rect.min.y <= nearly.min.y && rect.max.x >= nearly.max.x && rect.max.y >= nearly.max.y;
                if covers_screen {
                    continue;
                }
                rects.push(rect);
            }
            painted = rects.iter().copied().reduce(Rect::union);
            // Headless: the font texture delta is never uploaded anywhere.
            output.textures_delta.clear();
        }
        let painted = painted.expect("the dialog painted nothing");
        assert!(
            painted.min.x >= -0.5 && painted.min.y >= -0.5 && painted.max.x <= screen.max.x + 0.5 && painted.max.y <= screen.max.y + 0.5,
            "the dialog painted outside the 1280 x 800 screen: {painted:?}"
        );
        assert!(painted.width() < 5000.0, "the dialog kept the body's 5000 px width: {painted:?}");
    }
}
