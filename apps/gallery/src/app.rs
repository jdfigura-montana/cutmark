//! The gallery app shell: the main toolbar (which is the title bar), one
//! vertical scroll area per page, the status bar, the appearance dialog and
//! the toasts (spec: Design Language "Style gallery", "Main toolbar",
//! "Status bar").

use eframe::egui;
use egui_phosphor::variants::fill;
use ui_kit::fonts;
use ui_kit::tokens::{color, size, space, typography};
use ui_kit::{Theme, appearance, dialog};

use crate::core_page;
use crate::media;

/// The ui-kit version shown in the toolbar caption. Every suite package
/// shares the workspace version, so this tracks ui-kit's value.
const UI_KIT_VERSION: &str = "0.1.0";

/// The pages of the toolbar segmented control.
const PAGES: &[&str] = &["Core", "Media"];

/// The gallery app state.
pub struct Gallery {
    /// The selected page, an index into [`PAGES`].
    page: usize,
    /// The Core page's widget demo state.
    core: core_page::CoreState,
    /// The Media page's demo state.
    media: media::MediaState,
    /// True while the appearance dialog is open.
    appearance_open: bool,
}

impl Gallery {
    /// Fresh gallery: Core page, no dialogs open.
    pub fn new() -> Self {
        Self { page: 0, core: core_page::CoreState::new(), media: media::MediaState::new(), appearance_open: false }
    }

    /// True while any modal covers the page (pauses the focus rotation).
    fn modal_open(&self) -> bool {
        self.appearance_open || self.core.dialog_open.get()
    }
}

impl Default for Gallery {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for Gallery {
    /// Draws the whole gallery: toolbar, page, status bar, dialogs, toasts.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The appearance file feeds the theme; toasts draw on top of everything.
        appearance::poll(ui.ctx());
        ui_kit::show_toasts(ui.ctx());
        ui.painter().rect_filled(ui.max_rect(), 0.0, color::BG_GROUND);

        self.show_toolbar(ui);
        self.show_page(ui);
        self.show_status_bar(ui);
        self.show_dialogs(ui);
    }
}

impl Gallery {
    /// The main toolbar: app mark, document caption, page switch, actions.
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let page = std::cell::Cell::new(self.page);
        let appearance_open = std::cell::Cell::new(self.appearance_open);
        ui_kit::main_toolbar(
            ui,
            left_zone,
            |ui| {
                let mut selected = page.get();
                ui_kit::segmented(ui, &mut selected, PAGES);
                page.set(selected);
            },
            |ui| {
                ui.add_space(space::SCALE_6);
                if ui_kit::icon_button(ui, fill::PALETTE, "Appearance").clicked() {
                    appearance_open.set(true);
                }
                ui.add_space(space::SCALE_4);
                if ui_kit::primary_button(ui, "Toast").clicked() {
                    ui_kit::toast(ui.ctx(), "Toast from the gallery");
                }
            },
        );
        self.page = page.get();
        self.appearance_open = appearance_open.get();
    }

    /// The scroll area of the current page, with the window padding and the
    /// panel gap from the tokens.
    fn show_page(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        let gap = space::WINDOW_GAP as i8;
        let page = self.page;
        let modal_open = self.modal_open();
        let mut content = ui.new_child(egui::UiBuilder::new().max_rect(available));
        egui::ScrollArea::vertical().auto_shrink(false).id_salt(if page == 0 { "gallery-core" } else { "gallery-media" }).show(&mut content, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: gap, right: gap, top: gap, bottom: gap }).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = space::WINDOW_GAP;
                if page == 0 {
                    core_page::show(ui, &mut self.core, modal_open);
                } else {
                    media::show(ui, &mut self.media);
                }
            });
        });
    }

    /// The status bar pinned to the bottom of the window: the live accent
    /// hex and the glass and reduce motion switches.
    fn show_status_bar(&mut self, ui: &mut egui::Ui) {
        let theme = Theme::get(ui.ctx());
        let items = [
            format!("accent {}", core_page::hex(theme.accent.accent)),
            format!("glass {}", if theme.glass { "on" } else { "off" }),
            format!("reduce motion {}", if theme.reduce_motion { "on" } else { "off" }),
        ];
        let items: Vec<&str> = items.iter().map(String::as_str).collect();
        let available = ui.available_rect_before_wrap();
        let status_top = available.bottom() - size::STATUS_BAR_HEIGHT;
        let status_rect = egui::Rect::from_min_max(egui::pos2(available.left(), status_top), available.max);
        let mut status_ui = ui.new_child(egui::UiBuilder::new().max_rect(status_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
        ui_kit::status_bar(&mut status_ui, &items);
    }

    /// The sample dialog (opened from the Core page) and the appearance dialog.
    fn show_dialogs(&mut self, ui: &mut egui::Ui) {
        let mut open = self.core.dialog_open.get();
        dialog::dialog(
            ui.ctx(),
            egui::Id::new("gallery-sample-dialog"),
            "Sample dialog",
            &mut open,
            |ui| {
                ui.label(
                    egui::RichText::new("The dialog frame: bg.panel, radius.panel, 24 px padding and a dimmed backdrop.")
                        .font(fonts::medium(typography::BODY_SIZE))
                        .color(color::TEXT_SECONDARY),
                );
            },
            |ui| dialog_footer(ui, &self.core.dialog_open),
        );
        self.core.dialog_open.set(self.core.dialog_open.get() && open);
        ui_kit::appearance_dialog(ui.ctx(), &mut self.appearance_open);
    }
}

/// The left zone: the app mark placeholder and the document caption.
fn left_zone(ui: &mut egui::Ui) {
    app_mark(ui);
    ui.add_space(space::SCALE_4);
    ui.vertical(|ui| {
        ui.label(egui::RichText::new("Gallery").font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY));
        ui.label(egui::RichText::new(format!("ui-kit {UI_KIT_VERSION}")).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
    });
}

/// Draws the app mark placeholder: an accent circle with a notch cut from its
/// top right (spec "Open questions" item 1: placeholder until a real logo).
fn app_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    let c = rect.center();
    let painter = ui.painter();
    painter.circle_filled(c, 9.0, Theme::get(ui.ctx()).accent.accent);
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(c.x, c.y - 9.0), egui::vec2(9.0, 9.0)), 0.0, color::BG_GROUND);
}

/// The sample dialog's footer: Cancel, then the primary Close at the bottom
/// right. Both close through the shared open cell.
fn dialog_footer(ui: &mut egui::Ui, open: &std::cell::Cell<bool>) {
    if ui.button("Cancel").clicked() {
        open.set(false);
    }
    if ui_kit::primary_button(ui, "Close").clicked() {
        open.set(false);
    }
}
