//! The Webcam section of the window: camera, mode, live camera picture and the camera controls.

use capture::CameraFormat;
use egui::{Button, Checkbox, ComboBox, Grid, Slider, Ui};

use crate::config::ControlSetting;
use crate::webcam::WebcamHost;

/// The widest the camera picture is drawn, in points, so it does not grow as large as the screen preview.
const PREVIEW_MAX_WIDTH: f32 = 360.0;

/// The width of the control sliders, in points.
const SLIDER_WIDTH: f32 = 220.0;

/// Draws the Webcam section. Everything except the camera controls is read-only while recording.
pub fn show(ui: &mut Ui, host: &mut WebcamHost, recording: bool) {
    ui.heading("Webcam");
    let mut enabled = host.settings().is_some();
    if ui.add_enabled(!recording, Checkbox::new(&mut enabled, "Enable")).changed() {
        if enabled {
            host.enable();
        } else {
            host.disable();
        }
    }
    if host.settings().is_none() {
        return;
    }
    ui.add_enabled_ui(!recording, |ui| {
        camera_row(ui, host);
        format_row(ui, host);
    });
    ui.add_space(4.0);
    ui.scope(|ui| {
        ui.set_max_width(PREVIEW_MAX_WIDTH);
        host.show_preview(ui);
    });
    ui.add_space(4.0);
    controls(ui, host);
}

/// The camera dropdown with its Refresh button.
fn camera_row(ui: &mut Ui, host: &mut WebcamHost) {
    let Some(settings) = host.settings() else { return };
    let present = host.cameras().iter().any(|camera| camera.id == settings.camera_id);
    let selected = if settings.camera_name.is_empty() {
        if host.is_listing() { "Looking for cameras...".to_owned() } else { "No camera".to_owned() }
    } else if present || host.is_listing() {
        settings.camera_name.clone()
    } else {
        format!("{} (not found)", settings.camera_name)
    };
    let current_id = settings.camera_id.clone();
    let mut picked: Option<String> = None;
    ui.horizontal(|ui| {
        ui.label("Camera");
        ComboBox::from_id_salt("webcam-camera").selected_text(selected).width(260.0).show_ui(ui, |ui| {
            for camera in host.cameras() {
                if ui.selectable_label(camera.id == current_id, &camera.name).clicked() {
                    picked = Some(camera.id.clone());
                }
            }
        });
        if ui.add_enabled(!host.is_listing(), Button::new("Refresh")).clicked() {
            host.refresh_cameras();
        }
    });
    if let Some(id) = picked {
        host.select_camera(&id);
    }
}

/// The mode dropdown of the selected camera.
fn format_row(ui: &mut Ui, host: &mut WebcamHost) {
    let Some(settings) = host.settings() else { return };
    let saved = format!("{} {}x{} @ {} fps", settings.format.subtype, settings.format.width, settings.format.height, fps_text(f64::from(settings.format.fps)));
    let mut picked: Option<CameraFormat> = None;
    ui.horizontal(|ui| {
        ui.label("Format");
        ComboBox::from_id_salt("webcam-format").selected_text(saved).width(260.0).show_ui(ui, |ui| {
            for format in host.formats() {
                if ui.selectable_label(host.is_current_format(format), format_label(format)).clicked() {
                    picked = Some(*format);
                }
            }
        });
    });
    if let Some(format) = picked {
        host.select_format(&format);
    }
}

/// The sliders with their Auto boxes, and the Lock and Reset buttons. These stay live while recording.
fn controls(ui: &mut Ui, host: &mut WebcamHost) {
    ui.label("Camera controls");
    let infos = host.controls().to_vec();
    if infos.is_empty() {
        ui.weak("This camera has no adjustable controls, or it is not running.");
        return;
    }
    Grid::new("webcam-controls").num_columns(3).show(ui, |ui| {
        ui.spacing_mut().slider_width = SLIDER_WIDTH;
        for info in &infos {
            ui.label(info.prop.label());
            let mut value = info.current.value;
            let mut auto = info.current.auto;
            let slider = Slider::new(&mut value, info.range.min..=info.range.max).step_by(f64::from(info.range.step.max(1)));
            if ui.add_enabled(!auto, slider).changed() {
                host.set_control(info.prop, ControlSetting::Manual(value));
            }
            if info.range.auto_capable {
                if ui.checkbox(&mut auto, "Auto").changed() {
                    host.set_control(info.prop, if auto { ControlSetting::Auto } else { ControlSetting::Manual(info.current.value) });
                }
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Lock current values").clicked() {
            host.lock_current();
        }
        if ui.button("Reset to defaults").clicked() {
            host.reset_defaults();
        }
    });
}

/// A frame rate without trailing zeros: 30, 29.97, 59.94.
fn fps_text(fps: f64) -> String {
    let text = format!("{fps:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// How a camera mode is listed in the dropdown.
fn format_label(format: &CameraFormat) -> String {
    format!("{} {}x{} @ {} fps", format.subtype, format.width, format.height, fps_text(format.fps()))
}

#[cfg(test)]
mod tests {
    use capture::CameraSubtype;

    use super::*;

    #[test]
    fn frame_rates_lose_their_trailing_zeros() {
        assert_eq!(fps_text(30.0), "30");
        assert_eq!(fps_text(29.97), "29.97");
        assert_eq!(fps_text(59.94), "59.94");
        assert_eq!(fps_text(7.5), "7.5");
        assert_eq!(fps_text(0.0), "0");
    }

    #[test]
    fn a_mode_is_listed_with_its_format_size_and_rate() {
        let format = CameraFormat { subtype: CameraSubtype::Mjpg, width: 1920, height: 1080, fps_num: 30000, fps_den: 1001 };
        assert_eq!(format_label(&format), "MJPG 1920x1080 @ 29.97 fps");
    }
}
