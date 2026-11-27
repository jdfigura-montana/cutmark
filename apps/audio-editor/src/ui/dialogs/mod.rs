//! ui::dialogs: the modal dialogs and the few helpers they share.
//!
//! Every dialog is a ui-kit dialog (`bg.panel`, radius 18, 24 px padding, the
//! title in `type.headline`, the footer right-aligned with the primary action
//! last). Form rows are ui-kit inspector rows, and the controls are the ui-kit
//! segmented control, text field, slider, switch and primary button. The
//! helpers here only compose those widgets and read only ui-kit tokens.

pub mod audio_settings;
pub mod confirm;
pub mod export;
pub mod gain;
pub mod new_project;
pub mod normalize;
pub mod preferences;
pub mod project_settings;
pub mod save_as;

use eframe::egui;
use egui::{Align, Layout, RichText, Ui, UiBuilder, vec2};
use ui_kit::SliderKind;
use ui_kit::fonts;
use ui_kit::tokens::{color, size, space, typography};

/// Card width of the message and single-value dialogs (messages, Gain,
/// Normalize, the close prompts).
pub const NARROW_W: f32 = 420.0;
/// Card width of the dialogs with a form (New Project, Save As, Export,
/// Audio Settings, Project Settings).
pub const FORM_W: f32 = 500.0;
/// Width of the 1 px rim the ui-kit dialog draws around its card.
const RIM: f32 = 1.0;
/// Width of the typed-value box next to a dialog slider (three 24 px steps).
const VALUE_BOX_W: f32 = space::SCALE_24 * 3.0;

/// Shows a ui-kit dialog `width` wide (rim and padding included): `title`,
/// `add_body`, and the right-aligned `add_footer` row, whose last button is
/// the primary action. Esc and a backdrop click set `*open` to false.
pub fn dialog(ctx: &egui::Context, id: &str, title: &str, width: f32, open: &mut bool, add_body: impl FnOnce(&mut Ui), add_footer: impl FnOnce(&mut Ui)) {
    ui_kit::dialog(
        ctx,
        egui::Id::new(id),
        title,
        open,
        |ui| {
            ui.set_width(width - 2.0 * (space::SCALE_24 + RIM));
            add_body(ui);
            ui.add_space(space::SCALE_8);
        },
        add_footer,
    );
}

/// The one-line description under the dialog title, wrapped to the card.
pub fn description(ui: &mut Ui, text: &str) {
    let neutral = crate::ui::neutrals(ui.ctx());
    ui.label(RichText::new(text).font(fonts::medium(typography::BODY_SIZE)).color(neutral.text_secondary));
    ui.add_space(space::SCALE_12);
}

/// A description led by a warning icon in `sem.meter.warn`.
pub fn warning_description(ui: &mut Ui, text: &str) {
    let neutral = crate::ui::neutrals(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    job.append(ui_kit::icons::fill::WARNING, 0.0, egui::TextFormat::simple(fonts::icon_fill(typography::HEADLINE_SIZE), color::SEM_METER_WARN));
    job.append(text, space::SCALE_8, egui::TextFormat::simple(fonts::medium(typography::BODY_SIZE), neutral.text_secondary));
    ui.label(job);
    ui.add_space(space::SCALE_12);
}

/// Error text under the form, in `sem.record`.
pub fn error_text(ui: &mut Ui, text: &str) {
    ui.add_space(space::SCALE_8);
    ui.label(RichText::new(text).font(fonts::medium(typography::BODY_SIZE)).color(color::SEM_RECORD));
}

/// True when Enter went down this frame (the dialogs' confirm key, read from
/// the shortcut table). The press is left for whatever else reads it.
pub fn enter_pressed(ui: &Ui) -> bool {
    crate::shortcuts::peek(ui.ctx(), "dialog_confirm")
}

/// The form, wrapped in a vertical ScrollArea that only scrolls when the form
/// is taller than the space the card may take (85% of the window), so the
/// title and the footer always stay visible.
pub fn scroll_form(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let max_h = (0.85_f32 * ui.ctx().viewport_rect().height()).min(ui.available_height());
    egui::ScrollArea::vertical().id_salt("dialog_form").max_height(max_h).auto_shrink([false, true]).show(ui, add);
}

/// The ui-kit segmented control over typed `options`: shows the one equal to
/// `value` as selected and writes the chosen one back.
pub fn choice<T: PartialEq + Copy>(ui: &mut Ui, id_salt: &str, options: &[(T, &str)], value: &mut T) {
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    let before = options.iter().position(|(option, _)| option == value).unwrap_or(0);
    let mut index = before;
    ui_kit::segmented(ui, egui::Id::new(id_salt), &mut index, &labels);
    if index != before {
        *value = options[index].0;
    }
}

/// A footer button that is not the primary action: the ui-kit control fill.
pub fn secondary_button(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(label))
}

/// The ui-kit primary button, dimmed and inert when `enabled` is false.
pub fn primary_button(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| ui_kit::primary_button(ui, label)).inner
}

/// A row control made of a flexible part and a "Browse..." button flush with
/// the row's right edge, one item gap away from the flexible part.
///
/// `add` fills the width left of the button. Returns true when Browse was
/// clicked.
pub fn with_browse(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> bool {
    let row = ui.available_rect_before_wrap();
    // The button width comes from an invisible sizing pass, so it matches
    // what the real button allocates.
    let mut measure = ui.new_child(UiBuilder::new().invisible().sizing_pass());
    let button_w = secondary_button(&mut measure, "Browse...").rect.width();
    let gap = ui.spacing().item_spacing.x;
    let left = egui::Rect::from_min_max(row.min, egui::pos2((row.right() - button_w - gap).max(row.left()), row.bottom()));
    let right = egui::Rect::from_min_max(egui::pos2(row.right() - button_w, row.top()), row.max);
    add(&mut ui.new_child(UiBuilder::new().max_rect(left).layout(Layout::left_to_right(Align::Center))));
    let mut button = ui.new_child(UiBuilder::new().max_rect(right).layout(Layout::left_to_right(Align::Center)));
    secondary_button(&mut button, "Browse...").clicked()
}

/// The width the slider gets in a slider + typed value control: the row width
/// minus the value box and the gap between them.
fn slider_width(ui: &Ui) -> f32 {
    (ui.available_width() - VALUE_BOX_W - ui.spacing().item_spacing.x).max(0.0)
}

/// Places the ui-kit slider in its share of the row; returns its response.
fn row_slider(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> egui::Response) -> egui::Response {
    let width = slider_width(ui);
    ui.allocate_ui_with_layout(vec2(width, size::SEGMENT_HEIGHT), Layout::left_to_right(Align::Center), add).inner
}

/// The typed value next to a slider: a drag value in the mono value type.
fn value_box(ui: &mut Ui, drag: egui::DragValue<'_>) {
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(fonts::mono(typography::VALUE_SIZE));
        ui.add_sized([VALUE_BOX_W, size::TOOLBAR_BUTTON], drag);
    });
}

/// A dB control: the ui-kit slider plus a typed value in the mono value type.
///
/// The slider fills the row width minus the value box and the gap between
/// them. Double-click on the slider resets to 0.0.
pub fn db_control(ui: &mut Ui, value: &mut f32, range: (f32, f32)) {
    let slider = row_slider(ui, |ui| ui_kit::slider(ui, value, range.0..=range.1, SliderKind::Accent));
    if slider.double_clicked() {
        *value = 0.0;
    }
    value_box(ui, egui::DragValue::new(value).range(range.0..=range.1).speed(0.1).suffix(" dB"));
}

/// A whole-millisecond control: the same slider + typed value layout as
/// [`db_control`], with a signed mono value ("+0 ms"). Double-click on the
/// slider resets to 0.
pub fn ms_control(ui: &mut Ui, value: &mut i32, range: (i32, i32)) {
    let mut shown = *value as f32;
    let slider = row_slider(ui, |ui| ui_kit::slider(ui, &mut shown, range.0 as f32..=range.1 as f32, SliderKind::Accent));
    *value = (shown.round() as i32).clamp(range.0, range.1);
    if slider.double_clicked() {
        *value = 0;
    }
    value_box(ui, egui::DragValue::new(value).range(range.0..=range.1).speed(0.5).custom_formatter(|n, _| format!("{:+}", n.round() as i64)).suffix(" ms"));
}
