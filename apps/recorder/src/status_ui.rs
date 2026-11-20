//! The status bar at the bottom of the window: the live numbers while a take runs, the free disk space while idle.

use crate::engine::controller::{ControllerState, Phase};
use crate::engine::stats::{CamLine, GIB, TakeStats};

/// Draws the status bar's contents: every live number while recording or paused, the free disk space while idle.
pub fn show(ui: &mut egui::Ui, state: &ControllerState) {
    let live = if state.phase == Phase::Idle { None } else { state.take_stats.as_ref() };
    ui.horizontal_wrapped(|ui| match live {
        Some(stats) => recording(ui, stats),
        None => {
            let free = state.take_stats.as_ref().and_then(|stats| stats.disk_free_bytes).or(state.idle_free_bytes);
            ui.label(format!("Disk free: {}", format_free(free)));
        }
    });
}

/// The items of a running take, separated by thin lines.
fn recording(ui: &mut egui::Ui, stats: &TakeStats) {
    ui.label(format_hms(stats.elapsed_ms / 1000));
    ui.separator();
    ui.label(format!("Size {}", format_bytes(stats.bytes)));
    ui.separator();
    ui.label(format!("{:.1} Mbps", stats.bitrate_kbps() / 1000.0));
    ui.separator();
    ui.label(format!("Disk free: {}", format_free(stats.disk_free_bytes)));
    ui.separator();
    ui.label(format!("Left {}", format_left(stats.remaining_secs)));
    ui.separator();
    ui.label(format!("Duplicated {}", stats.duplicated));
    ui.separator();
    let dropped = format!("Dropped {}", stats.dropped);
    if stats.dropped > 0 {
        ui.label(egui::RichText::new(dropped).color(ui.visuals().error_fg_color));
    } else {
        ui.label(dropped);
    }
    ui.separator();
    ui.label(format!("Queue {}", stats.queue_depth));
    // The camera file's line, only while the camera file is being written.
    if let Some(cam) = &stats.cam {
        ui.separator();
        let text = cam_text(cam);
        if cam.dropped > 0 {
            ui.label(egui::RichText::new(text).color(ui.visuals().error_fg_color));
        } else {
            ui.label(text);
        }
    }
}

/// The camera file's line: its frames, the repeated frames and the dropped frames.
fn cam_text(cam: &CamLine) -> String {
    format!("Cam {} frames, duplicated {}, dropped {}", cam.frames, cam.duplicated, cam.dropped)
}

/// Formats a number of seconds as HH:MM:SS.
fn format_hms(total_seconds: u64) -> String {
    format!("{:02}:{:02}:{:02}", total_seconds / 3600, total_seconds / 60 % 60, total_seconds % 60)
}

/// Formats a byte count in MB below one GB and in GB above.
fn format_bytes(bytes: u64) -> String {
    if bytes >= GIB { format!("{:.2} GB", bytes as f64 / GIB as f64) } else { format!("{:.1} MB", bytes as f64 / 1_000_000.0) }
}

/// Formats free space in GB, or a dash when it is unknown.
fn format_free(free: Option<u64>) -> String {
    free.map_or_else(|| "--".to_owned(), |bytes| format!("{:.1} GB", bytes as f64 / GIB as f64))
}

/// Formats the time left as hours and minutes, or a dash when it is unknown.
fn format_left(seconds: Option<u64>) -> String {
    match seconds {
        None => "--".to_owned(),
        Some(seconds) if seconds >= 3600 => format!("{}h {:02}m", seconds / 3600, seconds / 60 % 60),
        Some(seconds) => format!("{} min", seconds.div_ceil(60)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_numbers_are_formatted_for_the_bar() {
        assert_eq!(format_hms(3725), "01:02:05");
        assert_eq!(format_bytes(12_500_000), "12.5 MB");
        assert_eq!(format_bytes(3 * GIB), "3.00 GB");
        assert_eq!(format_free(None), "--");
        assert_eq!(format_free(Some(5 * GIB)), "5.0 GB");
        assert_eq!(format_left(None), "--");
        assert_eq!(format_left(Some(90)), "2 min");
        assert_eq!(format_left(Some(7500)), "2h 05m");
    }

    #[test]
    fn the_camera_line_names_its_frames_duplicates_and_drops() {
        let cam = CamLine { frames: 900, duplicated: 3, dropped: 1 };
        assert_eq!(cam_text(&cam), "Cam 900 frames, duplicated 3, dropped 1");
    }
}
