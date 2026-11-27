//! The Layout tool's groups: which pictures the composite shows and how the camera looks
//! (spec "Setup mode": View, Camera position, Crop, Frame; the Background group is in
//! `background_ui`).
//!
//! The settings live in the active profile in the shared [`ProfileStore`]. A change goes through the store,
//! which saves it later; the compose thread picks it up on the next frame, so the preview follows live.
//! The groups are read-only while the profile is locked (a take or the replay buffer), and the
//! picture-in-picture groups are disabled while the view is not Picture in picture.

use std::ops::RangeInclusive;

use egui::Ui;
use ui_kit::SliderKind;
use ui_kit::tokens::space;
use ui_kit::{inspector_group, inspector_row, segmented, segmented_filled};

use crate::config::{AnchorCorner, LayoutKind, PipConfig, PipShape};
use crate::profile_ui::{Control, Lock};
use crate::profiles::ProfileStore;

/// The corners in the order the picker lists them.
const CORNERS: [AnchorCorner; 4] = [AnchorCorner::TopLeft, AnchorCorner::TopRight, AnchorCorner::BottomLeft, AnchorCorner::BottomRight];

/// The views of the View group's segmented control, in order, with the short
/// labels the panel's width needs; the full names are the segment tooltips.
const VIEWS: [(LayoutKind, &str); 3] = [(LayoutKind::ScreenOnly, "Screen"), (LayoutKind::Pip, "PiP"), (LayoutKind::CamOnly, "Cam")];

/// The full name of each view, as the segment tooltips.
const VIEW_NAMES: [&str; 3] = ["Screen only", "Picture in picture", "Cam only"];

/// The shapes of the Frame group's segmented control, in order.
const SHAPES: [(PipShape, &str); 2] = [(PipShape::Rectangle, "Rectangle"), (PipShape::Circle, "Circle")];

/// The text of a corner in the picker.
fn corner_label(corner: AnchorCorner) -> &'static str {
    match corner {
        AnchorCorner::TopLeft => "Top left",
        AnchorCorner::TopRight => "Top right",
        AnchorCorner::BottomLeft => "Bottom left",
        AnchorCorner::BottomRight => "Bottom right",
    }
}

/// Draws the Layout tool's groups for the active profile; `lock` makes them read-only.
pub fn show(ui: &mut Ui, store: &mut ProfileStore, profile_name: &str, lock: Lock) {
    let Some(profile) = store.get(profile_name) else {
        ui.weak("No profile is active.");
        return;
    };
    let mut layout = profile.layout;
    let mut pip = profile.pip.clone();
    let webcam_on = profile.webcam.is_some();
    let mut changed = false;
    // View group (spec "Setup mode": Screen only, Picture in picture, Cam only).
    inspector_group(ui, |ui| {
        lock.scope(ui, Control::Profile, true, |ui| {
            inspector_row(
                ui,
                "View",
                |ui| {
                    changed |= view_choice(ui, &mut layout);
                },
                None,
            );
        });
    });
    if lock.recording {
        ui.add_space(space::SCALE_4);
        ui.weak("The layout is fixed while recording.");
    }
    if !webcam_on && layout != LayoutKind::ScreenOnly {
        ui.add_space(space::SCALE_4);
        ui.weak("The webcam is off, so only the screen is shown.");
    }
    ui.add_space(space::SCALE_8);
    // The picture-in-picture groups are disabled while the view is not Picture in picture.
    camera_position_group(ui, &mut pip, lock, layout == LayoutKind::Pip, &mut changed);
    ui.add_space(space::SCALE_8);
    crop_group(ui, &mut pip, lock, layout == LayoutKind::Pip, &mut changed);
    ui.add_space(space::SCALE_8);
    frame_group(ui, &mut pip, lock, layout == LayoutKind::Pip, &mut changed);
    ui.add_space(space::SCALE_8);
    crate::background_ui::show(ui, store, profile_name, lock);
    if changed {
        store.edit(profile_name, |profile| {
            profile.layout = layout;
            profile.pip = pip;
        });
    }
}

/// The Camera position group: the corner, offsets and scale of the camera picture (PiP only).
fn camera_position_group(ui: &mut Ui, pip: &mut PipConfig, lock: Lock, enabled: bool, changed: &mut bool) {
    inspector_group(ui, |ui| {
        lock.scope(ui, Control::Profile, enabled, |ui| {
            inspector_row(
                ui,
                "Corner",
                |ui| {
                    let selected = corner_label(pip.anchor);
                    let mut picked = None;
                    egui::ComboBox::from_id_salt("pip-corner").selected_text(selected).show_ui(ui, |ui| {
                        for corner in CORNERS {
                            if ui.selectable_label(pip.anchor == corner, corner_label(corner)).clicked() {
                                picked = Some(corner);
                            }
                        }
                    });
                    if let Some(corner) = picked {
                        pip.anchor = corner;
                        *changed = true;
                    }
                },
                None,
            );
            *changed |= slider_row(ui, "X offset", &mut pip.offset_x_pct, 0.0..=50.0, " %");
            *changed |= slider_row(ui, "Y offset", &mut pip.offset_y_pct, 0.0..=50.0, " %");
            *changed |= slider_row(ui, "Scale", &mut pip.scale_pct, 5.0..=100.0, " %");
        });
    });
}

/// The Crop group: how much of the camera picture is cropped away (PiP only).
fn crop_group(ui: &mut Ui, pip: &mut PipConfig, lock: Lock, enabled: bool, changed: &mut bool) {
    inspector_group(ui, |ui| {
        lock.scope(ui, Control::Profile, enabled, |ui| {
            *changed |= slider_row(ui, "Crop left", &mut pip.crop_left_pct, 0.0..=90.0, " %");
            *changed |= slider_row(ui, "Crop right", &mut pip.crop_right_pct, 0.0..=90.0, " %");
            *changed |= slider_row(ui, "Crop top", &mut pip.crop_top_pct, 0.0..=90.0, " %");
            *changed |= slider_row(ui, "Crop bottom", &mut pip.crop_bottom_pct, 0.0..=90.0, " %");
        });
    });
}

/// The Frame group: shape, corner radius (Rectangle only), border width and color (PiP only).
fn frame_group(ui: &mut Ui, pip: &mut PipConfig, lock: Lock, enabled: bool, changed: &mut bool) {
    inspector_group(ui, |ui| {
        lock.scope(ui, Control::Profile, enabled, |ui| {
            inspector_row(
                ui,
                "Shape",
                |ui| {
                    *changed |= choice(ui, "pip-shape", &mut pip.shape, &SHAPES);
                },
                None,
            );
            ui.add_enabled_ui(pip.shape == PipShape::Rectangle, |ui| {
                *changed |= slider_row(ui, "Corner radius", &mut pip.radius_px, 0.0..=100.0, " px");
            });
            *changed |= slider_row(ui, "Border width", &mut pip.border_px, 0.0..=40.0, " px");
            inspector_row(
                ui,
                "Border color",
                |ui| {
                    *changed |= ui.color_edit_button_srgb(&mut pip.border_color).changed();
                },
                None,
            );
        });
    });
}

/// The View group's three-way segmented control: equal cells filling the row
/// width, each segment's full view name as its tooltip; true when the view changed.
fn view_choice(ui: &mut Ui, value: &mut LayoutKind) -> bool {
    let labels: [&str; 3] = VIEWS.map(|(_, label)| label);
    let mut index = VIEWS.iter().position(|(kind, _)| *kind == *value).unwrap_or(0);
    let before = index;
    segmented_filled(ui, "layout-view", &mut index, &labels, &VIEW_NAMES);
    if index != before
        && let Some((kind, _)) = VIEWS.get(index)
    {
        *value = *kind;
        return true;
    }
    false
}

/// One segmented control over a small copyable enum; true when the choice changed.
pub(crate) fn choice<T: Copy + PartialEq>(ui: &mut Ui, id_salt: &'static str, value: &mut T, options: &[(T, &'static str)]) -> bool {
    let mut index = options.iter().position(|(kind, _)| *kind == *value).unwrap_or(0);
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    let before = index;
    segmented(ui, id_salt, &mut index, &labels);
    if index != before
        && let Some((kind, _)) = options.get(index)
    {
        *value = *kind;
        return true;
    }
    false
}

/// One inspector row with a slider and the value on the right; true when the slider changed.
fn slider_row(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>, suffix: &str) -> bool {
    let text = format!("{value}{suffix}");
    let mut changed = false;
    inspector_row(
        ui,
        label,
        |ui| {
            changed = ui_kit::slider(ui, value, range, SliderKind::Accent).changed();
        },
        Some(&text),
    );
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_corner_has_its_own_label() {
        let labels: Vec<_> = CORNERS.iter().map(|corner| corner_label(*corner)).collect();
        for (index, label) in labels.iter().enumerate() {
            assert!(!labels[index + 1..].contains(label), "{label} is listed twice");
        }
    }
}
