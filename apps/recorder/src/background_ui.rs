//! The Background section of the window: the colour, image or video drawn behind the camera.
//!
//! It sits under the Layout section and edits the active profile in the shared [`ProfileStore`], so a
//! change reaches the compose thread on the next frame. The section is read-only while a take records,
//! and a failed background image or video shows the compose thread's warning as a banner.

use crate::config::{BackgroundFit, BackgroundKind};
use crate::profiles::ProfileStore;

/// The background kinds in the order the picker lists them, with their labels.
const KINDS: [(BackgroundKind, &str); 4] =
    [(BackgroundKind::None, "None"), (BackgroundKind::Color, "Colour"), (BackgroundKind::Image, "Image"), (BackgroundKind::Video, "Video")];

/// The image and video fits in the order the picker lists them, with their labels.
const FITS: [(BackgroundFit, &str); 3] = [(BackgroundFit::Cover, "Cover"), (BackgroundFit::Contain, "Contain"), (BackgroundFit::Stretch, "Stretch")];

/// Draws the Background section for the active profile; `recording` makes it read-only.
pub fn show(ui: &mut egui::Ui, store: &mut ProfileStore, profile_name: &str, recording: bool) {
    ui.heading("Background");
    let Some(profile) = store.get(profile_name) else {
        ui.label("No profile is active.");
        return;
    };
    let mut background = profile.background.clone();
    let mut changed = false;
    ui.add_enabled_ui(!recording, |ui| {
        ui.horizontal(|ui| {
            for (kind, label) in KINDS {
                changed |= ui.selectable_value(&mut background.kind, kind, label).changed();
            }
        });
        match background.kind {
            BackgroundKind::None => {
                ui.label("A keyed camera floats over the screen.");
            }
            BackgroundKind::Color => {
                ui.horizontal(|ui| {
                    ui.label("Colour");
                    changed |= ui.color_edit_button_srgb(&mut background.color).changed();
                });
            }
            BackgroundKind::Image | BackgroundKind::Video => {
                let video = background.kind == BackgroundKind::Video;
                ui.horizontal(|ui| {
                    ui.label(if video { "Video" } else { "Image" });
                    ui.label(file_label(
                        if video { &background.video_path } else { &background.image_path },
                        if video { "(no video picked)" } else { "(no image picked)" },
                    ));
                    if ui.button("Browse...").clicked() {
                        let picked = if video { crate::platform::pick_video_file() } else { crate::platform::pick_image_file() };
                        if let Some(path) = picked {
                            let path = path.to_string_lossy().into_owned();
                            if video {
                                background.video_path = path
                            } else {
                                background.image_path = path
                            }
                            changed = true;
                        }
                    }
                    let clear = if video { !background.video_path.is_empty() } else { !background.image_path.is_empty() };
                    if ui.button("Clear").clicked() && clear {
                        if video {
                            background.video_path.clear()
                        } else {
                            background.image_path.clear()
                        }
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Fit");
                    for (fit, label) in FITS {
                        changed |= ui.selectable_value(&mut background.fit, fit, label).changed();
                    }
                });
            }
        }
    });
    if recording {
        ui.label("The background is fixed while recording.");
    }
    if let Some(warning) = crate::compositor::shared().and_then(|handle| handle.warning()) {
        ui.label(format!("Warning: {warning}"));
    }
    if changed {
        store.edit(profile_name, |profile| profile.background = background);
    }
}

/// The file name of a background file path, or the hint when none is set.
fn file_label(path: &str, hint: &str) -> String {
    std::path::Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| hint.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_and_fit_has_its_own_label() {
        let kinds: Vec<&str> = KINDS.iter().map(|(_, label)| *label).collect();
        let fits: Vec<&str> = FITS.iter().map(|(_, label)| *label).collect();
        for (values, kind) in [(&kinds[..], "kind"), (&fits[..], "fit")] {
            for (index, label) in values.iter().enumerate() {
                assert!(!values[index + 1..].contains(label), "a {kind} label is listed twice: {label}");
            }
        }
    }

    #[test]
    fn an_empty_path_gets_a_hint() {
        assert_eq!(file_label("", "(no image picked)"), "(no image picked)");
        assert_eq!(file_label("", "(no video picked)"), "(no video picked)");
        assert_eq!(file_label(r"C:\pics\bg.png", "(no image picked)"), "bg.png");
        assert_eq!(file_label(r"C:\clips\bg.mkv", "(no video picked)"), "bg.mkv");
    }
}
