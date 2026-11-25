//! The gallery app shell: the main toolbar (which is the title bar), one
//! vertical scroll area per page, the status bar, the appearance dialog, the
//! toasts, the generated Keyboard shortcuts page, the demo tour and the About
//! box (spec: Design Language "Style gallery", "Main toolbar", "Status bar";
//! S1 Help "Shortcut sheet", "First-run tour").

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use ui_kit::help::{self, HelpBook, HelpState};
use ui_kit::icons::regular;
use ui_kit::shortcuts::{Shortcut, ShortcutTable};
use ui_kit::tokens::{color, size, space, typography};
use ui_kit::tour::{self, Tour, TourState, TourStep};
use ui_kit::{AboutInfo, Menu, Theme, Title, appearance, dialog, fonts};

use crate::core_page;
use crate::media;

/// The ui-kit version shown in the toolbar caption. Every suite package
/// shares the workspace version, so this tracks ui-kit's value.
const UI_KIT_VERSION: &str = "0.1.0";
/// The gallery's own appearance file folder and default accent.
pub const APPEARANCE: ui_kit::AppAppearance = ui_kit::AppAppearance::new("gallery", ui_kit::Preset::Blue);

/// The app mark's side (spec rev 3 "Title bar": 18).
const APP_MARK_SIZE: f32 = 18.0;

/// The pages of the toolbar segmented control.
const PAGES: &[&str] = &["Core", "Media"];

/// The Media page's index: the page where all three tour targets exist.
const MEDIA_PAGE: usize = 1;

/// The demo tour's steps: the toolbar, a panel and the transport deck.
const TOUR_STEPS: [TourStep; 3] = [
    TourStep { target: "toolbar", title: "The title bar", body: "The menus, the page switch and the actions all live here." },
    TourStep { target: "panel", title: "A panel", body: "Each panel groups the widgets for one job." },
    TourStep { target: "transport", title: "The transport deck", body: "Play, stop and record from the deck over the timeline." },
];

/// The gallery's demo tour. Its seen flag marks it shown once (spec: S1 Help
/// "First-run tour").
const TOUR: Tour = Tour { app: "gallery", id: "gallery-tour", version: 1, steps: &TOUR_STEPS };

/// The About box's content.
const ABOUT: AboutInfo = AboutInfo {
    app_name: "Cutmark Gallery",
    version: env!("CARGO_PKG_VERSION"),
    copyright: "Copyright 2026 Cutmark. All rights reserved.",
    notices: "Inter and JetBrains Mono are licensed under the SIL Open Font License 1.1.\n\
              Phosphor Icons and the egui-phosphor crate are licensed under MIT or Apache-2.0.\n\
              egui, eframe and epaint are licensed under MIT or Apache-2.0.\n\n\
              The font license files ship with the app.",
};

/// A shortcut with one key, for the sample table.
fn one_key(modifiers: Modifiers, key: Key) -> Vec<KeyboardShortcut> {
    vec![KeyboardShortcut::new(modifiers, key)]
}

/// The gallery's sample shortcuts: 10 entries in 3 groups, so the generated
/// keyboard page has content to show (spec: S1 Help "Shortcut sheet"). Four of
/// them drive the gallery itself (Show toast, Help topics, Keyboard shortcuts,
/// Show the tour); the rest are sample rows.
fn sample_shortcuts() -> ShortcutTable {
    ShortcutTable::new(vec![
        Shortcut { id: "file.new", label: "New project", group: "File", keys: one_key(Modifiers::COMMAND, Key::N), context: "global" },
        Shortcut { id: "file.open", label: "Open project", group: "File", keys: one_key(Modifiers::COMMAND, Key::O), context: "global" },
        Shortcut { id: "file.save", label: "Save project", group: "File", keys: one_key(Modifiers::COMMAND, Key::S), context: "global" },
        Shortcut { id: "file.toast", label: "Show toast", group: "File", keys: one_key(Modifiers::COMMAND, Key::T), context: "global" },
        Shortcut { id: "edit.undo", label: "Undo", group: "Edit", keys: one_key(Modifiers::COMMAND, Key::Z), context: "global" },
        Shortcut { id: "edit.redo", label: "Redo", group: "Edit", keys: one_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z), context: "global" },
        Shortcut { id: "edit.paste", label: "Paste", group: "Edit", keys: one_key(Modifiers::COMMAND, Key::V), context: "global" },
        Shortcut { id: "help.topics", label: "Help topics", group: "Help", keys: one_key(Modifiers::NONE, Key::F1), context: "global" },
        Shortcut { id: "help.keyboard", label: "Keyboard shortcuts", group: "Help", keys: one_key(Modifiers::COMMAND, Key::Slash), context: "global" },
        Shortcut { id: "help.tour", label: "Show the tour", group: "Help", keys: one_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::T), context: "global" },
    ])
}

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
    /// True while the About box is open.
    about_open: bool,
    /// The sample shortcuts: one table feeds the input handling, the menus and
    /// the generated keyboard page.
    shortcuts: ShortcutTable,
    /// The demo tour's running state.
    tour: TourState,
    /// The gallery's help topics, embedded at build time.
    book: HelpBook,
    /// The help window's open state and Back and Forward history.
    help: HelpState,
}

impl Gallery {
    /// Fresh gallery: Core page, no dialogs open, help closed, and the demo
    /// tour started on a fresh profile (spec: S1 Help "First-run tour").
    pub fn new(book: HelpBook) -> Self {
        let mut tour = TourState::default();
        if !tour::help_store::is_seen(TOUR.app, TOUR.id, TOUR.version) {
            tour.start();
        }
        Self {
            page: 0,
            core: core_page::CoreState::new(),
            media: media::MediaState::new(),
            appearance_open: false,
            about_open: false,
            shortcuts: sample_shortcuts(),
            tour,
            book,
            help: HelpState::default(),
        }
    }

    /// True while any modal covers the page (pauses the focus rotation).
    fn modal_open(&self) -> bool {
        self.appearance_open || self.about_open || self.core.dialog_open.get()
    }
}

impl eframe::App for Gallery {
    /// Draws the whole gallery: toolbar, page, status bar, dialogs, toasts.
    /// The toolbar is the top panel, the status bar the bottom panel, so the
    /// page's scroll area ends above the status bar and nothing overlaps.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // The appearance file feeds the theme; toasts draw on top of everything.
        appearance::poll(&ctx);
        ui_kit::show_toasts(&ctx);
        ui.painter().rect_filled(ui.max_rect(), 0.0, color::BG_GROUND);

        // The shortcuts act before anything draws, so a shortcut that opens a
        // window shows it this frame.
        self.handle_shortcuts(&ctx);
        self.show_toolbar(ui);
        self.show_status_bar(ui);
        self.show_page(ui);
        self.show_dialogs(ui);
        // F1 reads the tags the pages registered this frame, then the window
        // draws on top of everything.
        help::handle_f1(&ctx, &mut self.help, &self.book);
        self.show_help(&ctx);
        self.show_about(&ctx);
        // The tour's anchors were registered while the page drew, so it draws
        // last and over everything else.
        self.show_tour(&ctx);
    }
}

impl Gallery {
    /// Runs the sample shortcuts the gallery acts on. The more specific key is
    /// checked first, since egui's shortcut matching ignores extra modifiers.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.shortcuts.pressed(ctx, "help.tour") {
            self.tour.start();
        }
        if self.shortcuts.pressed(ctx, "file.toast") {
            ui_kit::toast(ctx, "Toast from a shortcut");
        }
        if self.shortcuts.pressed(ctx, "help.keyboard") {
            help::open(&mut self.help, help::KEYBOARD_PAGE_ID);
        }
    }

    /// Draws the help window, routing the generated Keyboard shortcuts page
    /// through the help window's extra callback.
    fn show_help(&mut self, ctx: &egui::Context) {
        let shortcuts = &self.shortcuts;
        let book = &self.book;
        let help_state = &mut self.help;
        let filter_id = egui::Id::new("gallery-shortcut-filter");
        help::help_window(ctx, help_state, book, &mut |id: &str, ui: &mut egui::Ui| {
            if id != help::KEYBOARD_PAGE_ID {
                return false;
            }
            let mut filter = ui.ctx().data_mut(|data| data.get_temp::<String>(filter_id).unwrap_or_default());
            ui_kit::shortcuts::shortcut_sheet(ui, shortcuts, &mut filter);
            ui.ctx().data_mut(|data| data.insert_temp(filter_id, filter));
            true
        });
    }

    /// Draws the About box.
    fn show_about(&mut self, ctx: &egui::Context) {
        ui_kit::about_dialog(ctx, &mut self.about_open, &ABOUT);
    }

    /// Draws the demo tour, and marks it seen when it finishes or is skipped.
    fn show_tour(&mut self, ctx: &egui::Context) {
        if tour::show_tour(ctx, &mut self.tour, &TOUR).is_some() {
            let _ = tour::help_store::mark_seen(TOUR.app, TOUR.id, TOUR.version);
        }
    }
}

impl Gallery {
    /// The main toolbar, which is the 44px title bar: app mark and the File and
    /// View menus on the left, the document title in the center, the page
    /// switch and the actions on the right.
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let page = std::cell::Cell::new(self.page);
        let appearance_open = std::cell::Cell::new(self.appearance_open);
        let open_help = std::cell::Cell::new(false);
        let open_keyboard = std::cell::Cell::new(false);
        let show_tour = std::cell::Cell::new(false);
        let open_about = std::cell::Cell::new(false);
        let shortcuts = &self.shortcuts;
        egui::Panel::top("gallery-toolbar").exact_size(size::TOOLBAR_MAIN_HEIGHT).frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| {
            ui_kit::main_toolbar(
                ui,
                |ui| left_zone(ui, &page, &appearance_open, &open_help, &open_keyboard, &show_tour, &open_about, shortcuts),
                |ui| {
                    let caption = format!("ui-kit {UI_KIT_VERSION}");
                    ui_kit::title_block(ui, &Title::Document { name: "Gallery", unsaved: false, caption: Some(&caption) });
                },
                |ui| {
                    let mut selected = page.get();
                    ui_kit::segmented(ui, "gallery-pages", &mut selected, PAGES);
                    page.set(selected);
                    ui.add_space(space::SCALE_6);
                    if ui_kit::icon_button(ui, regular::PALETTE, "Appearance").clicked() {
                        appearance_open.set(true);
                    }
                    ui.add_space(space::SCALE_4);
                    if ui_kit::primary_button(ui, "Toast").clicked() {
                        ui_kit::toast(ui.ctx(), "Toast from the gallery");
                    }
                },
            );
            // The whole bar is the tour's "toolbar" target.
            tour::anchor(ui, "toolbar", ui.max_rect());
        });
        self.page = page.get();
        self.appearance_open = appearance_open.get();
        if open_help.get() {
            self.help.open = true;
        }
        if open_keyboard.get() {
            help::open(&mut self.help, help::KEYBOARD_PAGE_ID);
        }
        if show_tour.get() {
            // Switch to the Media page, where the tour's three targets exist.
            self.page = MEDIA_PAGE;
            self.tour.start();
        }
        if open_about.get() {
            self.about_open = true;
        }
    }

    /// The scroll area of the current page, with the window padding and the
    /// panel gap from the tokens. It is the central panel: it ends above the
    /// status bar panel and sits inside the 8px window padding.
    fn show_page(&mut self, ui: &mut egui::Ui) {
        let gap = space::WINDOW_GAP as i8;
        let page = self.page;
        let modal_open = self.modal_open();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).id_salt(if page == 0 { "gallery-core" } else { "gallery-media" }).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin { left: gap, right: gap, top: gap, bottom: gap }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = space::WINDOW_GAP;
                    if page == 0 {
                        core_page::show(ui, &mut self.core, modal_open);
                    } else {
                        media::show(ui, &mut self.media);
                    }
                });
            });
        });
    }

    /// The status bar pinned to the bottom of the window: the live accent
    /// hex and the reduce motion switch. It is the bottom panel,
    /// so the page's scroll area ends above it.
    fn show_status_bar(&mut self, ui: &mut egui::Ui) {
        let theme = Theme::get(ui.ctx());
        let items = [format!("accent {}", core_page::hex(theme.accent.accent)), format!("reduce motion {}", if theme.reduce_motion { "on" } else { "off" })];
        let items: Vec<&str> = items.iter().map(String::as_str).collect();
        egui::Panel::bottom("gallery-status-bar").exact_size(size::STATUS_BAR_HEIGHT).frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| {
            ui_kit::status_bar(ui, &items);
        });
    }

    /// The sample dialog (opened from the Core page), the appearance dialog
    /// and the About box.
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
        ui_kit::about_dialog(ui.ctx(), &mut self.about_open, &ABOUT);
    }
}

/// The left zone: the app mark placeholder, then the menu bar with File, View
/// and Help menus of sample items. The shortcut text comes from the one sample
/// table, so the menus and the generated page can never drift apart.
#[allow(clippy::too_many_arguments)]
fn left_zone(
    ui: &mut egui::Ui,
    page: &std::cell::Cell<usize>,
    appearance_open: &std::cell::Cell<bool>,
    open_help: &std::cell::Cell<bool>,
    open_keyboard: &std::cell::Cell<bool>,
    show_tour: &std::cell::Cell<bool>,
    open_about: &std::cell::Cell<bool>,
    shortcuts: &ShortcutTable,
) {
    app_mark(ui);
    ui.add_space(space::SCALE_8);
    let toast_key = shortcuts.text(ui.ctx(), "file.toast");
    let keyboard_key = shortcuts.text(ui.ctx(), "help.keyboard");
    let tour_key = shortcuts.text(ui.ctx(), "help.tour");
    let mut menus = [
        Menu::new("File", |ui| {
            if ui.add(egui::Button::new("Show toast").shortcut_text(toast_key.as_str())).clicked() {
                ui_kit::toast(ui.ctx(), "Toast from the File menu");
                ui.close();
            }
            if ui.button("Appearance...").clicked() {
                appearance_open.set(true);
                ui.close();
            }
            ui.separator();
            if ui.add(egui::Button::new("Quit").shortcut_text("Alt+F4")).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }),
        Menu::new("View", |ui| {
            for (index, name) in PAGES.iter().enumerate() {
                if ui.selectable_label(page.get() == index, format!("{name} page")).clicked() {
                    page.set(index);
                    ui.close();
                }
            }
        }),
        Menu::new("Help", |ui| {
            if ui.add(egui::Button::new("Help topics").shortcut_text("F1")).clicked() {
                open_help.set(true);
                ui.close();
            }
            if ui.add(egui::Button::new("Keyboard shortcuts").shortcut_text(keyboard_key.as_str())).clicked() {
                open_keyboard.set(true);
                ui.close();
            }
            if ui.add(egui::Button::new("Show the tour").shortcut_text(tour_key.as_str())).clicked() {
                show_tour.set(true);
                ui.close();
            }
            ui.separator();
            if ui.button("About").clicked() {
                open_about.set(true);
                ui.close();
            }
        }),
    ];
    ui_kit::menu_bar(ui, &mut menus);
}

/// Draws the app mark placeholder: an accent circle with a notch cut from its
/// top right (spec "Open questions" item 1: placeholder until a real logo).
fn app_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(APP_MARK_SIZE, APP_MARK_SIZE), egui::Sense::hover());
    let c = rect.center();
    let radius = APP_MARK_SIZE / 2.0;
    let painter = ui.painter();
    painter.circle_filled(c, radius, Theme::get(ui.ctx()).accent.accent);
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(c.x, c.y - radius), egui::vec2(radius, radius)), 0.0, color::BG_GROUND);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The gallery saves its accent in its own folder and resets to Blue.
    #[test]
    fn the_gallery_has_its_own_appearance_identity() {
        assert_eq!(APPEARANCE.id, "gallery");
        assert_eq!(APPEARANCE.default_preset, ui_kit::Preset::Blue);
    }
}
