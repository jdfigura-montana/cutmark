//! The Setup right panel's tools: the remembered [`Tool`] choice and the Camera tool's panels
//! (spec "Setup mode": the right panel holds the tool strip and the open tool's panel).
//!
//! The Key tool's groups live in `key_ui`, the Layout tool's in `layout_ui` and `background_ui`,
//! the Output tool's in `profile_ui` and `webcam_ui`, and the Audio tool's in `tracks_ui`.

use std::time::Duration;

use capture::{CameraFormat, ControlInfo};
use egui::{Align, Align2, Button, Color32, ComboBox, DragValue, Label, Layout, ProgressBar, Rect, RichText, Ui, UiBuilder, pos2};
use media_io::SyncReport;
use serde::{Deserialize, Serialize};
use ui_kit::icons;
use ui_kit::tokens::{size, space, typography};
use ui_kit::{SliderKind, Theme, icon_button, inspector_group, inspector_row, switch};

use crate::config::ControlSetting;
use crate::data_text;
use crate::engine::bus::{BusSender, Command, CommandKind};
use crate::engine::controller::{ControllerState, Phase};
use crate::key_ui::{self, KeyUiState};
use crate::profiles::ProfileStore;
use crate::webcam::WebcamHost;
use crate::webcam_ui;

/// The color of the "Within 1 frame" verdict (`sem.success`).
const SYNC_GOOD: Color32 = ui_kit::tokens::color::SEM_SUCCESS;

/// The color of the "Off by N frames" verdict and of a failed sync test (`sem.meter.warn`).
const SYNC_WARN: Color32 = ui_kit::tokens::color::SEM_METER_WARN;

/// Width of a slider row's value column, in points.
const VALUE_COLUMN: f32 = 56.0;

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
    /// The layout of screen and camera, and the background behind it.
    #[default]
    Layout,
    /// The audio tracks; the panel lives in `tracks_ui`.
    Audio,
    /// The profile, its encode and format settings, and the camera ISO file.
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
///
/// Also drawn in the Sources panel's camera popover (spec "Setup mode": clicking the camera row
/// opens the same picker).
pub(crate) fn camera_row(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore) {
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
            let width = (ui.available_width() - size::ICON_BUTTON - ui.spacing().item_spacing.x).max(60.0);
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
///
/// Also drawn in the Sources panel's camera popover, under the camera dropdown.
pub(crate) fn format_row(ui: &mut Ui, host: &mut WebcamHost, store: &mut ProfileStore) {
    let Some(settings) = host.settings(store) else { return };
    let selected = webcam_ui::mode_text(&settings.format.subtype, settings.format.width, settings.format.height, f64::from(settings.format.fps));
    let mut picked: Option<CameraFormat> = None;
    inspector_row(
        ui,
        "Format",
        |ui| {
            let selected = RichText::new(selected).monospace();
            ComboBox::from_id_salt("webcam-format").selected_text(selected).width(ui.available_width()).truncate().show_ui(ui, |ui| {
                for format in host.formats(store) {
                    let entry = RichText::new(webcam_ui::format_label(format)).monospace();
                    if ui.selectable_label(host.is_current_format(store, format), entry).clicked() {
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

/// One inspector row with a slider, laid out in fixed columns so nothing runs into the value: the
/// value in a [`VALUE_COLUMN`] wide column at the right, the switch in its own column left of it,
/// and the slider in the rest. `add` gets the slider column and the switch column; the switch
/// column stays reserved when it is left empty, so the sliders of a group end together.
pub(crate) fn slider_row(ui: &mut Ui, label: &str, value: &str, add: impl FnOnce(&mut Ui, &mut Ui)) {
    inspector_row(
        ui,
        label,
        |ui| {
            let area = ui.max_rect();
            // With no value of its own, inspector_row ends the control area one gap short of the text inset.
            let right = area.right() + space::SCALE_8;
            let value_left = right - VALUE_COLUMN;
            let switch_right = value_left - space::SCALE_8;
            let switch_left = switch_right - size::SWITCH_WIDTH;
            let slider_right = (switch_left - space::SCALE_8).max(area.left());
            let column = |ui: &mut Ui, left: f32, right: f32| {
                let rect = Rect::from_min_max(pos2(left, area.top()), pos2(right, area.bottom()));
                ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)))
            };
            let mut slider_ui = column(ui, area.left(), slider_right);
            let mut switch_ui = column(ui, switch_left, switch_right);
            add(&mut slider_ui, &mut switch_ui);
            let color = Theme::get(ui.ctx()).neutrals.text_primary;
            let font = ui_kit::fonts::mono(typography::VALUE_SIZE);
            let galley = ui.painter().layout_job(ui_kit::fonts::tracked(value, font, color, ui_kit::fonts::MONO_LETTER_SPACING_EM));
            let place = Align2::RIGHT_CENTER.anchor_size(pos2(right, area.center().y), galley.size());
            ui.painter().galley(place.min, galley, color);
        },
        None,
    );
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
    slider_row(ui, &label, &value_text, move |slider_ui, switch_ui| {
        slider_ui.add_enabled_ui(!auto, |ui| {
            if ui_kit::slider(ui, &mut value, range, SliderKind::Accent).changed() {
                let snapped = (min + ((value - min) / step).round() * step).clamp(min, max);
                host.set_control(store, prop, ControlSetting::Manual(snapped.round() as i32));
            }
        });
        if auto_capable && switch(switch_ui, &mut auto).changed() {
            host.set_control(store, prop, if auto { ControlSetting::Auto } else { ControlSetting::Manual(current) });
        }
    });
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
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let median = format!("{:+.0} ms", report.median_ms);
    let spread = format!("{:.0} ms", report.spread_ms);
    let flashes = report.offsets_ms.len().to_string();
    data_line(ui, &format!("Measured offset {median} (spread {spread} over {flashes} flashes)"), &[&median, &spread, &flashes], neutrals.text_primary);
    // The test records with the offset at zero, so what the current offset leaves over is the raw median plus that offset.
    let residual_ms = report.median_ms + f64::from(host.settings(store).map_or(0, |settings| settings.offset_ms));
    let residual = format!("{residual_ms:+.0} ms");
    data_line(ui, &format!("Residual with current offset: {residual}"), &[&residual], neutrals.text_primary);
    match webcam_ui::frames_off(residual_ms, report.main_fps) {
        None => {
            ui.colored_label(SYNC_GOOD, "Within 1 frame");
        }
        Some(frames) => {
            let count = frames.to_string();
            data_line(ui, &format!("Off by {count} frame{}", if frames == 1 { "" } else { "s" }), &[&count], SYNC_WARN);
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
            let offset = format!("{} ms", settings.offset_ms);
            data_line(ui, &format!("Camera offset is {offset}"), &[&offset], neutrals.text_tertiary);
        }
    });
}

/// One wrapping line of the sync result, with its numbers in Martian Mono.
fn data_line(ui: &mut Ui, text: &str, data: &[&str], color: Color32) {
    ui.add(Label::new(data_text::with_data(text, data, color)).wrap());
}

/// The kittest harness and the overlap checks the row layout tests share.
#[cfg(test)]
pub(crate) mod row_checks {
    use egui::accesskit::Role;
    use egui::{Rect, Shape, Ui, UiBuilder, pos2, vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use ui_kit::tokens::space;

    /// How far a slider's knob reaches past the ends of its rect.
    const KNOB_REACH: f32 = 7.0;
    /// The height of the test window, enough for every row a test draws.
    const HEIGHT: f32 = 600.0;

    /// One painted text and the part of the window it may show in.
    pub(crate) struct Painted {
        pub(crate) text: String,
        pub(crate) rect: Rect,
        clip: Rect,
    }

    /// A harness that draws `draw` in a panel `width` wide, as the app's columns do, with the
    /// suite's fonts and style.
    pub(crate) fn panel_harness(width: f32, mut draw: impl FnMut(&mut Ui) + 'static) -> Harness<'static> {
        let mut first_pass = true;
        let mut harness = Harness::builder().with_size(vec2(width + 100.0, HEIGHT)).build_ui(move |ui| {
            // The fonts a style asks for are only there from the next pass on.
            if std::mem::take(&mut first_pass) {
                ui_kit::apply(ui.ctx());
                crate::data_text::apply(ui.ctx());
                return;
            }
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT));
            let mut column = ui.new_child(UiBuilder::new().max_rect(rect));
            column.set_clip_rect(rect);
            ui_kit::side_panel(&mut column, |ui| {
                ui.set_min_size(ui.available_size());
                draw(ui);
            });
        });
        harness.run();
        harness
    }

    /// Every text the last frame painted.
    pub(crate) fn painted(harness: &Harness<'_>) -> Vec<Painted> {
        fn collect(shape: &Shape, clip: Rect, found: &mut Vec<Painted>) {
            match shape {
                Shape::Text(text) if !text.galley.text().is_empty() => {
                    let rect = Rect::from_min_size(text.pos, text.galley.size());
                    found.push(Painted { text: text.galley.text().to_owned(), rect, clip });
                }
                Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, clip, found)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        for clipped in &harness.output().shapes {
            collect(&clipped.shape, clipped.clip_rect, &mut found);
        }
        found
    }

    /// The areas the sliders (with their knobs) and the switches cover.
    pub(crate) fn controls(harness: &Harness<'_>) -> Vec<Rect> {
        let sliders = harness.query_all_by_role(Role::Slider).map(|node| node.rect().expand2(vec2(KNOB_REACH, 0.0)));
        sliders.chain(harness.query_all_by_label("Switch").map(|node| node.rect())).collect()
    }

    /// Fails when a text is clipped or leaves the panel's content, or when any two of the texts
    /// and controls overlap.
    pub(crate) fn assert_no_overlap(texts: &[&Painted], controls: &[Rect], panel_width: f32) {
        // The side panel's padding.
        let content_right = panel_width - space::PANEL_PADDING;
        for painted in texts {
            assert!(painted.clip.expand(0.5).contains_rect(painted.rect), "{:?} is clipped: {:?} in {:?}", painted.text, painted.rect, painted.clip);
            assert!(
                painted.rect.left() >= space::PANEL_PADDING && painted.rect.right() <= content_right + 0.5,
                "{:?} leaves the panel: {:?}",
                painted.text,
                painted.rect
            );
        }
        let named: Vec<(&str, Rect)> =
            texts.iter().map(|painted| (painted.text.as_str(), painted.rect)).chain(controls.iter().map(|rect| ("a control", *rect))).collect();
        for (index, (name, rect)) in named.iter().enumerate() {
            for (other_name, other) in &named[index + 1..] {
                assert!(!rect.shrink(0.25).intersects(other.shrink(0.25)), "{name:?} {rect:?} overlaps {other_name:?} {other:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ui_kit::{SliderKind, inspector_group, switch};

    use super::row_checks::{assert_no_overlap, controls, painted, panel_harness};
    use super::{Tool, slider_row};
    use crate::config;
    use crate::config::Settings;

    /// The Camera tool's slider rows keep label, slider, Auto switch and value apart at the
    /// narrowest right panel, with the longest label and the widest values.
    #[test]
    fn camera_rows_do_not_overlap_at_the_narrowest_panel() {
        let harness = panel_harness(config::RIGHT_PANEL_MIN, |ui| {
            inspector_group(ui, |ui| {
                // The last row has no Auto switch.
                for (label, text, auto_capable) in [("Exposure", "-13", true), ("White balance", "10000", true), ("Brightness", "-255", false)] {
                    slider_row(ui, label, text, |slider_ui, switch_ui| {
                        let mut value = 1.0;
                        ui_kit::slider(slider_ui, &mut value, 0.0..=1.0, SliderKind::Accent);
                        if auto_capable {
                            let mut auto = false;
                            switch(switch_ui, &mut auto);
                        }
                    });
                }
            });
        });
        let texts = painted(&harness);
        let controls = controls(&harness);
        assert_eq!((texts.len(), controls.len()), (6, 5));
        assert_no_overlap(&texts.iter().collect::<Vec<_>>(), &controls, config::RIGHT_PANEL_MIN);
        assert!(controls.iter().all(|rect| rect.width() >= 40.0), "every slider keeps a usable width: {controls:?}");
    }

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
