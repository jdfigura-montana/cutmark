//! The About box (spec: S1 Help "Open questions" item 1: a simple About with
//! the app mark, the version and a licenses view): a dialog with the app's
//! name and version and a scrollable "Third-party licenses" page.
//! Styling comes only from ui-kit tokens.

use egui::{Context, Id, RichText, Ui, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{space, typography};

/// Side of the app mark drawn at the top of the box.
const MARK_SIZE: f32 = 48.0;
/// Height of the scrollable licenses view.
const LICENSES_HEIGHT: f32 = 220.0;

/// The dialog's egui id.
fn dialog_id() -> Id {
    Id::new("ui_kit_about")
}

/// The egui context data id holding the licenses view's open state.
fn view_id() -> Id {
    dialog_id().with("licenses")
}

/// What the About box shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AboutInfo {
    /// The app's name.
    pub app_name: &'static str,
    /// The app's version.
    pub version: &'static str,
    /// The copyright line.
    pub copyright: &'static str,
    /// The third-party notices text, shown in the licenses view.
    pub notices: &'static str,
}

/// Draws the About box while `open` is set, and closes it on Esc, a backdrop
/// click or the Close button.
pub fn about_dialog(ctx: &Context, open: &mut bool, info: &AboutInfo) {
    if !*open {
        return;
    }
    let mut dialog_open = true;
    let close = std::cell::Cell::new(false);
    let title = format!("About {}", info.app_name);
    crate::dialog(
        ctx,
        dialog_id(),
        &title,
        &mut dialog_open,
        |ui| body(ui, info),
        |ui| {
            if crate::primary_button(ui, "Close").clicked() {
                close.set(true);
            }
        },
    );
    if close.get() || !dialog_open {
        *open = false;
    }
}

/// The body: the main view or the licenses view, whichever is open.
fn body(ui: &mut Ui, info: &AboutInfo) {
    let licenses = ui.ctx().data(|data| data.get_temp::<bool>(view_id()).unwrap_or(false));
    if licenses {
        licenses_view(ui, info);
    } else {
        main_view(ui, info);
    }
}

/// The main view: the app mark, the name, the version and the copyright.
fn main_view(ui: &mut Ui, info: &AboutInfo) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    ui.vertical_centered(|ui| {
        app_mark(ui);
        ui.add_space(space::SCALE_8);
        ui.label(RichText::new(info.app_name).font(fonts::semibold(typography::TITLE_SIZE)).color(neutrals.text_primary));
        ui.add_space(space::SCALE_2);
        ui.label(RichText::new(format!("Version {}", info.version)).font(fonts::mono(typography::VALUE_SIZE)).color(neutrals.text_tertiary));
        ui.add_space(space::SCALE_2);
        ui.label(RichText::new(info.copyright).font(fonts::medium(typography::CAPTION_SIZE)).color(neutrals.text_tertiary));
    });
    ui.add_space(space::SCALE_12);
    if ui.button("Third-party licenses").clicked() {
        ui.ctx().data_mut(|data| data.insert_temp(view_id(), true));
    }
}

/// The licenses view: the notices in `type.value`, scrollable, and a Back
/// button that returns to the main view.
fn licenses_view(ui: &mut Ui, info: &AboutInfo) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    ui.label(RichText::new("Third-party licenses").font(fonts::semibold(typography::HEADLINE_SIZE)).color(neutrals.text_primary));
    ui.add_space(space::SCALE_8);
    egui::ScrollArea::vertical().max_height(LICENSES_HEIGHT).id_salt("ui_kit_about_licenses").show(ui, |ui| {
        ui.label(RichText::new(info.notices).font(fonts::mono(typography::VALUE_SIZE)).color(neutrals.text_secondary));
    });
    ui.add_space(space::SCALE_8);
    if ui.button("Back").clicked() {
        ui.ctx().data_mut(|data| data.insert_temp(view_id(), false));
    }
}

/// The app mark placeholder (spec "Open questions" item 1: a placeholder until
/// a real logo): an accent circle with a notch cut from its top right.
fn app_mark(ui: &mut Ui) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let (rect, _) = ui.allocate_exact_size(vec2(MARK_SIZE, MARK_SIZE), egui::Sense::hover());
    let center = rect.center();
    let radius = MARK_SIZE / 2.0;
    let painter = ui.painter();
    painter.circle_filled(center, radius, Theme::get(ui.ctx()).accent.accent);
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(center.x, center.y - radius), vec2(radius, radius)), 0.0, neutrals.bg_panel);
}
