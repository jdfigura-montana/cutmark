//! The webcam UI pieces that stayed behind: the Camera ISO file group of the parked column, the
//! camera picture the Camera and Key tools show, and the small formatting helpers they use.
//!
//! The Camera tool's panels moved to `tools_ui`, the Key tool's to `key_ui`.

use std::cell::RefCell;

use capture::CameraFormat;
use egui::{CursorIcon, Sense, Ui};

use crate::config::WebcamSettings;
use crate::preview_ui::Pick;
use crate::profile_ui::{AmfProbe, iso_codec_text, rate_control_editor, rate_errors};
use crate::profiles::ProfileStore;
use crate::webcam::WebcamHost;

thread_local! {
    /// The encoder probe the camera file group reads its option ranges from.
    static ISO_PROBE: RefCell<AmfProbe> = RefCell::new(AmfProbe::default());
}

/// The Camera ISO file group: whether the raw camera is saved on its own, how it is encoded, and the camera offset.
/// Edits go to the active profile through the store, and the group is read-only while recording.
///
/// PARKED: this group waits in the parked column until the Output tool takes it in R4.
pub(crate) fn iso_group(ui: &mut Ui, host: &WebcamHost, store: &mut ProfileStore, recording: bool, profile_name: &str) {
    let Some(settings) = host.settings(store).cloned() else { return };
    let Some(profile) = store.get(profile_name).cloned() else { return };
    ui.label("Camera ISO file");
    let key = (settings.format.width, settings.format.height, profile.fps);
    let (ranges, codec_text) = ISO_PROBE.with(|cell| {
        let mut probe = cell.borrow_mut();
        if !recording {
            probe.update(ui.ctx(), key);
        }
        (probe.ranges(profile.codec), iso_codec_text(&profile, settings.format.height, &probe))
    });
    let mut iso = settings.iso;
    let mut rate = settings.iso_rc;
    let mut offset = settings.offset_ms;
    let mut changed = false;
    ui.add_enabled_ui(!recording, |ui| {
        changed |= ui.checkbox(&mut iso, "Also save the raw camera as its own file").changed();
        ui.add_enabled_ui(iso, |ui| {
            let errors = rate_errors(profile.codec, rate, ranges.as_ref());
            egui::Grid::new("webcam-iso").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                changed |= rate_control_editor(ui, "webcam-iso", &mut rate, ranges.as_ref(), &errors);
                ui.label("Codec");
                ui.label(codec_text);
                ui.end_row();
                ui.label("Camera offset");
                changed |= ui.add(egui::DragValue::new(&mut offset).range(-500..=500).suffix(" ms")).changed();
                ui.end_row();
            });
        });
    });
    if changed {
        store.edit(profile_name, |profile| {
            if let Some(webcam) = &mut profile.webcam {
                webcam.iso = iso;
                webcam.iso_rc = rate;
                webcam.offset_ms = offset;
            }
        });
    }
}

/// Draws the camera picture at the full width of its panel; while the eyedropper is armed
/// returns the pixel under a click on it.
pub(crate) fn camera_picture(ui: &mut Ui, host: &mut WebcamHost, eyedropper: bool) -> Option<Pick> {
    let scope = ui.scope(|ui| host.show_preview(ui));
    if !eyedropper {
        return None;
    }
    let response = ui.interact(scope.response.rect, ui.id().with("webcam-preview-click"), Sense::click()).on_hover_cursor(CursorIcon::Crosshair);
    if response.clicked() { response.interact_pointer_pos().and_then(|pos| host.pick(pos)) } else { None }
}

/// A frame rate without trailing zeros: 30, 29.97, 59.94.
pub(crate) fn fps_text(fps: f64) -> String {
    let text = format!("{fps:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// How a camera mode is listed in the dropdown.
pub(crate) fn format_label(format: &CameraFormat) -> String {
    format!("{} {}x{} @ {} fps", format.subtype, format.width, format.height, fps_text(format.fps()))
}

/// How many whole frames a residual is off by, or none when it is within one frame either way.
pub(crate) fn frames_off(residual_ms: f64, fps: f64) -> Option<u32> {
    let frames = residual_ms * fps / 1000.0;
    (frames.abs() > 1.0).then(|| frames.abs().round() as u32)
}

/// The offset that cancels a measured median: its opposite, rounded, and limited to the range the window allows.
pub(crate) fn applied_offset(median_ms: f64) -> i32 {
    WebcamSettings { offset_ms: (-median_ms).round() as i32, ..WebcamSettings::default() }.offset_clamped()
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
