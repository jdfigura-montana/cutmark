//! The Background section of the window: the colour or image drawn behind the camera.
//!
//! It sits under the Layout section and edits the active profile in the shared [`ProfileStore`], so a
//! change reaches the compose thread on the next frame. The section is read-only while a take records,
//! and a failed background image shows the compose thread's warning as a banner.

use crate::config::{BackgroundConfig, BackgroundFit, BackgroundKind};
use crate::profiles::ProfileStore;

/// The background kinds in the order the picker lists them, with their labels.
const KINDS: [(BackgroundKind, &str); 3] = [(BackgroundKind::None, "None"), (BackgroundKind::Color, "Colour"), (BackgroundKind::Image, "Image")];

/// The image fits in the order the picker lists them, with their labels.
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
            BackgroundKind::Image => {
                ui.horizontal(|ui| {
                    ui.label("Image");
                    ui.label(image_label(&background));
                    if ui.button("Browse...").clicked()
                        && let Some(path) = crate::platform::pick_image_file()
                    {
                        background.image_path = path.to_string_lossy().into_owned();
                        changed = true;
                    }
                    if ui.button("Clear").clicked() && !background.image_path.is_empty() {
                        background.image_path.clear();
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

/// The file name of the background image, or a hint when none is set.
fn image_label(background: &BackgroundConfig) -> String {
    std::path::Path::new(&background.image_path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "(no image picked)".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_and_fit_has_its_own_label() {
        for (values, kind) in [(KINDS.map(|(_, label)| label), "kind"), (FITS.map(|(_, label)| label), "fit")] {
            for (index, label) in values.iter().enumerate() {
                assert!(!values[index + 1..].contains(label), "a {kind} label is listed twice: {label}");
            }
        }
    }

    #[test]
    fn an_empty_image_path_gets_a_hint() {
        assert_eq!(image_label(&BackgroundConfig::default()), "(no image picked)");
        assert_eq!(image_label(&BackgroundConfig { image_path: r"C:\pics\bg.png".to_owned(), ..BackgroundConfig::default() }), "bg.png");
    }
}
