//! The Prompter's root window: the Library (spec "Library window").

use std::cell::Cell;

use prompter_platform::os::OsError;
use prompter_platform::view::{NativeWindow, ViewStatus, ViewWindow};
use ui_kit::tokens::{color, radius, size, space, typography};

use crate::menu::{Item, MENUS};
use crate::renderer::AlphaInUse;
use crate::view::{self, FloatingView, log_failure};

/// The Prompter's appearance identity: its settings folder name and default accent.
pub const APPEARANCE: ui_kit::AppAppearance = ui_kit::AppAppearance::new("prompter", ui_kit::Preset::Violet);

/// The root window's title, which is also how its window handle is found.
pub const ROOT_TITLE: &str = "Cutmark Prompter";

/// How many frames to look for the root window before giving up.
const ROOT_FIND_FRAMES: u8 = 30;

/// Side of the app mark in the title bar (rev 3 "Title bar").
const APP_MARK: f32 = 18.0;
/// Width of the Scripts panel.
const SCRIPTS_WIDTH: f32 = 260.0;
/// Width of the Run panel.
const RUN_WIDTH: f32 = 320.0;
/// A panel title's inset past the panel's own padding, for 14 in all (rev 3 "Panel header").
const HEADER_PAD: f32 = 6.0;
/// The hairline stroke width.
const HAIRLINE: f32 = 1.0;

const ABOUT: ui_kit::widgets::AboutInfo = ui_kit::widgets::AboutInfo {
    app_name: "Cutmark Prompter",
    version: env!("CARGO_PKG_VERSION"),
    copyright: "Copyright 2026 Cutmark. All rights reserved.",
    notices: "Inter and JetBrains Mono are licensed under the SIL Open Font License 1.1.\n\
              Phosphor Icons and the egui-phosphor crate are licensed under MIT or Apache-2.0.\n\
              egui, eframe, epaint and wgpu are licensed under MIT or Apache-2.0.",
};

/// The root window's state.
pub struct PrompterApp {
    /// False when the Windows 11 check failed: only the dialog is shown, then the app exits.
    supported: bool,
    alpha: AlphaInUse,
    view: Option<FloatingView>,
    /// The number of the last view opened. Numbers only go up, so no two windows share a title.
    last_view_number: u32,
    click_through: bool,
    /// The size of the monitor the root window opened on, taken to be the primary one.
    monitor: Option<egui::Vec2>,
    /// Frames left to find and protect the root window; 0 once done.
    root_find_frames: u8,
    appearance_open: bool,
    about_open: bool,
}

impl PrompterApp {
    /// Takes the result of the startup OS check.
    pub fn new(os_check: Result<(), OsError>, alpha: AlphaInUse) -> Self {
        if let Err(err) = &os_check {
            log::error!("unsupported system: {err}");
        }
        Self {
            supported: os_check.is_ok(),
            alpha,
            view: None,
            last_view_number: 0,
            click_through: false,
            monitor: None,
            root_find_frames: ROOT_FIND_FRAMES,
            appearance_open: false,
            about_open: false,
        }
    }

    /// Keeps the Library out of screen captures.
    fn protect_root(&mut self, ctx: &egui::Context) {
        if self.root_find_frames == 0 {
            return;
        }
        self.root_find_frames -= 1;
        if let Some(window) = NativeWindow::find_own_window(ROOT_TITLE) {
            log_failure("exclude_from_capture on the Library", window.exclude_from_capture());
            log::info!("Library excluded from capture: {}", window.status().excluded);
            self.root_find_frames = 0;
        } else if self.root_find_frames == 0 {
            log::warn!("the Library window was not found; it is not excluded from capture");
        } else {
            ctx.request_repaint();
        }
    }

    /// Does what a menu item or a Run panel button asks. Items not wired yet are disabled, so
    /// they never get here.
    fn run(&mut self, ctx: &egui::Context, item: Item) {
        match item {
            Item::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Item::ShowView if self.view.is_none() => {
                self.last_view_number += 1;
                self.view = Some(FloatingView::new(self.last_view_number, self.monitor.map(view::default_position)));
            }
            Item::HideView => self.view = None,
            Item::ClickThrough => self.click_through = !self.click_through,
            Item::Appearance => self.appearance_open = true,
            Item::About => self.about_open = true,
            _ => {}
        }
    }

    /// The title bar, the three panels and the status bar.
    fn library(&mut self, ui: &mut egui::Ui) {
        // Before the menu bar, which has to see the hotkey's key press.
        if click_through_hotkey(ui) {
            self.click_through = !self.click_through;
        }
        let picked = Cell::new(None);
        let bar = title_bar(ui, self.view.is_some(), self.click_through, &picked);
        if let Some(item) = picked.get() {
            self.run(ui.ctx(), item);
        }

        let mut rest = ui.max_rect();
        rest.min.y = bar.bottom();
        let (body, status) = rest.split_top_bottom_at_y(rest.bottom() - size::STATUS_BAR_HEIGHT);
        let [scripts, editor, run] = columns(body.shrink(space::SCALE_8));
        column(ui, scripts, |ui| {
            panel_header(ui, "Scripts");
            empty_state(ui, "No scripts yet");
        });
        column(ui, editor, |ui| {
            panel_toolbar(ui);
            empty_state(ui, "Scripts come in Phase 2");
        });
        column(ui, run, |ui| {
            panel_header(ui, "Run");
            self.view_controls(ui);
        });

        let capture = capture_label(self.view.as_ref().map(|view| view.status()));
        let mut items = vec!["Ready"];
        items.extend(capture);
        ui.scope_builder(egui::UiBuilder::new().max_rect(status.shrink2(egui::vec2(space::SCALE_8, 0.0))), |ui| {
            ui_kit::status_bar(ui, &items);
        });

        ui_kit::appearance_dialog(ui.ctx(), &mut self.appearance_open);
        ui_kit::about_dialog(ui.ctx(), &mut self.about_open, &ABOUT);
    }

    /// The Run panel's view controls and diagnostics line.
    fn view_controls(&mut self, ui: &mut egui::Ui) {
        let view_open = self.view.is_some();
        let mut picked = None;
        ui.add_space(space::SCALE_8);
        ui.horizontal_wrapped(|ui| {
            for (item, label) in [(Item::ShowView, "Show"), (Item::HideView, "Hide")] {
                if ui.add_enabled(item.enabled(view_open), egui::Button::new(label)).clicked() {
                    picked = Some(item);
                }
            }
            let button = egui::Button::new(Item::ClickThrough.label()).selected(self.click_through);
            if ui.add(button).on_hover_text(Item::ClickThrough.chord()).clicked() {
                picked = Some(Item::ClickThrough);
            }
        });
        if let Some(item) = picked {
            self.run(ui.ctx(), item);
        }
        ui.add_space(space::SCALE_8);
        let status = self.view.as_ref().map(|view| view.status());
        ui.label(diagnostics(self.alpha.get().map(|mode| format!("{mode:?}")), status));
    }
}

/// True on the frame Ctrl+Alt+C is pressed in the Library.
fn click_through_hotkey(ui: &egui::Ui) -> bool {
    ui.input_mut(|i| {
        // egui-winit reports Ctrl with C as a copy even when Alt is held, so no C key arrives.
        let pressed = i.modifiers.ctrl && i.modifiers.alt && i.events.iter().any(|event| matches!(event, egui::Event::Copy));
        if pressed {
            // The menu bar takes Alt released with no key press as a tap and moves the keyboard
            // to File, so the C key is put back.
            i.events.push(egui::Event::Key { key: egui::Key::C, physical_key: None, pressed: true, repeat: false, modifiers: i.modifiers });
        }
        pressed
    })
}

/// The 44px title bar (rev 3 "Title bar"). The toolbar adds the separator and the caption
/// buttons. A clicked menu item is reported through `picked`.
fn title_bar(ui: &mut egui::Ui, view_open: bool, click_through: bool, picked: &Cell<Option<Item>>) -> egui::Rect {
    ui_kit::main_toolbar(
        ui,
        |ui| {
            app_mark(ui);
            let mut menus: Vec<ui_kit::Menu<'_>> = MENUS
                .iter()
                .map(|&(title, groups)| {
                    ui_kit::Menu::new(title, move |ui| {
                        for (index, group) in groups.iter().enumerate() {
                            if index > 0 {
                                ui.separator();
                            }
                            for &item in *group {
                                let checked = item == Item::ClickThrough && click_through;
                                if ui.add_enabled(item.enabled(view_open), menu_button(item).selected(checked)).clicked() {
                                    picked.set(Some(item));
                                    ui.close();
                                }
                            }
                        }
                    })
                })
                .collect();
            ui_kit::menu_bar(ui, &mut menus);
        },
        |ui| {
            ui_kit::title_block(ui, &ui_kit::Title::App(ROOT_TITLE));
        },
        |ui| {
            ui.add_enabled_ui(false, |ui| ui_kit::primary_button(ui, "Start Prompter"));
        },
    )
    .bar
}

/// A menu row: the label, then the shortcut if the item has one.
fn menu_button(item: Item) -> egui::Button<'static> {
    let mut job = egui::text::LayoutJob::default();
    job.append(item.label(), 0.0, egui::TextFormat::simple(ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_PRIMARY));
    if !item.chord().is_empty() {
        job.append(item.chord(), space::SCALE_12, egui::TextFormat::simple(ui_kit::fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY));
    }
    egui::Button::new(job)
}

/// The app mark at the left of the title bar: a small rounded square in the accent color.
fn app_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(APP_MARK), egui::Sense::hover());
    ui.painter().rect_filled(rect, radius::SEGMENT, ui_kit::Theme::get(ui.ctx()).accent.accent);
}

/// Splits the body into Scripts, editor and Run, with an 8px gap between them.
fn columns(body: egui::Rect) -> [egui::Rect; 3] {
    let mut scripts = body;
    scripts.max.x = body.left() + SCRIPTS_WIDTH;
    let mut run = body;
    run.min.x = body.right() - RUN_WIDTH;
    let mut editor = body;
    editor.min.x = scripts.right() + space::SCALE_8;
    editor.max.x = run.left() - space::SCALE_8;
    [scripts, editor, run]
}

/// A panel that fills `rect`.
fn column(ui: &mut egui::Ui, rect: egui::Rect, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui_kit::panel(ui, |ui| {
            ui.set_min_size(ui.available_size());
            add_contents(ui);
        });
    });
}

/// A 44px row at the top of a panel, with a hairline along its bottom that runs edge to edge.
fn panel_row(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let row = egui::vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(row);
        add_contents(ui);
    });
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
}

/// A side panel's header row (rev 3 "Panel header").
fn panel_header(ui: &mut egui::Ui, title: &str) {
    panel_row(ui, |ui| {
        ui.add_space(HEADER_PAD);
        let text = egui::RichText::new(title).font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY);
        ui.add(egui::Label::new(text).selectable(false));
    });
}

/// The editor's toolbar (rev 3 "Panel toolbar"). Nothing can be undone until there is an editor.
fn panel_toolbar(ui: &mut egui::Ui) {
    panel_row(ui, |ui| {
        ui.add_enabled_ui(false, |ui| {
            ui_kit::icon_button(ui, ui_kit::icons::regular::ARROW_COUNTER_CLOCKWISE, "Undo");
            ui_kit::icon_button(ui, ui_kit::icons::regular::ARROW_CLOCKWISE, "Redo");
        });
    });
}

/// A caption in the middle of the space left in a panel.
fn empty_state(ui: &mut egui::Ui, caption: &str) {
    let text = egui::RichText::new(caption).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY);
    ui.centered_and_justified(|ui| ui.label(text));
}

/// The status bar's capture text, or `None` with no view open. `Some(None)` is an open view whose
/// window has not been found, so nothing excludes it yet.
fn capture_label(view: Option<Option<ViewStatus>>) -> Option<&'static str> {
    view.map(|status| if status.is_some_and(|status| status.excluded) { "Hidden from capture" } else { "Visible to capture" })
}

/// The diagnostics line. `view` is `None` with no view open, and `Some(None)` while the open
/// view's window has not been found.
fn diagnostics(alpha: Option<String>, view: Option<Option<ViewStatus>>) -> String {
    let yes_no = |on: bool| if on { "yes" } else { "no" };
    let alpha = alpha.unwrap_or_else(|| "unknown".to_owned());
    let view = match view {
        None => "hidden".to_owned(),
        Some(None) => "window not found".to_owned(),
        Some(Some(s)) => format!(
            "excluded {}, no-activate {}, topmost {}, backing clear {}",
            yes_no(s.excluded),
            yes_no(s.no_activate),
            yes_no(s.topmost),
            yes_no(s.backing_is_clear)
        ),
    };
    format!("Alpha mode: {alpha} | View: {view}")
}

impl eframe::App for PrompterApp {
    /// The root viewport is transparent only so that eframe picks a transparent alpha mode for
    /// the views; the Library itself stays opaque.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ui_kit::tokens::color::BG_GROUND.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.monitor.is_none() {
            self.monitor = ui.input(|i| i.viewport().monitor_size);
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| {
            if self.supported {
                self.library(ui);
            } else {
                unsupported_dialog(ui.ctx());
            }
        });
        if !self.supported {
            return;
        }
        self.protect_root(ui.ctx());
        if let Some(view) = &mut self.view
            && !view.show(ui.ctx(), self.click_through)
        {
            self.view = None;
        }
    }
}

/// The one dialog an unsupported system sees. OK, Esc and a backdrop click all exit.
fn unsupported_dialog(ctx: &egui::Context) {
    let mut open = true;
    let mut accepted = false;
    ui_kit::dialog(
        ctx,
        egui::Id::new("unsupported-os"),
        "Cutmark Prompter",
        &mut open,
        |ui| {
            ui.label("Cutmark Prompter needs Windows 11.");
        },
        |ui| accepted = ui_kit::primary_button(ui, "OK").clicked(),
    );
    if accepted || !open {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame with the hotkey check ahead of a menu bar, as the Library does.
    fn frame(ctx: &egui::Context, modifiers: egui::Modifiers, events: Vec<egui::Event>) -> (bool, Option<usize>) {
        let mut result = (false, None);
        let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput { events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            result.0 = click_through_hotkey(ui);
            result.1 = ui.horizontal(|ui| ui_kit::menu_bar(ui, &mut [ui_kit::Menu::new("File", |_ui| {})]).open).inner;
        });
        output.textures_delta.clear();
        result
    }

    #[test]
    fn the_click_through_hotkey_does_not_enter_the_menu_bar() {
        let ctx = egui::Context::default();
        ctx.set_fonts(ui_kit::fonts::definitions());
        let held = egui::Modifiers { ctrl: true, command: true, alt: true, ..Default::default() };
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&ctx, Default::default(), Vec::new());
        frame(&ctx, held, Vec::new());
        assert!(frame(&ctx, held, vec![egui::Event::Copy]).0);
        frame(&ctx, Default::default(), Vec::new());
        // Enter opens the first menu only if releasing Alt moved the keyboard to the bar.
        assert_eq!(frame(&ctx, Default::default(), vec![enter]).1, None);
    }

    #[test]
    fn the_status_bar_says_whether_the_view_is_hidden_from_capture() {
        let hidden = ViewStatus { excluded: true, ..Default::default() };
        assert_eq!(capture_label(None), None);
        assert_eq!(capture_label(Some(None)), Some("Visible to capture"));
        assert_eq!(capture_label(Some(Some(ViewStatus::default()))), Some("Visible to capture"));
        assert_eq!(capture_label(Some(Some(hidden))), Some("Hidden from capture"));
    }

    #[test]
    fn the_side_panels_keep_their_width_with_8px_gaps() {
        let body = egui::Rect::from_min_size(egui::pos2(8.0, 52.0), egui::vec2(1084.0, 634.0));
        let [scripts, editor, run] = columns(body);
        assert_eq!((scripts.width(), run.width()), (260.0, 320.0));
        assert_eq!((editor.left() - scripts.right(), run.left() - editor.right()), (8.0, 8.0));
        assert_eq!((scripts.left(), run.right()), (body.left(), body.right()));
    }
}
