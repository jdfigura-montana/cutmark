//! The Setup right panel's tools: the remembered [`Tool`] choice and the Camera tool's panels
//! (spec "Setup mode": the right panel holds the tool strip and the open tool's panel).
//!
//! The Key tool's groups live in `key_ui`; Layout, Audio and Output show the parked column in
//! `app` until R4 and R5 take their rows.

use std::time::Duration;

use capture::{CameraFormat, ControlInfo};
use egui::{Button, Color32, ComboBox, DragValue, ProgressBar, Ui};
use media_io::SyncReport;
use serde::{Deserialize, Serialize};
use ui_kit::icons;
use ui_kit::tokens::{size, space};
use ui_kit::{SliderKind, icon_button, inspector_group, inspector_row, switch};

use crate::config::ControlSetting;
use crate::engine::bus::{BusSender, Command, CommandKind};
use crate::engine::controller::{ControllerState, Phase};
use crate::key_ui::{self, KeyUiState};
use crate::profiles::ProfileStore;
use crate::webcam::WebcamHost;
use crate::webcam_ui;

/// The color of the "Within 1 frame" verdict.
const SYNC_GOOD: Color32 = Color32::from_rgb(0x5C, 0xB8, 0x5C);

/// The color of the "Off by N frames" verdict and of a failed sync test.
const SYNC_WARN: Color32 = Color32::from_rgb(0xE6, 0xB4, 0x3C);

/// How often the window redraws while a sync test runs, so its progress moves.
const SYNC_REPAINT: Duration = Duration::from_millis(100);

/// The tools of the Setup right panel; the open one is remembered in the settings (spec "Mode rules").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    /// The camera device, its picture, its image controls and the sync test.
    Camera,
    /// The chroma key, garbage crop and mask rectangle over the camera picture.
    Key,
    /// The layout of screen and camera; the parked column until R4.
    #[default]
    Layout,
    /// The audio tracks; the parked column until R5.
    Audio,
    /// The encode and format settings; the parked column until R4.
    Output,
}

impl Tool {
    /// The tools in the order the tool strip shows them.
    pub const ALL: [Tool; 5] = [Self::Camera, Self::Key, Self::Layout, Self::Audio, Self::Output];

    /// The label the tool strip shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Camera => "Camera",
            Self::Key => "Key",
            Self::Layout => "Layout",
            Self::Audio => "Audio",
            Self::Output => "Output",
        }
    }

    /// The phosphor regular icon the tool strip shows.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Camera => icons::regular::CAMERA,
            Self::Key => icons::regular::KEY,
            Self::Layout => icons::regular::LAYOUT,
            Self::Audio => icons::regular::MICROPHONE,
            Self::Output => icons::regular::EXPORT,
        }
    }
}

/// The tool strip's items in display order: `(tool, icon, label)`.
pub fn strip_items() -> [(Tool, &'static str, &'static str); 5] {
    Tool::ALL.map(|tool| (tool, tool.icon(), tool.label()))
}

/// The Camera tool: the Device group, the camera picture, the Image group and the Sync group.
///
/// Everything except the image controls and the sync result is read-only while recording. The
/// camera picture is the eyedropper's target here too: while the eyedropper is armed in the
/// Key tool, a click on it sets the key color.
pub fn camera_panel(
    ui: &mut Ui,
    host: &mut WebcamHost,
    store: &mut ProfileStore,
    recording: bool,
    state: &ControllerState,
    bus: &BusSender,
    key_state: &mut KeyUiState,
) {
    device_group(ui, host, store, recording);
    if host.settings(store).is_none() {
        return;
    }
    let picked = webcam_ui::camera_picture(ui, host, key_state.eyedropper);
    key_ui::take_pick(store, &state.active_profile, key_state, picked.as_ref());
    ui.add_space(space::SCALE_8);
    image_group(ui, host, store);
    ui.add_space(space::SCALE_8);
    sync_group(ui, host, store, recording, state, bus);
}

/// The Device group: the Enable switch, the camera dropdown with its Refresh icon button and the format dropdown.
fn device_group(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore, recording: bool) {
    inspector_group(ui, |ui| {
        let enabled = host.settings(store).is_some();
        ui.add_enabled_ui(!recording, |ui| {
            inspector_row(
                ui,
                "Enable",
                |ui| {
                    let mut on = enabled;
                    if switch(ui, &mut on).changed() {
                        if on {
                            host.enable(store);
                        } else {
                            host.disable(store);
                        }
                    }
                },
                None,
            );
        });
        if host.settings(store).is_some() {
            ui.add_enabled_ui(!recording, |ui| {
                camera_row(ui, host, store);
                format_row(ui, host, store);
            });
        }
    });
}

/// The camera dropdown with its Refresh icon button, as one inspector row.
fn camera_row(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore) {
    let Some(settings) = host.settings(store) else { return };
    let present = host.cameras().iter().any(|camera| camera.id == settings.camera_id);
    let is_listing = host.is_listing();
    let current_id = settings.camera_id.clone();
    let selected = if settings.camera_name.is_empty() {
        if is_listing { "Looking for cameras...".to_owned() } else { "No camera".to_owned() }
    } else if present || is_listing {
        settings.camera_name.clone()
    } else {
        format!("{} (not found)", settings.camera_name)
    };
    let mut picked: Option<String> = None;
    inspector_row(
        ui,
        "Camera",
        |ui| {
            let width = (ui.available_width() - size::ICON_BUTTON).max(60.0);
            ComboBox::from_id_salt("webcam-camera").selected_text(selected).width(width).show_ui(ui, |ui| {
                for camera in host.cameras() {
                    if ui.selectable_label(camera.id == current_id, &camera.name).clicked() {
                        picked = Some(camera.id.clone());
                    }
                }
            });
            ui.add_enabled_ui(!host.is_listing(), |ui| {
                if icon_button(ui, icons::regular::ARROWS_CLOCKWISE, "Refresh").clicked() {
                    host.refresh_cameras();
                }
            });
        },
        None,
    );
    if let Some(id) = picked {
        host.select_camera(store, &id);
    }
}

/// The format dropdown of the selected camera, as one inspector row.
fn format_row(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore) {
    let Some(settings) = host.settings(store) else { return };
    let selected = format!(
        "{} {}x{} @ {} fps",
        settings.format.subtype,
        settings.format.width,
        settings.format.height,
        webcam_ui::fps_text(f64::from(settings.format.fps))
    );
    let mut picked: Option<CameraFormat> = None;
    inspector_row(
        ui,
        "Format",
        |ui| {
            ComboBox::from_id_salt("webcam-format").selected_text(selected).width(ui.available_width()).show_ui(ui, |ui| {
                for format in host.formats(store) {
                    if ui.selectable_label(host.is_current_format(store, format), webcam_ui::format_label(format)).clicked() {
                        picked = Some(*format);
                    }
                }
            });
        },
        None,
    );
    if let Some(format) = picked {
        host.select_format(store, &format);
    }
}

/// The Image group: one row per supported camera control, then Lock and Reset.
///
/// The controls stay live while recording, like today.
fn image_group(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore) {
    let infos = host.controls().to_vec();
    inspector_group(ui, |ui| {
        if infos.is_empty() {
            ui.add_space(space::SCALE_4);
            ui.weak("This camera has no adjustable controls, or it is not running.");
            ui.add_space(space::SCALE_4);
            return;
        }
        for info in &infos {
            control_row(ui, host, store, info);
        }
        inspector_row(
            ui,
            "",
            |ui| {
                if ui.button("Lock current values").clicked() {
                    host.lock_current(store);
                }
                if ui.button("Reset to defaults").clicked() {
                    host.reset_defaults(store);
                }
            },
            None,
        );
    });
}

/// One camera control: its slider (disabled while the camera drives it) with an Auto switch where the camera offers one.
fn control_row(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore, info: &ControlInfo) {
    let label = info.prop.label().to_owned();
    let range = info.range.min as f32..=info.range.max as f32;
    let (min, max) = (*range.start(), *range.end());
    let step = info.range.step.max(1) as f32;
    let mut value = info.current.value as f32;
    let mut auto = info.current.auto;
    let auto_capable = info.range.auto_capable;
    let current = info.current.value;
    let prop = info.prop;
    let value_text = format!("{value}");
    inspector_row(
        ui,
        &label,
        move |ui| {
            ui.add_enabled_ui(!auto, |ui| {
                if ui_kit::slider(ui, &mut value, range, SliderKind::Accent).changed() {
                    let snapped = (min + ((value - min) / step).round() * step).clamp(min, max);
                    host.set_control(store, prop, ControlSetting::Manual(snapped.round() as i32));
                }
            });
            if auto_capable && switch(ui, &mut auto).changed() {
                host.set_control(store, prop, if auto { ControlSetting::Auto } else { ControlSetting::Manual(current) });
            }
        },
        Some(&value_text),
    );
}

/// The Sync group: the camera offset, the sync test with its hint and progress, and the result.
fn sync_group(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore, recording: bool, state: &ControllerState, bus: &BusSender) {
    let Some(settings) = host.settings(store).cloned() else { return };
    let iso = settings.iso;
    let idle = state.phase == Phase::Idle && state.preflight.is_none() && state.sync_test.is_none();
    inspector_group(ui, |ui| {
        ui.add_enabled_ui(!recording, |ui| {
            let mut offset = settings.offset_ms;
            let offset_text = format!("{offset} ms");
            inspector_row(
                ui,
                "Camera offset",
                |ui| {
                    if ui.add(DragValue::new(&mut offset).range(-500..=500).suffix(" ms")).changed() {
                        store.edit(&state.active_profile, |profile| {
                            if let Some(webcam) = &mut profile.webcam {
                                webcam.offset_ms = offset;
                            }
                        });
                    }
                },
                Some(&offset_text),
            );
        });
        inspector_row(
            ui,
            "Sync test",
            |ui| {
                if ui.add_enabled(idle && iso, Button::new("Sync test (12 s)")).clicked() {
                    let command = Command::new(CommandKind::SyncTest, capture::qpc_now());
                    if bus.send(command).is_err() {
                        log::warn!("the sync test was not started because the controller has ended");
                    }
                }
            },
            None,
        );
        ui.add_space(space::SCALE_4);
        ui.weak("Face the camera; dim the room if it fails.");
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
    });
}

/// The measured offset, the verdict, and the button that saves the opposite offset to the profile.
fn sync_result(ui: &mut Ui, host: &WebcamHost, store: &mut ProfileStore, report: &SyncReport, profile: &str) {
    ui.label(format!("Measured offset {:+.0} ms (spread {:.0} ms over {} flashes)", report.median_ms, report.spread_ms, report.offsets_ms.len()));
    // The test records with the offset at zero, so what the current offset leaves over is the raw median plus that offset.
    let residual_ms = report.median_ms + f64::from(host.settings(store).map_or(0, |settings| settings.offset_ms));
    ui.label(format!("Residual with current offset: {residual_ms:+.0} ms"));
    match webcam_ui::frames_off(residual_ms, report.main_fps) {
        None => {
            ui.colored_label(SYNC_GOOD, "Within 1 frame");
        }
        Some(frames) => {
            ui.colored_label(SYNC_WARN, format!("Off by {frames} frame{}", if frames == 1 { "" } else { "s" }));
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Apply as camera offset").clicked() {
            let offset = webcam_ui::applied_offset(report.median_ms);
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

#[cfg(test)]
mod tests {
    use super::Tool;
    use crate::config::Settings;

    #[test]
    fn tool_selection_persists_through_a_settings_save_and_load() {
        let mut settings = Settings::default();
        assert_eq!(settings.tool, Tool::Layout, "a fresh install opens on Layout");
        settings.tool = Tool::Key;
        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.tool, Tool::Key, "the saved tool comes back from the settings file");
        // A settings file written before the tools existed opens on the default tool.
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.tool, Tool::Layout);
    }
}
