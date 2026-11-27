//! The About box (spec: S1 Help "About box" and Brand-Mark.md section 5): a
//! dialog with the app's wordmark lockup (or its name, for an app with no
//! brand art), its version and a scrollable "Third-party licenses" page.
//! Styling comes only from ui-kit tokens and the brand art.

use egui::{Context, Id, RichText, Ui};

use crate::Theme;
use crate::brand;
use crate::fonts;
use crate::tokens::{space, typography};

/// Height of the wordmark lockup at the top of the box.
const WORDMARK_HEIGHT: f32 = 32.0;
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
    /// The brand art whose wordmark heads the box; `None` shows the name as text.
    pub brand: Option<brand::App>,
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

/// The main view: the wordmark (or the name), the version and the copyright.
fn main_view(ui: &mut Ui, info: &AboutInfo) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    ui.vertical_centered(|ui| {
        match info.brand {
            Some(app) => {
                brand::wordmark(ui, app, WORDMARK_HEIGHT);
                ui.add_space(space::SCALE_8);
            }
            None => {
                ui.label(RichText::new(info.app_name).font(fonts::semibold(typography::TITLE_SIZE)).color(neutrals.text_primary));
            }
        }
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
