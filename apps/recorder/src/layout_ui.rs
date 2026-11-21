//! The Layout section of the window: which pictures the composite shows and how the camera looks.
//!
//! The settings live in the active profile in the shared [`ProfileStore`]. A change goes through the store,
//! which saves it later; the compose thread picks it up on the next frame, so the preview follows live.
//! The section is read-only while a take is recording.

use std::ops::RangeInclusive;

use crate::config::{AnchorCorner, LayoutKind, PipConfig, PipShape};
use crate::profiles::ProfileStore;

/// The corners in the order the picker lists them.
const CORNERS: [AnchorCorner; 4] = [AnchorCorner::TopLeft, AnchorCorner::TopRight, AnchorCorner::BottomLeft, AnchorCorner::BottomRight];

/// The text of a corner in the picker.
fn corner_label(corner: AnchorCorner) -> &'static str {
    match corner {
        AnchorCorner::TopLeft => "Top left",
        AnchorCorner::TopRight => "Top right",
        AnchorCorner::BottomLeft => "Bottom left",
        AnchorCorner::BottomRight => "Bottom right",
    }
}

/// Draws the Layout section for the active profile; `recording` makes it read-only.
pub fn show(ui: &mut egui::Ui, store: &mut ProfileStore, profile_name: &str, recording: bool) {
    ui.heading("Layout");
    let Some(profile) = store.get(profile_name) else {
        ui.label("No profile is active.");
        return;
    };
    let mut layout = profile.layout;
    let mut pip = profile.pip.clone();
    let mut background = profile.cam_only_bg;
    let webcam_on = profile.webcam.is_some();
    let mut changed = false;
    ui.add_enabled_ui(!recording, |ui| {
        ui.horizontal(|ui| {
            changed |= ui.selectable_value(&mut layout, LayoutKind::ScreenOnly, "Screen only").changed();
            changed |= ui.selectable_value(&mut layout, LayoutKind::Pip, "Picture in picture").changed();
            changed |= ui.selectable_value(&mut layout, LayoutKind::CamOnly, "Cam only").changed();
        });
        match layout {
            LayoutKind::ScreenOnly => {}
            LayoutKind::Pip => changed |= pip_controls(ui, &mut pip),
            LayoutKind::CamOnly => {
                ui.horizontal(|ui| {
                    ui.label("Background");
                    changed |= ui.color_edit_button_srgb(&mut background).changed();
                });
            }
        }
    });
    if recording {
        ui.label("The layout is fixed while recording.");
    }
    if !webcam_on && layout != LayoutKind::ScreenOnly {
        ui.label("The webcam is off, so only the screen is shown.");
    }
    if changed {
        store.edit(profile_name, |profile| {
            profile.layout = layout;
            profile.pip = pip;
            profile.cam_only_bg = background;
        });
    }
}

/// Draws the picture in picture controls; true when one of them changed.
fn pip_controls(ui: &mut egui::Ui, pip: &mut PipConfig) -> bool {
    let mut changed = false;
    egui::Grid::new("pip-controls").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Corner");
        egui::ComboBox::from_id_salt("pip-corner").selected_text(corner_label(pip.anchor)).show_ui(ui, |ui| {
            for corner in CORNERS {
                changed |= ui.selectable_value(&mut pip.anchor, corner, corner_label(corner)).changed();
            }
        });
        ui.end_row();
        changed |= slider_row(ui, "X offset", &mut pip.offset_x_pct, 0.0..=50.0, " %");
        changed |= slider_row(ui, "Y offset", &mut pip.offset_y_pct, 0.0..=50.0, " %");
        changed |= slider_row(ui, "Scale", &mut pip.scale_pct, 5.0..=100.0, " %");
        changed |= slider_row(ui, "Crop left", &mut pip.crop_left_pct, 0.0..=90.0, " %");
        changed |= slider_row(ui, "Crop right", &mut pip.crop_right_pct, 0.0..=90.0, " %");
        changed |= slider_row(ui, "Crop top", &mut pip.crop_top_pct, 0.0..=90.0, " %");
        changed |= slider_row(ui, "Crop bottom", &mut pip.crop_bottom_pct, 0.0..=90.0, " %");
        changed |= slider_row(ui, "Border width", &mut pip.border_px, 0.0..=40.0, " px");
        ui.label("Border color");
        changed |= ui.color_edit_button_srgb(&mut pip.border_color).changed();
        ui.end_row();
        ui.label("Shape");
        ui.horizontal(|ui| {
            changed |= ui.selectable_value(&mut pip.shape, PipShape::Rectangle, "Rectangle").changed();
            changed |= ui.selectable_value(&mut pip.shape, PipShape::Circle, "Circle").changed();
        });
        ui.end_row();
        if pip.shape == PipShape::Rectangle {
            changed |= slider_row(ui, "Corner radius", &mut pip.radius_px, 0.0..=100.0, " px");
        }
    });
    changed
}

/// Draws one labelled slider as a row of the grid; true when it changed.
fn slider_row(ui: &mut egui::Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>, suffix: &str) -> bool {
    ui.label(label);
    let response = ui.add(egui::Slider::new(value, range).suffix(suffix));
    ui.end_row();
    response.changed()
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
