//! The tray icon: a grey or red dot with a menu to show the window, start or stop and quit.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::engine::bus::{BusSender, Command, CommandKind};
use crate::engine::controller::ControlSender;
use crate::platform;

/// Width and height of the tray icon in pixels.
const ICON_SIZE: u32 = 32;
/// Tray icon colour while idle.
const IDLE_RGB: [u8; 3] = [150, 150, 150];
/// Tray icon colour while recording.
const RECORDING_RGB: [u8; 3] = [220, 50, 50];
/// Tooltip text while idle.
const TOOLTIP_IDLE: &str = "Cutmark Recorder";
/// Tooltip text while recording.
const TOOLTIP_RECORDING: &str = "Cutmark Recorder - recording";

/// The tray icon: it handles its own menu clicks for the window and sends recording commands to the controller.
pub struct Tray {
    icon: TrayIcon,
    idle_icon: Icon,
    recording_icon: Icon,
}

impl Tray {
    /// Builds the tray icon and menu. Call it on the main thread. Show and Quit run here directly, so they
    /// work while the window is hidden; start or stop goes to the controller stamped with QPC at the click,
    /// and Quit tells the controller to stop before it closes the window.
    pub fn new(ctx: &egui::Context, hwnd: Option<isize>, bus: BusSender, control: ControlSender) -> Result<Self, String> {
        let idle_icon = circle_icon(IDLE_RGB)?;
        let recording_icon = circle_icon(RECORDING_RGB)?;

        let show = MenuItem::new("Show window", true, None);
        let start_stop = MenuItem::new("Start or stop recording", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[&show, &start_stop, &quit]).map_err(|err| format!("could not build the tray menu: {err}"))?;

        let (show_id, start_stop_id, quit_id) = (show.id().clone(), start_stop.id().clone(), quit.id().clone());
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                if let Some(hwnd) = hwnd {
                    platform::show_and_focus(hwnd);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.request_repaint();
            } else if event.id == start_stop_id {
                let press_qpc = capture::qpc_now();
                if bus.send(Command::new(CommandKind::StartStop, press_qpc)).is_err() {
                    log::warn!("a tray menu click was dropped because the app is gone");
                }
            } else if event.id == quit_id {
                control.stop();
                if let Some(hwnd) = hwnd {
                    platform::show_and_focus(hwnd);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
            }
        }));

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(idle_icon.clone())
            .with_tooltip(TOOLTIP_IDLE)
            .build()
            .map_err(|err| format!("could not create the tray icon: {err}"))?;
        Ok(Self { icon, idle_icon, recording_icon })
    }

    /// Swaps the icon colour and tooltip to match the recording state.
    pub fn set_recording(&self, recording: bool) {
        let (icon, tooltip) = if recording { (&self.recording_icon, TOOLTIP_RECORDING) } else { (&self.idle_icon, TOOLTIP_IDLE) };
        if let Err(err) = self.icon.set_icon(Some(icon.clone())) {
            log::warn!("could not change the tray icon: {err}");
        }
        if let Err(err) = self.icon.set_tooltip(Some(tooltip)) {
            log::warn!("could not change the tray tooltip: {err}");
        }
    }
}

/// Draws a filled circle with a soft edge as a 32x32 RGBA icon.
fn circle_icon(rgb: [u8; 3]) -> Result<Icon, String> {
    Icon::from_rgba(circle_rgba(rgb), ICON_SIZE, ICON_SIZE).map_err(|err| format!("could not build the tray icon image: {err}"))
}

/// The RGBA pixels of a filled circle that touches a one pixel margin around the icon.
fn circle_rgba(rgb: [u8; 3]) -> Vec<u8> {
    let size = ICON_SIZE as f32;
    let centre = (size - 1.0) / 2.0;
    let radius = size / 2.0 - 1.0;
    let mut pixels = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let distance = ((x as f32 - centre).powi(2) + (y as f32 - centre).powi(2)).sqrt();
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], (coverage * 255.0).round() as u8]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_is_opaque_in_the_middle_and_clear_in_the_corner() {
        let pixels = circle_rgba(RECORDING_RGB);
        assert_eq!(pixels.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
        let alpha_at = |x: u32, y: u32| pixels[((y * ICON_SIZE + x) * 4 + 3) as usize];
        assert_eq!(alpha_at(16, 16), 255);
        assert_eq!(alpha_at(0, 0), 0);
    }
}
