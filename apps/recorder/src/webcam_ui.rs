//! The Webcam section of the window: camera, mode, live camera picture and the camera controls.

use std::time::Duration;

use capture::CameraFormat;
use egui::{Button, Checkbox, Color32, ComboBox, Grid, ProgressBar, Slider, Ui};
use media_io::SyncReport;

use crate::config::{ControlSetting, WebcamSettings};
use crate::engine::bus::{BusSender, Command, CommandKind};
use crate::engine::controller::{ControllerState, Phase};
use crate::profiles::ProfileStore;
use crate::webcam::WebcamHost;

/// The widest the camera picture is drawn, in points, so it does not grow as large as the screen preview.
const PREVIEW_MAX_WIDTH: f32 = 360.0;

/// The width of the control sliders, in points.
const SLIDER_WIDTH: f32 = 220.0;

/// The color of the "Within 1 frame" verdict.
const SYNC_GOOD: Color32 = Color32::from_rgb(0x5C, 0xB8, 0x5C);

/// The color of the "Off by N frames" verdict and of a failed sync test.
const SYNC_WARN: Color32 = Color32::from_rgb(0xE6, 0xB4, 0x3C);

/// How often the window redraws while a sync test runs, so its progress moves.
const SYNC_REPAINT: Duration = Duration::from_millis(100);

/// Draws the Webcam section. Everything except the camera controls is read-only while recording.
///
/// The controller `state` and the command `bus` drive the camera sync test at the bottom of the section.
pub fn show(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore, recording: bool, state: &ControllerState, bus: &BusSender) {
    ui.heading("Webcam");
    let mut enabled = host.settings(store).is_some();
    if ui.add_enabled(!recording, Checkbox::new(&mut enabled, "Enable")).changed() {
        if enabled {
            host.enable(store);
        } else {
            host.disable(store);
        }
    }
    if host.settings(store).is_none() {
        return;
    }
    ui.add_enabled_ui(!recording, |ui| {
        camera_row(ui, host, store);
        format_row(ui, host, store);
    });
    ui.add_space(4.0);
    ui.scope(|ui| {
        ui.set_max_width(PREVIEW_MAX_WIDTH);
        host.show_preview(ui);
    });
    ui.add_space(4.0);
    controls(ui, host, store);
    ui.add_space(4.0);
    sync_test(ui, host, store, state, bus);
}

/// The camera dropdown with its Refresh button.
fn camera_row(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore) {
    let Some(settings) = host.settings(store) else { return };
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
        host.select_camera(store, &id);
    }
}

/// The mode dropdown of the selected camera.
fn format_row(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore) {
    let Some(settings) = host.settings(store) else { return };
    let saved = format!("{} {}x{} @ {} fps", settings.format.subtype, settings.format.width, settings.format.height, fps_text(f64::from(settings.format.fps)));
    let mut picked: Option<CameraFormat> = None;
    ui.horizontal(|ui| {
        ui.label("Format");
        ComboBox::from_id_salt("webcam-format").selected_text(saved).width(260.0).show_ui(ui, |ui| {
            for format in host.formats(store) {
                if ui.selectable_label(host.is_current_format(store, format), format_label(format)).clicked() {
                    picked = Some(*format);
                }
            }
        });
    });
    if let Some(format) = picked {
        host.select_format(store, &format);
    }
}

/// The sliders with their Auto boxes, and the Lock and Reset buttons. These stay live while recording.
fn controls(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore) {
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
                host.set_control(store, info.prop, ControlSetting::Manual(value));
            }
            if info.range.auto_capable {
                if ui.checkbox(&mut auto, "Auto").changed() {
                    host.set_control(store, info.prop, if auto { ControlSetting::Auto } else { ControlSetting::Manual(info.current.value) });
                }
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Lock current values").clicked() {
            host.lock_current(store);
        }
        if ui.button("Reset to defaults").clicked() {
            host.reset_defaults(store);
        }
    });
}

/// The Sync test button with its hint, the progress while it runs, and the result with the button that applies it.
fn sync_test(ui: &mut Ui, host: &WebcamHost, store: &mut ProfileStore, state: &ControllerState, bus: &BusSender) {
    ui.label("Camera sync");
    let iso = host.settings(store).is_some_and(|settings| settings.iso);
    let idle = state.phase == Phase::Idle && state.preflight.is_none() && state.sync_test.is_none();
    ui.horizontal(|ui| {
        if ui.add_enabled(idle && iso, Button::new("Sync test (12 s)")).clicked() {
            let command = Command::new(CommandKind::SyncTest, capture::qpc_now());
            if bus.send(command).is_err() {
                log::warn!("the sync test was not started because the controller has ended");
            }
        }
        ui.weak("Face the camera; dim the room if it fails.");
    });
    if !iso {
        ui.weak("Turn on the camera file to run the sync test.");
    }
    if let Some(run) = &state.sync_test {
        ui.add(ProgressBar::new(run.progress).text(run.status));
        ui.ctx().request_repaint_after(SYNC_REPAINT);
    }
    match &state.sync_result {
        Some(Ok(report)) => sync_result(ui, host, store, report, &state.active_profile),
        Some(Err(message)) => {
            ui.colored_label(SYNC_WARN, message);
        }
        None => {}
    }
}

/// The measured offset, the verdict, and the button that saves the opposite offset to the profile.
fn sync_result(ui: &mut Ui, host: &WebcamHost, store: &mut ProfileStore, report: &SyncReport, profile: &str) {
    ui.label(format!("Measured offset {:+.0} ms (spread {:.0} ms over {} flashes)", report.median_ms, report.spread_ms, report.offsets_ms.len()));
    // The test records with the offset at zero, so what the current offset leaves over is the raw median plus that offset.
    let residual_ms = report.median_ms + f64::from(host.settings(store).map_or(0, |settings| settings.offset_ms));
    ui.label(format!("Residual with current offset: {residual_ms:+.0} ms"));
    match frames_off(residual_ms, report.main_fps) {
        None => {
            ui.colored_label(SYNC_GOOD, "Within 1 frame");
        }
        Some(frames) => {
            ui.colored_label(SYNC_WARN, format!("Off by {frames} frame{}", if frames == 1 { "" } else { "s" }));
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Apply as camera offset").clicked() {
            let offset = applied_offset(report.median_ms);
            store.edit(profile, |profile| {
                if let Some(settings) = &mut profile.webcam {
                    settings.offset_ms = offset;
                }
            });
            store.flush();
        }
        if let Some(settings) = host.settings(store) {
            ui.weak(format!("Camera offset is {} ms", settings.offset_ms));
        }
    });
}

/// How many whole frames a residual is off by, or none when it is within one frame either way.
fn frames_off(residual_ms: f64, fps: f64) -> Option<u32> {
    let frames = residual_ms * fps / 1000.0;
    (frames.abs() > 1.0).then(|| frames.abs().round() as u32)
}

/// The offset that cancels a measured median: its opposite, rounded, and limited to the range the window allows.
fn applied_offset(median_ms: f64) -> i32 {
    WebcamSettings { offset_ms: (-median_ms).round() as i32, ..WebcamSettings::default() }.offset_clamped()
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
    fn the_applied_offset_is_the_opposite_of_the_median_within_the_limit() {
        assert_eq!(applied_offset(83.4), -83);
        assert_eq!(applied_offset(-120.0), 120);
        assert_eq!(applied_offset(0.4), 0);
        assert_eq!(applied_offset(900.0), -500);
        assert_eq!(applied_offset(-900.0), 500);
    }

    #[test]
    fn the_residual_is_judged_in_frames_at_the_main_rate() {
        assert_eq!(frames_off(15.0, 60.0), None);
        assert_eq!(frames_off(-16.0, 60.0), None);
        assert_eq!(frames_off(100.0, 60.0), Some(6));
        assert_eq!(frames_off(-70.0, 30.0), Some(2));
    }

    #[test]
    fn a_mode_is_listed_with_its_format_size_and_rate() {
        let format = CameraFormat { subtype: CameraSubtype::Mjpg, width: 1920, height: 1080, fps_num: 30000, fps_den: 1001 };
        assert_eq!(format_label(&format), "MJPG 1920x1080 @ 29.97 fps");
    }
}
