//! The Background group of the Layout tool: the colour, image or video drawn behind the camera.
//!
//! It edits the active profile in the shared [`ProfileStore`], so a change reaches the compose
//! thread on the next frame. The group is read-only while a take records, and a failed background
//! image or video shows the compose thread's warning under the group.

use egui::{RichText, Ui};
use ui_kit::tokens::space;
use ui_kit::{inspector_group, inspector_row};

use crate::config::{BackgroundFit, BackgroundKind};
use crate::layout_ui::choice;
use crate::profiles::ProfileStore;

/// The background kinds in the order the segmented control lists them, with their labels.
const KINDS: [(BackgroundKind, &str); 4] =
    [(BackgroundKind::None, "None"), (BackgroundKind::Color, "Colour"), (BackgroundKind::Image, "Image"), (BackgroundKind::Video, "Video")];

/// The image and video fits in the order the segmented control lists them, with their labels.
const FITS: [(BackgroundFit, &str); 3] = [(BackgroundFit::Cover, "Cover"), (BackgroundFit::Contain, "Contain"), (BackgroundFit::Stretch, "Stretch")];

/// Draws the Background group for the active profile; `recording` makes it read-only.
pub fn show(ui: &mut Ui, store: &mut ProfileStore, profile_name: &str, recording: bool) {
    let Some(profile) = store.get(profile_name) else {
        ui.weak("No profile is active.");
        return;
    };
    let mut background = profile.background.clone();
    let mut changed = false;
    inspector_group(ui, |ui| {
        ui.add_enabled_ui(!recording, |ui| {
            inspector_row(
                ui,
                "Background",
                |ui| {
                    changed |= choice(ui, "background-kind", &mut background.kind, &KINDS);
                },
                None,
            );
            match background.kind {
                BackgroundKind::None => {
                    ui.add_space(space::SCALE_4);
                    ui.label(RichText::new("A keyed camera floats over the screen.").small().weak());
                    ui.add_space(space::SCALE_4);
                }
                BackgroundKind::Color => {
                    inspector_row(
                        ui,
                        "Colour",
                        |ui| {
                            changed |= ui.color_edit_button_srgb(&mut background.color).changed();
                        },
                        None,
                    );
                }
                BackgroundKind::Image | BackgroundKind::Video => {
                    let video = background.kind == BackgroundKind::Video;
                    let label = if video { "Video" } else { "Image" };
                    let name =
                        if video { file_label(&background.video_path, "(no video picked)") } else { file_label(&background.image_path, "(no image picked)") };
                    let clear = if video { !background.video_path.is_empty() } else { !background.image_path.is_empty() };
                    inspector_row(
                        ui,
                        label,
                        |ui| {
                            if ui.button("Browse...").clicked() {
                                let picked = if video { crate::platform::pick_video_file() } else { crate::platform::pick_image_file() };
                                if let Some(path) = picked {
                                    let path = path.to_string_lossy().into_owned();
                                    if video {
                                        background.video_path = path;
                                    } else {
                                        background.image_path = path;
                                    }
                                    changed = true;
                                }
                            }
                            if ui.button("Clear").clicked() && clear {
                                if video {
                                    background.video_path.clear();
                                } else {
                                    background.image_path.clear();
                                }
                                changed = true;
                            }
                        },
                        Some(&name),
                    );
                    inspector_row(
                        ui,
                        "Fit",
                        |ui| {
                            changed |= choice(ui, "background-fit", &mut background.fit, &FITS);
                        },
                        None,
                    );
                }
            }
        });
    });
    if recording {
        ui.add_space(space::SCALE_4);
        ui.weak("The background is fixed while recording.");
    }
    if let Some(warning) = crate::compositor::shared().and_then(|handle| handle.warning()) {
        ui.add_space(space::SCALE_4);
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
