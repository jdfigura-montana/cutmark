//! The Marks panel: the most recent marks of the running or last take, newest first.

use std::collections::VecDeque;

use ui_kit::palette::{ACCENT, METER_GREEN, METER_YELLOW, RECORD_RED, TEXT_MUTED, TEXT_NORMAL, TEXT_PRIMARY};

use crate::engine::marks::RecentMark;

/// How many rows the panel lists.
const ROW_LIMIT: usize = 10;

/// Draws the Marks panel for `recent`, which holds the marks oldest first.
pub fn show(ui: &mut egui::Ui, recent: &VecDeque<RecentMark>) {
    ui.heading("Marks");
    if recent.is_empty() {
        ui.colored_label(TEXT_MUTED, "No marks yet");
        return;
    }
    egui::Grid::new("recent-marks").num_columns(3).spacing([10.0, 4.0]).show(ui, |ui| {
        for (timecode, category, label) in recent.iter().rev().take(ROW_LIMIT) {
            ui.monospace(timecode);
            let color = category_color(category);
            ui.label(egui::RichText::new(category).small().color(color).background_color(color.gamma_multiply(0.2)));
            ui.label(label.as_deref().unwrap_or(""));
            ui.end_row();
        }
    });
}

/// The tag color of a category name.
fn category_color(category: &str) -> egui::Color32 {
    match category {
        "Highlight" => METER_YELLOW,
        "Cut" => RECORD_RED,
        "Retake" => METER_GREEN,
        "Chapter" => ACCENT,
        "B-roll" => TEXT_PRIMARY,
        "System" => TEXT_MUTED,
        _ => TEXT_NORMAL,
    }
}
