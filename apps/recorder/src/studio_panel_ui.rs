//! The Studio inspector panels (Studio spec "Cursor", "Keys" and "Frame"). They edit a
//! [`StudioSettings`] in place and say whether anything changed; the caller shows the change in the
//! preview and saves it.

use std::ops::RangeInclusive;
use std::path::PathBuf;

use egui::{Align, Layout, RichText, Ui, UiBuilder};
use ui_kit::tokens::{color, space, typography};
use ui_kit::{SliderKind, inspector_group, inspector_row, segmented_filled, switch};

use crate::studio::settings::{
    BackgroundKind, ClickEffect, CursorSettings, FrameSettings, Highlight, KeysPosition, KeysSettings, KeysSize, Shadow, Smoothing, StudioSettings,
};
use crate::studio_ui::StudioTool;

/// What the Keys panel says for a take without shortcuts.
const NO_CHORDS_NOTE: &str = "Keyboard shortcuts were not recorded for this take";

/// How long the cursor waits before it hides, when hiding is switched on.
const DEFAULT_HIDE_S: f32 = 3.0;

const SMOOTHING: [(Smoothing, &str); 3] = [(Smoothing::Off, "Off"), (Smoothing::Light, "Light"), (Smoothing::Strong, "Strong")];
const CLICK_EFFECTS: [(ClickEffect, &str); 3] = [(ClickEffect::None, "None"), (ClickEffect::Ring, "Ring"), (ClickEffect::Ripple, "Ripple")];
const HIGHLIGHTS: [(Highlight, &str); 2] = [(Highlight::Off, "Off"), (Highlight::Spotlight, "Spotlight")];
const POSITIONS: [(KeysPosition, &str); 4] = [
    (KeysPosition::BottomCenter, "Bottom center"),
    (KeysPosition::BottomLeft, "Bottom left"),
    (KeysPosition::BottomRight, "Bottom right"),
    (KeysPosition::TopCenter, "Top center"),
];
const SIZES: [(KeysSize, &str); 3] = [(KeysSize::Small, "Small"), (KeysSize::Medium, "Medium"), (KeysSize::Large, "Large")];
const BACKGROUNDS: [(BackgroundKind, &str); 3] = [(BackgroundKind::Solid, "Solid"), (BackgroundKind::Gradient, "Gradient"), (BackgroundKind::Image, "Image")];
const GRADIENTS: [(u8, &str); 6] = [(1, "1"), (2, "2"), (3, "3"), (4, "4"), (5, "5"), (6, "6")];
const SHADOWS: [(Shadow, &str); 3] = [(Shadow::Off, "Off"), (Shadow::Soft, "Soft"), (Shadow::Strong, "Strong")];

/// What the panels need to know about the open take besides its settings.
#[derive(Debug, Clone, Copy)]
pub struct PanelFacts {
    /// Whether the take recorded any shortcut.
    pub has_chords: bool,
    /// Why the picked background image is not on screen.
    pub image_note: Option<&'static str>,
}

/// What a frame of a panel did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PanelOutcome {
    /// A setting changed.
    pub changed: bool,
    /// A file was picked, which is to be read again even if its path is the old one.
    pub image_picked: bool,
}

/// Draws the panel of `tool`; a tool without one draws nothing.
pub fn show(ui: &mut Ui, tool: StudioTool, settings: &mut StudioSettings, facts: PanelFacts) -> PanelOutcome {
    let mut outcome = PanelOutcome::default();
    match tool {
        StudioTool::Cursor => outcome.changed = cursor_panel(ui, &mut settings.cursor),
        StudioTool::Keys => outcome.changed = keys_panel(ui, &mut settings.keys, facts.has_chords),
        StudioTool::Frame => outcome = frame_panel(ui, &mut settings.frame, facts.image_note),
        StudioTool::Zoom | StudioTool::Export => {}
    }
    outcome
}

/// A short line of quiet text in the right column.
pub fn caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).selectable(false).wrap());
}

fn cursor_panel(ui: &mut Ui, cursor: &mut CursorSettings) -> bool {
    let mut changed = false;
    inspector_group(ui, |ui| {
        changed |= switch_row(ui, "Cursor visible", &mut cursor.visible);
        changed |= count_row(ui, "Size", &mut cursor.size_percent, 100..=300, "%");
        changed |= choice_row(ui, "Smoothing", "studio-smoothing", &mut cursor.smoothing, &SMOOTHING);
    });
    ui.add_space(space::SCALE_8);
    inspector_group(ui, |ui| {
        changed |= choice_row(ui, "Click effect", "studio-click", &mut cursor.click_effect, &CLICK_EFFECTS);
        ui.add_enabled_ui(cursor.click_effect != ClickEffect::None, |ui| {
            changed |= color_row(ui, "Click color", &mut cursor.click_color);
            changed |= count_row(ui, "Click size", &mut cursor.click_size_percent, 50..=300, "%");
        });
    });
    ui.add_space(space::SCALE_8);
    inspector_group(ui, |ui| {
        changed |= choice_row(ui, "Highlight", "studio-highlight", &mut cursor.highlight, &HIGHLIGHTS);
        ui.add_enabled_ui(cursor.highlight == Highlight::Spotlight, |ui| {
            changed |= count_row(ui, "Radius", &mut cursor.highlight_radius_px, 20..=400, " px");
        });
    });
    ui.add_space(space::SCALE_8);
    inspector_group(ui, |ui| {
        let mut hides = cursor.hide_idle_s.is_some();
        if switch_row(ui, "Hide when idle", &mut hides) {
            cursor.hide_idle_s = hides.then_some(DEFAULT_HIDE_S);
            changed = true;
        }
        ui.add_enabled_ui(hides, |ui| {
            let mut seconds = cursor.hide_idle_s.unwrap_or(DEFAULT_HIDE_S);
            let text = format!("{seconds} s");
            if slider_row(ui, "After", &mut seconds, 1.0..=10.0, 1.0, &text) {
                cursor.hide_idle_s = Some(seconds);
                changed = true;
            }
        });
    });
    changed
}

fn keys_panel(ui: &mut Ui, keys: &mut KeysSettings, has_chords: bool) -> bool {
    if !has_chords {
        caption(ui, NO_CHORDS_NOTE);
        ui.add_space(space::SCALE_8);
    }
    let mut changed = false;
    ui.add_enabled_ui(has_chords, |ui| {
        inspector_group(ui, |ui| {
            changed |= switch_row(ui, "Show shortcuts", &mut keys.show);
            ui.add_enabled_ui(keys.show, |ui| {
                inspector_row(ui, "Position", |ui| changed |= position_choice(ui, &mut keys.position), None);
                changed |= choice_row(ui, "Size", "studio-keys-size", &mut keys.size, &SIZES);
                let text = format!("{:.1} s", keys.duration_s);
                changed |= slider_row(ui, "Duration", &mut keys.duration_s, 0.8..=3.0, 0.1, &text);
            });
        });
    });
    changed
}

fn frame_panel(ui: &mut Ui, frame: &mut FrameSettings, image_note: Option<&str>) -> PanelOutcome {
    let mut outcome = PanelOutcome::default();
    inspector_group(ui, |ui| outcome.changed |= switch_row(ui, "Frame", &mut frame.enabled));
    ui.add_space(space::SCALE_8);
    ui.add_enabled_ui(frame.enabled, |ui| {
        inspector_group(ui, |ui| {
            outcome.changed |= choice_row(ui, "Background", "studio-background", &mut frame.background, &BACKGROUNDS);
            match frame.background {
                BackgroundKind::Solid => outcome.changed |= color_row(ui, "Color", &mut frame.solid_color),
                BackgroundKind::Gradient => outcome.changed |= choice_row(ui, "Preset", "studio-gradient", &mut frame.gradient_preset, &GRADIENTS),
                BackgroundKind::Image => {
                    if let Some(path) = image_row(ui, frame) {
                        frame.image = Some(path);
                        outcome = PanelOutcome { changed: true, image_picked: true };
                    }
                }
            }
        });
        if let Some(note) = image_note.filter(|_| frame.background == BackgroundKind::Image) {
            ui.add_space(space::SCALE_4);
            caption(ui, note);
        }
        ui.add_space(space::SCALE_8);
        inspector_group(ui, |ui| {
            let text = format!("{}%", frame.padding_percent);
            outcome.changed |= slider_row(ui, "Padding", &mut frame.padding_percent, 0.0..=15.0, 1.0, &text);
            outcome.changed |= count_row(ui, "Corner radius", &mut frame.corner_radius_px, 0..=32, " px");
            outcome.changed |= choice_row(ui, "Shadow", "studio-shadow", &mut frame.shadow, &SHADOWS);
        });
    });
    outcome
}

/// The picked picture's name and Browse; answers the file picked this frame, as an absolute path.
fn image_row(ui: &mut Ui, frame: &FrameSettings) -> Option<PathBuf> {
    let mut picked = None;
    inspector_row(
        ui,
        "Image",
        |ui| {
            let browse = ui.button("Browse...");
            if let Some(path) = &frame.image {
                let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                ui.add(egui::Label::new(name).truncate()).on_hover_text(path.display().to_string());
            }
            if browse.clicked() {
                picked = crate::platform::pick_image_file().map(|path| std::path::absolute(&path).unwrap_or(path));
            }
        },
        None,
    );
    picked
}

/// The four places of the shortcut overlay; too many for a segmented control.
fn position_choice(ui: &mut Ui, position: &mut KeysPosition) -> bool {
    let label = |wanted: KeysPosition| POSITIONS.iter().find(|(kind, _)| *kind == wanted).map_or("", |(_, label)| label);
    let before = *position;
    egui::ComboBox::from_id_salt("studio-keys-position").selected_text(label(before)).show_ui(ui, |ui| {
        for (kind, text) in POSITIONS {
            ui.selectable_value(position, kind, text);
        }
    });
    *position != before
}

/// An inspector row with a segmented control across it; true when the choice changed.
fn choice_row<T: Copy + PartialEq>(ui: &mut Ui, label: &str, id_salt: &'static str, value: &mut T, options: &[(T, &'static str)]) -> bool {
    let before = options.iter().position(|(kind, _)| kind == value).unwrap_or(0);
    let mut index = before;
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    inspector_row(
        ui,
        label,
        |ui| {
            // With no value on the right, inspector_row ends the control area one gap short of the
            // text inset. The cells take that gap, or "Gradient" is cramped in the narrowest column.
            let mut area = ui.max_rect();
            area.max.x += space::SCALE_8;
            let mut wide = ui.new_child(UiBuilder::new().max_rect(area).layout(Layout::left_to_right(Align::Center)));
            segmented_filled(&mut wide, id_salt, &mut index, &labels, &[]);
        },
        None,
    );
    match options.get(index) {
        Some((kind, _)) if index != before => {
            *value = *kind;
            true
        }
        _ => false,
    }
}

fn switch_row(ui: &mut Ui, label: &str, on: &mut bool) -> bool {
    let mut changed = false;
    inspector_row(ui, label, |ui| changed = switch(ui, on).on_hover_text(label).changed(), None);
    changed
}

fn color_row(ui: &mut Ui, label: &str, value: &mut [u8; 3]) -> bool {
    let mut changed = false;
    inspector_row(ui, label, |ui| changed = ui.color_edit_button_srgb(value).changed(), None);
    changed
}

/// A slider row over a whole number.
fn count_row(ui: &mut Ui, label: &str, value: &mut u32, range: RangeInclusive<u32>, suffix: &str) -> bool {
    let mut number = *value as f32;
    let text = format!("{value}{suffix}");
    let changed = slider_row(ui, label, &mut number, *range.start() as f32..=*range.end() as f32, 1.0, &text);
    if changed {
        *value = number.round() as u32;
    }
    changed
}

/// An inspector row with a slider that moves in steps of `step`, and `text` as its value on the
/// right; true when the value changed. The value is compared, because snapping can leave it where
/// it was.
fn slider_row(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>, step: f32, text: &str) -> bool {
    let before = *value;
    let mut raw = before;
    let mut focused = false;
    inspector_row(ui, label, |ui| focused = ui_kit::slider(ui, &mut raw, range.clone(), SliderKind::Accent).has_focus(), Some(text));
    if raw == before {
        return false;
    }
    let mut snapped = snap(raw, step);
    // An arrow key moves the slider by a hundredth of its range, which can be less than a step.
    let arrow = focused && ui.input(|input| input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::ArrowRight));
    if arrow && snapped == before {
        snapped = snap(before + step.copysign(raw - before), step);
    }
    *value = snapped.clamp(*range.start(), *range.end());
    *value != before
}

/// The multiple of `step` nearest to `value`. A step below one is divided by, so that tenths come
/// out as the nearest numbers to them.
fn snap(value: f32, step: f32) -> f32 {
    if step < 1.0 {
        let per_one = (1.0 / step).round();
        (value * per_one).round() / per_one
    } else {
        (value / step).round() * step
    }
}

#[cfg(test)]
mod tests {
    use egui::vec2;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    /// A panel on its own, with the settings it edits and whether any frame reported a change.
    fn harness(tool: StudioTool, has_chords: bool) -> Harness<'static, (StudioSettings, bool)> {
        let facts = PanelFacts { has_chords, image_note: None };
        let mut styled = false;
        Harness::builder().with_size(vec2(320.0, 600.0)).build_ui_state(
            move |ui, state: &mut (StudioSettings, bool)| {
                if !std::mem::replace(&mut styled, true) {
                    ui_kit::apply(ui.ctx());
                    // The new fonts are only there from the next frame on.
                    ui.ctx().request_repaint();
                    return;
                }
                state.1 |= show(ui, tool, &mut state.0, facts).changed;
            },
            (StudioSettings::default(), false),
        )
    }

    fn click(harness: &mut Harness<'static, (StudioSettings, bool)>, label: &str) {
        harness.run();
        harness.get_by_label(label).click();
        harness.run();
    }

    #[test]
    fn a_click_on_a_control_changes_the_setting_and_reports_it() {
        let mut panel = harness(StudioTool::Cursor, true);
        panel.run();
        assert!(!panel.state().1, "drawing alone is no change");
        click(&mut panel, "Strong");
        assert_eq!(panel.state().0.cursor.smoothing, Smoothing::Strong);
        assert!(panel.state().1);
    }

    #[test]
    fn keys_are_disabled_without_chords() {
        let mut without = harness(StudioTool::Keys, false);
        without.run();
        without.get_by_label(NO_CHORDS_NOTE);
        click(&mut without, "Large");
        assert_eq!((without.state().0.keys.clone(), without.state().1), (KeysSettings::default(), false));

        let mut with = harness(StudioTool::Keys, true);
        with.run();
        assert!(with.query_by_label(NO_CHORDS_NOTE).is_none());
        click(&mut with, "Large");
        assert_eq!((with.state().0.keys.size, with.state().1), (KeysSize::Large, true));
    }

    #[test]
    fn slider_values_land_on_whole_steps() {
        assert_eq!((snap(6.4, 1.0), snap(1.4999, 0.1), snap(0.8, 0.1), snap(2.04, 0.1)), (6.0, 1.5, 0.8, 2.0));
    }
}
