//! The status bar at the bottom of the window (spec "Status bar"): the state dot and label,
//! the free disk space, the camera ISO codec note, and the Control API clients on the right.

use egui::{Color32, FontId, Pos2, Sense, Vec2};

use crate::engine::controller::{ControllerState, Phase};
use crate::engine::stats::GIB;

/// Diameter of the state dot.
const DOT_DIAMETER: f32 = 6.0;

/// Draws the status bar contents: the state dot and label on the left, then the free disk
/// space and the camera ISO codec note when present; the Control API client count sits on
/// the right edge. While idle the label reads Ready, or Blocked when starting is blocked
/// (the reason stays in the banner strip; spec "Mapping").
pub fn show(ui: &mut egui::Ui, state: &ControllerState, api_clients: Option<usize>, blocked: bool) {
    use ui_kit::tokens::{color, size, space, typography};
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), size::STATUS_BAR_HEIGHT), Sense::hover());
    let painter = ui.painter();
    let font = ui_kit::fonts::medium(typography::CAPTION_SIZE);
    let text = color::TEXT_TERTIARY;
    let y = rect.center().y;

    // The state dot and its label; a countdown is a recording that has not begun yet.
    let (dot, label) = match state.phase {
        Phase::Idle if blocked => (color::SEM_RECORD, "Blocked"),
        Phase::Idle => (color::SEM_SUCCESS, "Ready"),
        Phase::Countdown(_) => (color::SEM_RECORD, "Starting"),
        Phase::Recording => (color::SEM_RECORD, "Recording"),
        Phase::Paused => (color::SEM_METER_WARN, "Paused"),
    };
    let mut x = rect.left() + space::SCALE_4;
    painter.circle_filled(Pos2::new(x + DOT_DIAMETER / 2.0, y), DOT_DIAMETER / 2.0, dot);
    x += DOT_DIAMETER + space::SCALE_8;
    x = draw_text(painter, x, y, label, &font, text);

    // The free disk space, while idle and while a take writes.
    let free = state.take_stats.as_ref().and_then(|stats| stats.disk_free_bytes).or(state.idle_free_bytes);
    x = draw_text(painter, x + space::SCALE_12, y, &format!("Disk free: {}", format_free(free)), &font, text);

    // The camera ISO codec note: an info line, not a warning.
    if let Some(info) = crate::engine::cam::info_line() {
        draw_text(painter, x + space::SCALE_12, y, &info, &font, text);
    }

    // The Control API clients, on the right edge of the bar.
    if let Some(clients) = api_clients {
        let label = format!("{clients} client{}", if clients == 1 { "" } else { "s" });
        let galley = painter.layout_no_wrap(label, font, text);
        let pos = Pos2::new(rect.right() - space::SCALE_4 - galley.size().x, y - galley.size().y / 2.0);
        painter.galley(pos, galley, text);
    }
}

/// Draws one caption text at `x`, vertically centered on `y`, and returns the x after it.
fn draw_text(painter: &egui::Painter, x: f32, y: f32, label: &str, font: &FontId, text: Color32) -> f32 {
    let galley = painter.layout_no_wrap(label.to_owned(), font.clone(), text);
    painter.galley(Pos2::new(x, y - galley.size().y / 2.0), galley.clone(), text);
    x + galley.size().x
}

/// Formats free space in GB, or a dash when it is unknown.
fn format_free(free: Option<u64>) -> String {
    free.map_or_else(|| "--".to_owned(), |bytes| format!("{:.1} GB", bytes as f64 / GIB as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_formats_in_gb_or_dashes_when_unknown() {
        assert_eq!(format_free(None), "--");
        assert_eq!(format_free(Some(5 * GIB)), "5.0 GB");
    }
}
