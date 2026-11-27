//! The webcam UI pieces: the Camera ISO file group of the Output tool, the camera picture the
//! Camera and Key tools show, and the small formatting helpers they use.
//!
//! The Camera tool's panels moved to `tools_ui`, the Key tool's to `key_ui`.

use std::cell::RefCell;

use capture::CameraFormat;
use egui::{CursorIcon, Label, RichText, Sense, Ui};
use ui_kit::tokens::{color, space, typography};
use ui_kit::{inspector_group, inspector_row, switch};

use crate::config::{CompositeMode, Profile, WebcamSettings};
use crate::data_text;
use crate::preview_ui::Pick;
use crate::profile_ui::{AmfProbe, Control, Lock, iso_codec_text, rate_control_editor, rate_errors};
use crate::profiles::ProfileStore;
use crate::webcam::WebcamHost;

thread_local! {
    /// The encoder probe the camera file group reads its option ranges from.
    static ISO_PROBE: RefCell<AmfProbe> = RefCell::new(AmfProbe::default());
}

/// The Camera ISO file group of the Output tool (spec "Setup mode"): whether the raw camera is
/// saved on its own and how it is encoded. The rows are disabled while the switch is off, and the
/// group is read-only while `lock` locks the profile. While Clean applies the switch is locked on, and the saved
/// switch is left alone so it comes back when Clean goes off. The camera offset lives in the Camera
/// tool's Sync group.
pub(crate) fn iso_group(ui: &mut Ui, host: &WebcamHost, store: &mut ProfileStore, lock: Lock, profile_name: &str) {
    let Some(settings) = host.settings(store).cloned() else { return };
    let Some(profile) = store.get(profile_name).cloned() else { return };
    let key = (settings.format.width, settings.format.height, profile.fps);
    let (ranges, (codec_text, codec_caption)) = ISO_PROBE.with(|cell| {
        let mut probe = cell.borrow_mut();
        if !lock.locks(Control::Profile) {
            probe.update(ui.ctx(), key);
        }
        (probe.ranges(profile.codec), iso_codec_text(&profile, settings.format.height, &probe))
    });
    let mut iso = settings.iso;
    let mut rate = settings.iso_rc;
    let mut changed = false;
    inspector_group(ui, |ui| {
        let (mut shown, locked) = iso_switch(&profile, iso);
        lock.scope(ui, Control::Profile, !locked, |ui| {
            let state = if shown { "On" } else { "Off" };
            let before = shown;
            inspector_row(
                ui,
                "Camera ISO file",
                |ui| {
                    switch(ui, &mut shown);
                },
                Some(state),
            );
            // ui_kit's switch never marks its response changed, so the value is compared.
            if shown != before {
                iso = shown;
                changed = true;
            }
        });
        let caption = if locked { "Clean mode records the camera to its own file" } else { "Also save the raw camera as its own file." };
        ui.add_space(space::SCALE_4);
        ui.label(RichText::new(caption).small().weak());
        ui.add_space(space::SCALE_4);
        lock.scope(ui, Control::Profile, shown, |ui| {
            let errors = rate_errors(profile.codec, rate, ranges.as_ref());
            changed |= rate_control_editor(ui, "webcam-iso", &mut rate, ranges.as_ref(), &errors);
            inspector_row(ui, "Codec", |_ui| {}, Some(&codec_text));
            if let Some(reason) = &codec_caption {
                ui.add_space(space::SCALE_4);
                ui.add(Label::new(RichText::new(reason).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).wrap());
                ui.add_space(space::SCALE_4);
            }
        });
    });
    if changed {
        store.edit(profile_name, |profile| {
            if let Some(webcam) = &mut profile.webcam {
                webcam.iso = iso;
                webcam.iso_rc = rate;
            }
        });
    }
}

/// What the Camera ISO switch shows for a profile with the saved switch `saved`: whether it is on,
/// and whether it is locked there because a Clean take records the camera to its own file.
fn iso_switch(profile: &Profile, saved: bool) -> (bool, bool) {
    let locked = profile.effective_composite_mode() == CompositeMode::Clean;
    (saved || locked, locked)
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
    mode_text(&format.subtype.to_string(), format.width, format.height, format.fps())
}

/// A camera mode's size, rate and pixel format as separate items, drawn in Martian Mono.
pub(crate) fn mode_text(subtype: &str, width: u32, height: u32, fps: f64) -> String {
    format!("{}  {} fps  {subtype}", data_text::size_text(width, height), fps_text(fps))
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
    use crate::config::LayoutKind;

    #[test]
    fn the_camera_file_switch_is_locked_on_under_clean_picture_in_picture() {
        let webcam = Some(WebcamSettings::default());
        let clean = Profile { layout: LayoutKind::Pip, webcam, composite_mode: CompositeMode::Clean, ..Profile::tutorial_av1_1440p60() };
        for saved in [false, true] {
            assert_eq!(iso_switch(&clean, saved), (true, true));
            let baked = Profile { composite_mode: CompositeMode::Baked, ..clean.clone() };
            assert_eq!(iso_switch(&baked, saved), (saved, false));
            let screen = Profile { layout: LayoutKind::ScreenOnly, ..clean.clone() };
            assert_eq!(iso_switch(&screen, saved), (saved, false));
        }
    }

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
        assert_eq!(format_label(&format), "1920\u{d7}1080  29.97 fps  MJPG");
    }
}
