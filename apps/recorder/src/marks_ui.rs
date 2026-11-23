//! The marks rows of the Record mode's Live panel (spec "Record mode"): one
//! list row per recent mark, newest first and selected.

use std::collections::VecDeque;

use egui::Ui;
use ui_kit::{MarkerKind, marker_color};

use crate::engine::marks::RecentMark;

/// Draws the recent marks, newest first, with the newest row selected so the
/// newest mark is always the one the list starts on (spec "Record mode").
pub fn rows(ui: &mut Ui, recent: &VecDeque<RecentMark>) {
    if recent.is_empty() {
        ui.weak("No marks yet");
        return;
    }
    for (index, (timecode, category, label)) in recent.iter().rev().enumerate() {
        let label = match label {
            Some(label) if !label.is_empty() => label.clone(),
            _ => category.clone(),
        };
        ui_kit::list_row(ui, index == 0, Some(marker_color(kind(category))), &label, Some(timecode));
    }
}

/// The marker kind of a category name. The ui-kit marker palette has no
/// highlight or chapter colors (markers never follow the accent), so those
/// take the closest categories' colors.
fn kind(category: &str) -> MarkerKind {
    match category {
        "Cut" => MarkerKind::Cut,
        "Retake" | "Highlight" => MarkerKind::Retake,
        "B-roll" => MarkerKind::BRoll,
        "System" => MarkerKind::System,
        _ => MarkerKind::Note,
    }
}
