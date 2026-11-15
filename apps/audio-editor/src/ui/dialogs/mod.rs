//! ui::dialogs: modal dialogs and the shared card / field / footer builders.

pub mod audio_settings;
pub mod confirm;
pub mod export;
pub mod gain;
pub mod new_project;
pub mod normalize;
pub mod project_settings;
pub mod save_as;

use eframe::egui;
use eframe::egui::{Align, Align2, Layout, RichText, Sense, Stroke, StrokeKind, UiBuilder};

use crate::ui::theme::{THEME, Theme};

/// Builds the app modal: black 55% backdrop and the dialog card (panel_bg
/// fill, 1 px control_border, radius 10, soft shadow, 20 px padding).
///
/// Call [`card_width`] at the top of the content to set the card width.
pub fn modal(id: &str) -> egui::Modal {
    egui::Modal::new(egui::Id::new(id))
        .backdrop_color(THEME.modal_shade)
        .frame(card_frame())
}

/// The dialog card frame used by every modal.
fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(THEME.panel_bg)
        .stroke(Stroke::new(THEME.hairline, THEME.control_border))
        .corner_radius(THEME.card_radius)
        .shadow(THEME.card_shadow)
        .inner_margin(egui::Margin::same(THEME.card_pad as i8))
}

/// Sets the content width of a card shown with [`modal`] so the whole card,
/// padding and 1 px outline included, is exactly `width` wide.
pub fn card_width(ui: &mut egui::Ui, width: f32) {
    ui.set_width(width - 2.0 * (THEME.card_pad + THEME.hairline));
}

/// The dialog title: PlexSemiBold 15 px in text_primary.
pub fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(Theme::semibold(THEME.dialog_title_size))
            .color(THEME.text_primary)
            .size(THEME.dialog_title_size),
    );
}

/// The one-line description under the title: 12 px text_muted.
pub fn description(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .color(THEME.text_muted)
            .size(THEME.dialog_desc_size),
    );
}

/// The 16 px gap between title/description and the form.
pub fn before_form(ui: &mut egui::Ui) {
    ui.add_space(THEME.space_lg);
}

/// One form row: a 140 px label column (12.5 px text_normal) and the control
/// filling the rest, 10 px below the previous row. Fixed 30 px row height.
pub fn form_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    let row_w = ui.available_width();
    let (row, _) = ui.allocate_exact_size(egui::vec2(row_w, THEME.field_h), Sense::hover());
    // Label: left-aligned in a fixed 140 px column, vertically centered.
    // Painted directly so the column cannot stretch or reflow the row.
    ui.painter().text(
        egui::pos2(row.left(), row.center().y),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(THEME.dialog_label_size),
        THEME.text_normal,
    );
    // The control fills the remaining width exactly; the bounded max rect
    // keeps the row 30 px tall (a Center cross-aligned horizontal layout
    // otherwise fills the whole available height of the modal).
    let control = egui::Rect::from_min_max(
        egui::pos2(row.left() + THEME.form_label_w, row.top()),
        egui::pos2(row.right(), row.bottom()),
    );
    add(&mut ui.new_child(
        UiBuilder::new()
            .max_rect(control)
            .layout(Layout::left_to_right(Align::Center)),
    ));
    ui.add_space(THEME.form_row_gap);
}

/// A bare form row (no label column), e.g. for a checkbox. Fixed 30 px row
/// height, control vertically centered, 10 px below the previous row.
pub fn bare_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let row_w = ui.available_width();
    let (row, _) = ui.allocate_exact_size(egui::vec2(row_w, THEME.field_h), Sense::hover());
    add(&mut ui.new_child(
        UiBuilder::new()
            .max_rect(row)
            .layout(Layout::left_to_right(Align::Center)),
    ));
    ui.add_space(THEME.form_row_gap);
}

/// The dialog form, wrapped in a vertical ScrollArea that only scrolls when
/// the form is taller than the space the card may take (85% of the window),
/// so the title and the footer always stay visible.
pub fn form_scroll(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let max_h = (0.85_f32 * ui.ctx().viewport_rect().height()).min(ui.available_height());
    egui::ScrollArea::vertical()
        .id_salt("dialog_form")
        .max_height(max_h)
        .auto_shrink([false, true])
        .show(ui, add);
}

/// A dialog text field: recessed_a fill, 1 px control_border, radius 6,
/// height 30; the focused border is the accent. Returns the response.
pub fn text_field(ui: &mut egui::Ui, text: &mut String, id_salt: &str) -> egui::Response {
    let width = ui.available_width();
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.widgets.active.bg_stroke = Stroke::new(THEME.hairline, THEME.accent);
        v.widgets.active.bg_fill = THEME.recessed_a;
        v.widgets.active.weak_bg_fill = THEME.recessed_a;
        v.widgets.inactive.bg_stroke = Stroke::new(THEME.hairline, THEME.control_border);
        ui.add_sized(
            [width, THEME.field_h],
            egui::TextEdit::singleline(text).id_salt(id_salt),
        )
    })
    .inner
}
/// A segmented control: one bordered group of `options`; the selected
/// segment is filled accent_soft with an accent border. Returns true when
/// the selection changed this frame.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id_salt: &str,
    options: &[(T, &str)],
    value: &mut T,
) -> bool {
    let all_enabled: Vec<(T, &str, bool)> = options
        .iter()
        .map(|&(option, label)| (option, label, true))
        .collect();
    segmented_enabled(ui, id_salt, &all_enabled, value)
}

/// [`segmented`] where each option also says whether it can be chosen. A
/// disabled segment is drawn in `text_faint`, has no hover and ignores clicks.
pub fn segmented_enabled<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id_salt: &str,
    options: &[(T, &str, bool)],
    value: &mut T,
) -> bool {
    let mut changed = false;
    ui.scope(|ui| {
        // Segment widths: the label plus `segment_pad` of padding. When the
        // labels would not fit the row, the padding shrinks (down to
        // `segment_pad_min`) so the group never runs past the row's edge.
        let texts: Vec<f32> = options
            .iter()
            .map(|(_, label, _)| {
                ui.painter()
                    .layout_no_wrap(
                        label.to_string(),
                        egui::FontId::proportional(THEME.dialog_label_size),
                        THEME.text_normal,
                    )
                    .size()
                    .x
            })
            .collect();
        let text_total: f32 = texts.iter().sum();
        let room = (ui.available_width() - text_total) / texts.len().max(1) as f32;
        let pad = room.clamp(THEME.segment_pad_min, THEME.segment_pad);
        let sizes: Vec<f32> = texts.iter().map(|text| text + pad).collect();
        let group_w: f32 = sizes.iter().sum();
        let (group, _) = ui.allocate_exact_size(egui::vec2(group_w, THEME.field_h), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(group, 6.0, THEME.control_fill);
        painter.rect_stroke(
            group,
            6.0,
            Stroke::new(THEME.hairline, THEME.control_border),
            StrokeKind::Inside,
        );
        let mut x = group.left();
        for (i, (opt, label, enabled)) in options.iter().enumerate() {
            let seg = egui::Rect::from_min_max(
                egui::pos2(x, group.top()),
                egui::pos2(x + sizes[i], group.bottom()),
            );
            let sense = if *enabled {
                Sense::click()
            } else {
                Sense::hover()
            };
            let resp = ui.interact(seg, egui::Id::new((id_salt, i)), sense);
            if *enabled && resp.clicked() && *value != *opt {
                *value = *opt;
                changed = true;
            }
            let selected = *value == *opt;
            if selected {
                painter.rect_filled(seg.shrink2(egui::vec2(2.0, 2.0)), 4.0, THEME.accent_soft);
                painter.rect_stroke(
                    seg.shrink2(egui::vec2(2.0, 2.0)),
                    4.0,
                    Stroke::new(THEME.hairline, THEME.accent),
                    StrokeKind::Inside,
                );
            } else if *enabled && resp.hovered() {
                painter.rect_filled(seg.shrink2(egui::vec2(2.0, 2.0)), 4.0, THEME.control_hover);
            }
            painter.text(
                seg.center(),
                egui::Align2::CENTER_CENTER,
                *label,
                egui::FontId::proportional(THEME.dialog_label_size),
                if !*enabled {
                    THEME.text_faint
                } else if selected {
                    THEME.text_primary
                } else {
                    THEME.text_normal
                },
            );
            x += sizes[i];
        }
    });
    changed
}

/// The role of a footer button.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FooterButton {
    /// Accent fill, white text, lighter on hover.
    Primary,
    /// Control fill, normal text.
    Cancel,
    /// Control fill, destructive red text.
    Destructive,
}

/// A footer button: height 30, radius 6. The 8 px gap to a neighbouring
/// button is the layout's item spacing; nothing trails the button, so a row
/// of buttons ends exactly at its right edge.
pub fn footer_button(
    ui: &mut egui::Ui,
    label: &str,
    kind: FooterButton,
    enabled: bool,
) -> egui::Response {
    let (fill, hover, text) = match kind {
        FooterButton::Primary => (THEME.accent, THEME.accent_hover, egui::Color32::WHITE),
        FooterButton::Cancel => (THEME.control_fill, THEME.control_hover, THEME.text_normal),
        FooterButton::Destructive => (THEME.control_fill, THEME.control_hover, THEME.destructive),
    };
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.widgets.inactive.weak_bg_fill = fill;
        v.widgets.inactive.bg_fill = fill;
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.hovered.weak_bg_fill = hover;
        v.widgets.hovered.bg_fill = hover;
        v.widgets.hovered.bg_stroke = Stroke::NONE;
        v.widgets.active.weak_bg_fill = hover;
        v.widgets.active.bg_fill = hover;
        v.widgets.active.bg_stroke = Stroke::NONE;
        let button = egui::Button::new(
            RichText::new(label)
                .color(text)
                .size(THEME.dialog_label_size),
        )
        .corner_radius(THEME.control_rounding)
        .min_size(egui::vec2(0.0, THEME.field_h));
        ui.add_enabled(enabled, button)
    })
    .inner
}

/// The width [`footer_button`] gets for `label`: the text plus the button
/// padding on both sides.
pub fn footer_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let text = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(THEME.dialog_label_size),
        THEME.text_normal,
    );
    (text.size().x + 2.0 * ui.spacing().button_padding.x).max(ui.spacing().interact_size.x)
}

/// A row control made of a flexible part and a "Browse..." button flush with
/// the row's right edge, one item gap away from the flexible part.
///
/// `add` fills the width left of the button. Returns true when Browse was
/// clicked.
pub fn with_browse(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) -> bool {
    let row = ui.available_rect_before_wrap();
    let button_w = footer_button_width(ui, "Browse...");
    let gap = ui.spacing().item_spacing.x;
    let left = egui::Rect::from_min_max(
        row.min,
        egui::pos2((row.right() - button_w - gap).max(row.left()), row.bottom()),
    );
    let right = egui::Rect::from_min_max(egui::pos2(row.right() - button_w, row.top()), row.max);
    add(&mut ui.new_child(
        UiBuilder::new()
            .max_rect(left)
            .layout(Layout::left_to_right(Align::Center)),
    ));
    let mut button = ui.new_child(
        UiBuilder::new()
            .max_rect(right)
            .layout(Layout::left_to_right(Align::Center)),
    );
    footer_button(&mut button, "Browse...", FooterButton::Cancel, true).clicked()
}

/// The slider half of a slider + typed value control: fills the row width
/// minus the value box and the gap between them. Returns the slider's outcome
/// (new value, double-click reset).
fn value_slider(
    ui: &mut egui::Ui,
    id_salt: &str,
    value: f32,
    range: (f32, f32),
) -> crate::ui::widgets::fader::SliderOutcome {
    let slider_w = (ui.available_width() - THEME.value_box_w - THEME.value_gap).max(0.0);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(slider_w, THEME.field_h - 6.0),
        egui::Sense::click_and_drag(),
    );
    crate::ui::widgets::fader::slider_bare(
        ui,
        egui::Id::new((id_salt, "slider")),
        rect,
        value,
        range,
        false,
    )
}

/// A dB control: `slider_bare` plus a typed value in JetBrains Mono.
///
/// The slider fills the row width minus the value box and the gap between
/// them. Double-click on the slider resets to 0.0.
pub fn db_control(ui: &mut egui::Ui, id_salt: &str, value: &mut f32, range: (f32, f32)) {
    let out = value_slider(ui, id_salt, *value, range);
    if let Some(v) = out.value {
        *value = v;
    }
    if out.reset {
        *value = 0.0;
    }
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(Theme::mono(THEME.dialog_label_size));
        ui.add_sized(
            [THEME.value_box_w, THEME.field_h],
            egui::DragValue::new(value)
                .range(range.0..=range.1)
                .speed(0.1)
                .suffix(" dB"),
        );
    });
}

/// A whole-millisecond control: the same slider + typed value layout as
/// [`db_control`], with a signed mono value ("+0 ms"). Double-click on the
/// slider resets to 0.
pub fn ms_control(ui: &mut egui::Ui, id_salt: &str, value: &mut i32, range: (i32, i32)) {
    let out = value_slider(ui, id_salt, *value as f32, (range.0 as f32, range.1 as f32));
    if let Some(v) = out.value {
        *value = (v.round() as i32).clamp(range.0, range.1);
    }
    if out.reset {
        *value = 0;
    }
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(Theme::mono(THEME.dialog_label_size));
        ui.add_sized(
            [THEME.value_box_w, THEME.field_h],
            egui::DragValue::new(value)
                .range(range.0..=range.1)
                .speed(0.5)
                .custom_formatter(|n, _| format!("{:+}", n.round() as i64))
                .suffix(" ms"),
        );
    });
}

/// Title row with a phosphor WARNING icon in #E3C04A left of the title.
pub fn warning_title(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(egui_phosphor::fill::WARNING)
                .font(egui::FontId::proportional(16.0))
                .color(THEME.meter_yellow),
        );
        title(ui, text);
    });
}

/// The footer: a 1 px border line above, 16 px top padding, and the buttons
/// right-aligned while reading left to right ("OK  Cancel"): the button
/// group is measured in an invisible sizing pass, then laid out left to
/// right in a right-anchored group (first added = leftmost).
pub fn footer(ui: &mut egui::Ui, mut add: impl FnMut(&mut egui::Ui)) {
    let (line, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), THEME.hairline),
        Sense::hover(),
    );
    ui.painter().rect_filled(line, 0.0, THEME.control_border);
    ui.add_space(THEME.space_lg);
    let (area, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), THEME.field_h),
        Sense::hover(),
    );
    // Sizing pass: invisible and disabled, so nothing is painted and no
    // click can fire while the button group is measured.
    let mut measure = ui.new_child(
        UiBuilder::new()
            .invisible()
            .max_rect(area)
            .layout(Layout::left_to_right(Align::Center)),
    );
    add(&mut measure);
    // Nothing trails the last button, so the measured width is the group.
    let group_w = measure.min_rect().width();
    let group = egui::Rect::from_min_max(
        egui::pos2((area.right() - group_w).max(area.left()), area.top()),
        egui::pos2(area.right(), area.bottom()),
    );
    let _ = ui.scope_builder(
        UiBuilder::new()
            .max_rect(group)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| add(ui),
    );
}
