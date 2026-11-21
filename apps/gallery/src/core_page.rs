//! The Core page: one panel per component, every state side by side with a
//! `type.caption` label under it (spec: Design Language "Components").
//!
//! Hover and press states cannot be forced from outside a widget, so those
//! cells are idle copies with a "hover me" / "press me" note; the scrollbar
//! thumb takes its hover state as a parameter and is shown both ways. egui
//! has one focused widget at a time, so the focused copies hold the focus in
//! a slow rotation (paused after the first click or while a modal is open).

use eframe::egui;
use ui_kit::fonts;
use ui_kit::icons::regular;
use ui_kit::tokens::{color, size, space, typography};
use ui_kit::{
    SliderKind, Theme, chip, icon_button, inspector_group, inspector_header, inspector_row, list_row, panel, primary_button, scroll_thumb, search_field,
    segmented, slider, status_bar, switch, text_field,
};

/// Width of the demo text and search fields, so three cells fit one row.
const FIELD_WIDTH: f32 = 220.0;
/// Width of the demo sliders.
const SLIDER_WIDTH: f32 = 240.0;
/// Width of the demo list rows.
const LIST_ROW_WIDTH: f32 = 260.0;
/// Width of the demo status bar.
const STATUS_WIDTH: f32 = 280.0;
/// Width of the inspector panel.
const INSPECTOR_WIDTH: f32 = 340.0;
/// Width the color panel wraps its swatches at.
const COLOR_WIDTH: f32 = 520.0;
/// Natural width of the widest Core page panel (the five-cell button and
/// field rows). The page wraps panels into as many columns of this width as
/// fit (up to three), so every panel keeps its natural width and no row
/// overflows (8px gaps from the tokens).
const GRID_COLUMN_WIDTH: f32 = 700.0;
/// Height of the demo scrollbar thumbs.
const THUMB_HEIGHT: f32 = 120.0;
/// Side of a color swatch.
const SWATCH_SIDE: f32 = 24.0;
/// The 1 px hairline stroke width (the same rule ui-kit uses).
const HAIRLINE: f32 = 1.0;
/// Seconds each focused demo copy holds the focus before the next one does.
const FOCUS_SECONDS: f64 = 1.5;
/// How many focusable demos share the focus rotation.
const FOCUS_SLOTS: usize = 9;
/// Focus rotation slot of the primary button demo.
const FOCUS_PRIMARY: usize = 0;
/// Focus rotation slot of the icon button demo.
const FOCUS_ICON: usize = 1;
/// Focus rotation slot of the segmented control demo.
const FOCUS_SEGMENTED: usize = 2;
/// Focus rotation slot of the chip demo.
const FOCUS_CHIP: usize = 3;
/// Focus rotation slot of the switch demo.
const FOCUS_SWITCH: usize = 4;
/// Focus rotation slot of the slider demo.
const FOCUS_SLIDER: usize = 5;
/// Focus rotation slot of the text field demo.
const FOCUS_TEXT_FIELD: usize = 6;
/// Focus rotation slot of the search field demo.
const FOCUS_SEARCH: usize = 7;
/// Focus rotation slot of the list row demo.
const FOCUS_LIST_ROW: usize = 8;

/// The Core page's demo state: one field per interactive demo widget.
pub struct CoreState {
    /// The 2-segment segmented control's selection.
    segments2: usize,
    /// The 3-segment segmented control's selection.
    segments3: usize,
    /// The 4-segment segmented control's selection.
    segments4: usize,
    /// The inspector row's 2-segment control's selection.
    inspector_mode: usize,
    /// The off chip (the idle, hover and press copies share it).
    chip: bool,
    /// The on chip (the focused copy shares it).
    chip_on: bool,
    /// The off switch.
    switch: bool,
    /// The on switch.
    switch_on: bool,
    /// The accent slider value.
    slider_accent: f32,
    /// The zoom slider value.
    slider_zoom: f32,
    /// The inspector gain slider value.
    inspector_slider: f32,
    /// The idle text field's text.
    field_text: String,
    /// The focused text field copy's text.
    field_focused: String,
    /// The idle search field's text.
    search_text: String,
    /// The focused search field copy's text.
    search_focused: String,
    /// True while the sample dialog is open (a cell: the dialog's footer
    /// closes it from inside ui-kit's dialog call).
    pub dialog_open: std::cell::Cell<bool>,
    /// True once the user clicked anywhere: the focus rotation stops.
    focus_paused: bool,
}

impl CoreState {
    /// Fresh demo state: defaults everywhere, dialog closed.
    pub fn new() -> Self {
        Self {
            segments2: 0,
            segments3: 0,
            segments4: 0,
            inspector_mode: 0,
            chip: false,
            chip_on: true,
            switch: false,
            switch_on: true,
            slider_accent: 0.4,
            slider_zoom: 0.75,
            inspector_slider: 0.0,
            field_text: String::new(),
            field_focused: String::new(),
            search_text: String::new(),
            search_focused: String::new(),
            dialog_open: std::cell::Cell::new(false),
            focus_paused: false,
        }
    }
}

/// Draws the whole Core page: one panel per component, arranged in a
/// wrapping grid (three columns when the width allows, else two, 8px gaps);
/// each panel keeps its natural width up to its column width.
pub fn show(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    if !state.focus_paused && ui.input(|i| i.pointer.any_click()) {
        state.focus_paused = true;
    }
    let columns = ((ui.available_width() + space::WINDOW_GAP) / (GRID_COLUMN_WIDTH + space::WINDOW_GAP)).floor().clamp(1.0, 3.0) as usize;
    let column_width = (ui.available_width() - (columns - 1) as f32 * space::WINDOW_GAP) / columns as f32;
    // One row of panels at a time: each panel sits in its own vertical scope
    // (an explicit layout, so no grid placer leaks into the panel frames) and
    // keeps its natural width up to the column width.
    for row in PANELS.chunks(columns) {
        ui.horizontal(|ui| {
            for panel_fn in row {
                ui.vertical(|ui| {
                    ui.set_max_width(column_width);
                    panel_fn(ui, state, modal_open);
                });
            }
        });
    }
}

/// One Core page panel as a plain function pointer, so the panel list can be
/// chunked into grid rows.
type PanelFn = fn(&mut egui::Ui, &mut CoreState, bool);

/// Every Core page panel, in grid order.
const PANELS: &[PanelFn] = &[
    |ui, state, modal_open| primary_button_panel(ui, state, modal_open),
    |ui, state, modal_open| icon_button_panel(ui, state, modal_open),
    |ui, state, modal_open| segmented_panel(ui, state, modal_open),
    |ui, state, modal_open| chip_panel(ui, state, modal_open),
    |ui, state, modal_open| switch_panel(ui, state, modal_open),
    |ui, state, modal_open| slider_panel(ui, state, modal_open),
    |ui, state, modal_open| text_field_panel(ui, state, modal_open),
    |ui, state, modal_open| search_field_panel(ui, state, modal_open),
    |ui, state, modal_open| list_row_panel(ui, state, modal_open),
    |ui, state, _modal_open| inspector_panel(ui, state),
    |ui, _state, _modal_open| status_bar_panel(ui),
    |ui, _state, _modal_open| scroll_thumb_panel(ui),
    |ui, state, _modal_open| dialog_panel(ui, state),
    |ui, _state, _modal_open| type_panel(ui),
    |ui, _state, _modal_open| color_panel(ui),
];

/// Requests focus for the demo copy that currently owns the focus rotation
/// slot, so every focusable component shows its focus ring in turn.
fn maybe_focus(ui: &mut egui::Ui, state: &CoreState, modal_open: bool, slot: usize, response: egui::Response) {
    if modal_open || state.focus_paused {
        return;
    }
    let current = (ui.input(|i| i.time) / FOCUS_SECONDS) as usize % FOCUS_SLOTS;
    if slot == current {
        response.request_focus();
    }
}

/// One gallery panel: a `type.headline` title over the demo content.
fn demo_panel(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    panel(ui, |ui| {
        ui.label(egui::RichText::new(title).font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY));
        ui.add_space(space::SCALE_2);
        add(ui);
    });
}

/// One state cell: the demo content with a `type.caption` label under it.
fn state_cell(ui: &mut egui::Ui, caption: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        add(ui);
        ui.add_space(space::SCALE_2);
        ui.label(egui::RichText::new(caption).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
    });
}

/// The primary button: idle, hover, press, focused, disabled.
fn primary_button_panel(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    demo_panel(ui, "Primary button", |ui| {
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "idle", |ui| {
                primary_button(ui, "Export");
            });
            state_cell(ui, "hover me", |ui| {
                primary_button(ui, "Export");
            });
            state_cell(ui, "press me", |ui| {
                primary_button(ui, "Export");
            });
            state_cell(ui, "focused", |ui| {
                let response = primary_button(ui, "Export");
                maybe_focus(ui, state, modal_open, FOCUS_PRIMARY, response);
            });
            state_cell(ui, "disabled", |ui| {
                ui.add_enabled_ui(false, |ui| {
                    primary_button(ui, "Export");
                });
            });
        });
    });
}

/// The icon button: idle, hover, press, focused, disabled.
fn icon_button_panel(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    demo_panel(ui, "Icon button", |ui| {
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "idle", |ui| {
                icon_button(ui, regular::GEAR_SIX, "Settings");
            });
            state_cell(ui, "hover me", |ui| {
                icon_button(ui, regular::GEAR_SIX, "Settings");
            });
            state_cell(ui, "press me", |ui| {
                icon_button(ui, regular::GEAR_SIX, "Settings");
            });
            state_cell(ui, "focused", |ui| {
                let response = icon_button(ui, regular::GEAR_SIX, "Settings");
                maybe_focus(ui, state, modal_open, FOCUS_ICON, response);
            });
            state_cell(ui, "disabled", |ui| {
                ui.add_enabled_ui(false, |ui| {
                    icon_button(ui, regular::GEAR_SIX, "Settings");
                });
            });
        });
    });
}

/// The segmented control: 2, 3 and 4 segments, focused and disabled. Each
/// copy carries its own id_salt: several controls with the same labels must
/// never share widget ids.
fn segmented_panel(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    demo_panel(ui, "Segmented control", |ui| {
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "2 segments", |ui| {
                segmented(ui, "core-segments-2", &mut state.segments2, &["Edit", "Mix"]);
            });
            state_cell(ui, "3 segments", |ui| {
                segmented(ui, "core-segments-3", &mut state.segments3, &["Edit", "Mix", "Master"]);
            });
            state_cell(ui, "4 segments", |ui| {
                segmented(ui, "core-segments-4", &mut state.segments4, &["Cut", "Fade", "Slip", "Roll"]);
            });
        });
        ui.add_space(space::SCALE_4);
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "focused", |ui| {
                let response = segmented(ui, "core-segments-3-focused", &mut state.segments3, &["Edit", "Mix", "Master"]);
                maybe_focus(ui, state, modal_open, FOCUS_SEGMENTED, response);
            });
            state_cell(ui, "disabled", |ui| {
                ui.add_enabled_ui(false, |ui| {
                    segmented(ui, "core-segments-3-disabled", &mut state.segments3, &["Edit", "Mix", "Master"]);
                });
            });
        });
    });
}

/// The chip: off, on, hover, press, focused, disabled.
fn chip_panel(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    demo_panel(ui, "Chip", |ui| {
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "idle", |ui| {
                chip(ui, &mut state.chip, "Snap");
            });
            state_cell(ui, "on", |ui| {
                chip(ui, &mut state.chip_on, "Ripple");
            });
            state_cell(ui, "hover me", |ui| {
                chip(ui, &mut state.chip, "Linked");
            });
            state_cell(ui, "press me", |ui| {
                chip(ui, &mut state.chip_on, "Snap");
            });
            state_cell(ui, "focused", |ui| {
                let response = chip(ui, &mut state.chip, "Snap");
                maybe_focus(ui, state, modal_open, FOCUS_CHIP, response);
            });
            state_cell(ui, "disabled", |ui| {
                ui.add_enabled_ui(false, |ui| {
                    chip(ui, &mut state.chip_on, "Ripple");
                });
            });
        });
    });
}

/// The switch: off, on, focused, disabled.
fn switch_panel(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    demo_panel(ui, "Switch", |ui| {
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "idle", |ui| {
                switch(ui, &mut state.switch);
            });
            state_cell(ui, "on", |ui| {
                switch(ui, &mut state.switch_on);
            });
            state_cell(ui, "focused", |ui| {
                let response = switch(ui, &mut state.switch_on);
                maybe_focus(ui, state, modal_open, FOCUS_SWITCH, response);
            });
            state_cell(ui, "disabled", |ui| {
                ui.add_enabled_ui(false, |ui| {
                    switch(ui, &mut state.switch);
                });
            });
        });
    });
}

/// The slider: Accent and Zoom fills, focused and disabled.
fn slider_panel(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    demo_panel(ui, "Slider", |ui| {
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "accent", |ui| {
                ui.set_max_width(SLIDER_WIDTH);
                slider(ui, &mut state.slider_accent, 0.0..=1.0, SliderKind::Accent);
            });
            state_cell(ui, "zoom", |ui| {
                ui.set_max_width(SLIDER_WIDTH);
                slider(ui, &mut state.slider_zoom, 0.0..=1.0, SliderKind::Zoom);
            });
        });
        ui.add_space(space::SCALE_4);
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "focused", |ui| {
                ui.set_max_width(SLIDER_WIDTH);
                let response = slider(ui, &mut state.slider_accent, 0.0..=1.0, SliderKind::Accent);
                maybe_focus(ui, state, modal_open, FOCUS_SLIDER, response);
            });
            state_cell(ui, "disabled", |ui| {
                ui.add_enabled_ui(false, |ui| {
                    ui.set_max_width(SLIDER_WIDTH);
                    slider(ui, &mut state.slider_zoom, 0.0..=1.0, SliderKind::Zoom);
                });
            });
        });
    });
}

/// The text field: idle, focused, disabled.
fn text_field_panel(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    demo_panel(ui, "Text field", |ui| {
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "idle", |ui| {
                ui.set_max_width(FIELD_WIDTH);
                text_field(ui, &mut state.field_text, "Project name");
            });
            state_cell(ui, "focused", |ui| {
                ui.set_max_width(FIELD_WIDTH);
                let response = text_field(ui, &mut state.field_focused, "Focused copy");
                maybe_focus(ui, state, modal_open, FOCUS_TEXT_FIELD, response);
            });
            state_cell(ui, "disabled", |ui| {
                ui.add_enabled_ui(false, |ui| {
                    ui.set_max_width(FIELD_WIDTH);
                    text_field(ui, &mut state.field_text, "Project name");
                });
            });
        });
    });
}

/// The search field: idle, focused, disabled.
fn search_field_panel(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    demo_panel(ui, "Search field", |ui| {
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "idle", |ui| {
                ui.set_max_width(FIELD_WIDTH);
                search_field(ui, &mut state.search_text, "Search takes");
            });
            state_cell(ui, "focused", |ui| {
                ui.set_max_width(FIELD_WIDTH);
                let response = search_field(ui, &mut state.search_focused, "Focused copy");
                maybe_focus(ui, state, modal_open, FOCUS_SEARCH, response);
            });
            state_cell(ui, "disabled", |ui| {
                ui.add_enabled_ui(false, |ui| {
                    ui.set_max_width(FIELD_WIDTH);
                    search_field(ui, &mut state.search_text, "Search takes");
                });
            });
        });
    });
}

/// The list row: plain, with dot, with dot and trailing value, selected and
/// disabled.
fn list_row_panel(ui: &mut egui::Ui, state: &mut CoreState, modal_open: bool) {
    demo_panel(ui, "List row", |ui| {
        ui.set_max_width(LIST_ROW_WIDTH);
        ui.spacing_mut().item_spacing.y = space::SCALE_2;
        state_cell(ui, "idle", |ui| {
            list_row(ui, false, None, "Take 1", None);
        });
        state_cell(ui, "with dot", |ui| {
            list_row(ui, false, Some(color::MARKER_NOTE), "Take 2", None);
        });
        state_cell(ui, "dot and value", |ui| {
            list_row(ui, false, Some(color::MARKER_RETAKE), "Take 3", Some("02:14"));
        });
        state_cell(ui, "selected", |ui| {
            let response = list_row(ui, true, Some(color::MARKER_CUT), "Take 4", Some("03:02"));
            maybe_focus(ui, state, modal_open, FOCUS_LIST_ROW, response);
        });
        state_cell(ui, "disabled", |ui| {
            ui.add_enabled_ui(false, |ui| {
                list_row(ui, false, None, "Take 5", None);
            });
        });
    });
}

/// The inspector: header over a group of four rows.
fn inspector_panel(ui: &mut egui::Ui, state: &mut CoreState) {
    demo_panel(ui, "Inspector", |ui| {
        ui.set_max_width(INSPECTOR_WIDTH);
        inspector_header(ui, "Interview take 3", "Clip \u{b7} audio");
        ui.add_space(space::SCALE_4);
        inspector_group(ui, |ui| {
            inspector_row(
                ui,
                "Gain",
                |ui| {
                    slider(ui, &mut state.inspector_slider, -60.0..=6.0, SliderKind::Accent);
                },
                Some("0.0 dB"),
            );
            inspector_row(
                ui,
                "Solo",
                |ui| {
                    switch(ui, &mut state.switch);
                },
                Some("Off"),
            );
            inspector_row(
                ui,
                "Snap",
                |ui| {
                    chip(ui, &mut state.chip, "Snap");
                },
                Some("On"),
            );
            inspector_row(
                ui,
                "Mode",
                |ui| {
                    segmented(ui, "core-inspector-mode", &mut state.inspector_mode, &["A", "B"]);
                },
                Some("A"),
            );
        });
    });
}

/// The status bar: the caption items over the window background.
fn status_bar_panel(ui: &mut egui::Ui) {
    demo_panel(ui, "Status bar", |ui| {
        ui.set_max_width(STATUS_WIDTH);
        status_bar(ui, &["Ready", "48 kHz", "24 bit"]);
    });
}

/// The scrollbar thumb: idle and forced hovered.
fn scroll_thumb_panel(ui: &mut egui::Ui) {
    demo_panel(ui, "Scroll thumb", |ui| {
        ui.horizontal_wrapped(|ui| {
            state_cell(ui, "idle", |ui| {
                thumb(ui, false);
            });
            state_cell(ui, "hovered", |ui| {
                thumb(ui, true);
            });
        });
    });
}

/// One demo scrollbar thumb.
fn thumb(ui: &mut egui::Ui, hovered: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size::SCROLLBAR_THUMB_WIDTH, THUMB_HEIGHT), egui::Sense::hover());
    scroll_thumb(ui.painter(), rect, hovered);
}

/// The dialog: a button that opens the sample dialog (drawn in app.rs).
fn dialog_panel(ui: &mut egui::Ui, state: &mut CoreState) {
    demo_panel(ui, "Dialog", |ui| {
        state_cell(ui, "idle", |ui| {
            if primary_button(ui, "Open dialog").clicked() {
                state.dialog_open.set(true);
            }
        });
    });
}

/// The type scale: every type token rendered with its name.
fn type_panel(ui: &mut egui::Ui) {
    demo_panel(ui, "Type scale", |ui| {
        // type.caption is weight 400: the default proportional family carries
        // Inter Regular, the named families only carry the heavier weights.
        let rows = [
            ("type.title", fonts::semibold(typography::TITLE_SIZE), "Interview take 3"),
            ("type.headline", fonts::semibold(typography::HEADLINE_SIZE), "Interview take 3"),
            ("type.body", fonts::medium(typography::BODY_SIZE), "Everything else reads in the body style."),
            ("type.caption", egui::FontId::proportional(typography::CAPTION_SIZE), "Subtitles and status bars"),
            ("type.micro", fonts::medium(typography::MICRO_SIZE), "Tool strip labels"),
            ("type.time.large", fonts::mono_semibold(typography::TIME_LARGE_SIZE), "00:01:23.45"),
            ("type.value", fonts::mono(typography::VALUE_SIZE), "00:01:23.45"),
        ];
        ui.spacing_mut().item_spacing.y = space::SCALE_4;
        for (name, font, sample) in rows {
            state_cell(ui, name, move |ui| {
                ui.label(egui::RichText::new(sample).font(font).color(color::TEXT_PRIMARY));
            });
        }
    });
}

/// The static color tokens with their token names; the accent row is live
/// from the theme.
const COLOR_TOKENS: &[(&str, egui::Color32)] = &[
    ("bg.ground", color::BG_GROUND),
    ("bg.panel", color::BG_PANEL),
    ("bg.group", color::BG_GROUP),
    ("bg.control", color::BG_CONTROL),
    ("bg.hover", color::BG_HOVER),
    ("bg.selected", color::BG_SELECTED),
    ("bg.row.selected", color::BG_ROW_SELECTED),
    ("bg.track.selected", color::BG_TRACK_SELECTED),
    ("bg.track.alt", color::BG_TRACK_ALT),
    ("bg.clip", color::BG_CLIP),
    ("stroke.hairline", color::STROKE_HAIRLINE),
    ("stroke.separator", color::STROKE_SEPARATOR),
    ("stroke.clip", color::STROKE_CLIP),
    ("text.primary", color::TEXT_PRIMARY),
    ("text.secondary", color::TEXT_SECONDARY),
    ("text.tertiary", color::TEXT_TERTIARY),
    ("text.disabled", color::TEXT_DISABLED),
    ("sem.record", color::SEM_RECORD),
    ("sem.meter.ok", color::SEM_METER_OK),
    ("sem.meter.warn", color::SEM_METER_WARN),
    ("sem.meter.clip", color::SEM_METER_CLIP),
    ("sem.success", color::SEM_SUCCESS),
    ("sem.pending", color::SEM_PENDING),
    ("marker.note", color::MARKER_NOTE),
    ("marker.retake", color::MARKER_RETAKE),
    ("marker.cut", color::MARKER_CUT),
    ("marker.broll", color::MARKER_BROLL),
    ("marker.system", color::MARKER_SYSTEM),
    ("glass.fill", color::GLASS_FILL),
    ("glass.top-highlight", color::GLASS_TOP_HIGHLIGHT),
    ("glass.rim", color::GLASS_RIM),
    ("glass.shadow", color::GLASS_SHADOW),
    ("caption.close.hover", color::CAPTION_CLOSE_HOVER),
    ("caption.close.glyph.hover", color::CAPTION_CLOSE_GLYPH_HOVER),
];

/// The color swatches: every color token with its name and hex, the accent
/// tokens live from the theme.
fn color_panel(ui: &mut egui::Ui) {
    demo_panel(ui, "Color", |ui| {
        ui.set_max_width(COLOR_WIDTH);
        ui.label(
            egui::RichText::new("Surfaces, text, meaning, markers, glass and chrome").font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY),
        );
        ui.add_space(space::SCALE_2);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(space::SCALE_12, space::SCALE_8);
            for (name, value) in COLOR_TOKENS {
                swatch(ui, name, *value);
            }
        });
        ui.add_space(space::SCALE_4);
        ui.label(egui::RichText::new("Accent tokens, live from the theme").font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
        ui.add_space(space::SCALE_2);
        let accent = Theme::get(ui.ctx()).accent;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(space::SCALE_12, space::SCALE_8);
            for (name, value) in [
                ("accent.pick", accent.pick),
                ("accent", accent.accent),
                ("accent.hover", accent.hover),
                ("accent.pressed", accent.pressed),
                ("accent.on", accent.on),
                ("accent.tint", accent.tint),
                ("accent.content", accent.content),
            ] {
                swatch(ui, name, value);
            }
        });
    });
}

/// One color swatch: the color over a hairline rim, its token name and hex.
fn swatch(ui: &mut egui::Ui, name: &str, value: egui::Color32) {
    ui.vertical(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(SWATCH_SIDE, SWATCH_SIDE), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, value);
        painter.rect_stroke(rect, 0.0, egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE), egui::StrokeKind::Inside);
        ui.label(egui::RichText::new(name).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_SECONDARY));
        ui.label(egui::RichText::new(hex(value)).font(fonts::mono(typography::VALUE_SIZE)).color(color::TEXT_TERTIARY));
    });
}

/// Formats a color as `#RRGGBB`, or `#RRGGBBAA` when it has alpha.
pub fn hex(c: egui::Color32) -> String {
    if c.a() == 255 { format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()) } else { format!("#{:02X}{:02X}{:02X}{:02X}", c.r(), c.g(), c.b(), c.a()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An opaque color gets six hex digits.
    #[test]
    fn opaque_colors_get_six_hex_digits() {
        assert_eq!(hex(egui::Color32::from_rgb(0x5B, 0x9D, 0xFF)), "#5B9DFF"); // style-lint: allow test fixture, not a design value
    }

    /// A translucent color gets eight hex digits.
    #[test]
    fn translucent_colors_get_eight_hex_digits() {
        assert_eq!(hex(egui::Color32::from_rgba_premultiplied(0, 0, 0, 128)), "#00000080"); // style-lint: allow test fixture, not a design value
    }
}
