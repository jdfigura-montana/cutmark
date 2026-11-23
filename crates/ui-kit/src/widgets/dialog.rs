//! The dialog (spec "Components": row "Dialogs"): `bg.panel`, `radius.panel`,
//! 24 px padding, centered over a dimmed backdrop, the primary button at the
//! bottom right. Esc and a backdrop click close it.

use egui::{Align, Context, Id, Layout, Margin, Pos2, Rect, Stroke, Ui, UiBuilder};

use crate::HAIRLINE;
use crate::fonts;
use crate::tokens::{color, radius, space, typography};

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
        ui.add(egui::Label::new(egui::RichText::new(title).font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY)));
        ui.add_space(space::SCALE_8);
        add_body(ui);
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
