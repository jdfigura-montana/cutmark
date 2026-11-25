//! The Floating view (spec "Floating view"): one transparent immediate viewport that scrolls
//! sample text. A spike: the script, the widgets and the saved geometry come later.

use std::time::{Duration, Instant};

use egui::{Align2, Color32, Pos2, Rect, Sense, Vec2, ViewportBuilder, ViewportCommand, ViewportId};
use prompter_platform::view::{NativeWindow, ViewStatus, ViewWindow};
use ui_kit::tokens::{color, radius, size, space, typography};

const VIEW_SIZE: Vec2 = Vec2::new(760.0, 420.0);
/// 75% of 255.
const PANEL_ALPHA: u8 = 191;
const SCRIPT_TEXT_SIZE: f32 = 44.0;
const SCRIPT_LINE_HEIGHT: f32 = SCRIPT_TEXT_SIZE * 1.3;
const SCRIPT_LINES: usize = 40;
/// Points per second.
const SCROLL_SPEED: f64 = 60.0;
/// The reading line's distance from the top, as a share of the view's height.
const READING_LINE: f32 = 0.33;
/// winit applies a window change after the frame that asked for it, so the flags are read back on
/// this many frames after each change.
const RECHECK_FRAMES: u8 = 5;
/// How often a settled view is read back. The app gets no signal here for resume from sleep or
/// unlock, so this is what catches a repaint from those, or from anything else.
const HEALTH_CHECK: Duration = Duration::from_secs(1);

/// What a view's window sits on. A change means Windows may have repainted it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Placement {
    monitor: Option<isize>,
    scale: Option<f32>,
}

/// One open Floating view. Dropping it closes the window.
pub struct FloatingView {
    number: u32,
    position: Option<Pos2>,
    window: Option<NativeWindow>,
    status: ViewStatus,
    /// The click-through value last sent to this window.
    passthrough: bool,
    recheck: u8,
    placement: Placement,
    checked_at: Instant,
    /// Where in the drag bar the pointer took hold, in view coordinates.
    grab: Option<Pos2>,
    shown_at: Instant,
}

/// Where a new view goes: centered across the primary monitor and on the middle of its top third,
/// without leaving the top edge. The primary monitor starts at the desktop origin.
pub fn default_position(monitor: Vec2) -> Pos2 {
    Pos2::new(((monitor.x - VIEW_SIZE.x) / 2.0).max(0.0), (monitor.y / 6.0 - VIEW_SIZE.y / 2.0).max(0.0))
}

impl FloatingView {
    /// `number` must differ from that of every other open view: the window is found by its title.
    pub fn new(number: u32, position: Option<Pos2>) -> Self {
        let now = Instant::now();
        Self {
            number,
            position,
            window: None,
            status: ViewStatus::default(),
            passthrough: false,
            recheck: 0,
            placement: Placement::default(),
            checked_at: now,
            grab: None,
            shown_at: now,
        }
    }

    fn title(&self) -> String {
        format!("Cutmark Prompter View {}", self.number)
    }

    /// The view's flags as last read back, or `None` while its window has not been found.
    pub fn status(&self) -> Option<ViewStatus> {
        self.window.map(|_| self.status)
    }

    /// Draws one frame of the view. Returns false when the window was asked to close.
    pub fn show(&mut self, ctx: &egui::Context, click_through: bool) -> bool {
        let mut builder = ViewportBuilder::default()
            .with_title(self.title())
            .with_inner_size(VIEW_SIZE)
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false);
        if let Some(position) = self.position {
            builder = builder.with_position(position);
        }
        let id = ViewportId::from_hash_of(("prompter-view", self.number));
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            self.apply_window_setup(ui.ctx(), click_through);
            self.draw(ui, click_through);
            ui.ctx().request_repaint();
            !ui.ctx().input(|i| i.viewport().close_requested())
        })
    }

    /// The OS setup on the first frame the window exists, then the read-backs: after each change,
    /// and once a second from then on.
    fn apply_window_setup(&mut self, ctx: &egui::Context, click_through: bool) {
        if let Some(window) = self.window {
            let placement = placement(ctx, window);
            if self.placement != placement {
                log::info!("{} monitor or scale changed: {placement:?}", self.title());
                self.placement = placement;
                log_failure("clear_backing", window.clear_backing());
                self.recheck = RECHECK_FRAMES;
            }
            if check_due(self.recheck, self.checked_at.elapsed()) {
                self.status = repair(window);
                self.checked_at = Instant::now();
                if self.recheck > 0 {
                    self.recheck -= 1;
                    if self.recheck == 0 {
                        log::info!("{} settled: {:?}", self.title(), self.status);
                    }
                }
            }
        } else {
            let Some(window) = NativeWindow::find_own_window(&self.title()) else {
                return;
            };
            // First, as this frame has not been presented yet. Every show makes a new window, so
            // this runs on every show.
            log_failure("clear_backing", window.clear_backing());
            log_failure("exclude_from_capture", window.exclude_from_capture());
            log_failure("set_no_activate", window.set_no_activate());
            log_failure("reassert_topmost", window.reassert_topmost());
            self.window = Some(window);
            self.placement = placement(ctx, window);
            self.status = window.status();
            self.checked_at = Instant::now();
            log::info!("{} shown: {:?}", self.title(), self.status);
            self.recheck = RECHECK_FRAMES;
        }
        if self.passthrough != click_through {
            self.passthrough = click_through;
            self.grab = None;
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(click_through));
            log::info!("{} click-through: {click_through}", self.title());
            self.recheck = RECHECK_FRAMES;
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, click_through: bool) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();

        let [r, g, b, _] = color::GLASS_FILL.to_srgba_unmultiplied();
        let frame = ui_kit::glass_frame(&ctx, radius::PANEL).fill(Color32::from_rgba_unmultiplied(r, g, b, PANEL_ALPHA)).shadow(egui::Shadow::NONE);
        ui.painter().add(frame.paint(rect.shrink(frame.stroke.width)));
        ui_kit::widgets::glass::paint_glass_highlight(ui.painter(), rect, radius::PANEL);

        // The script area starts under the drag bar whether or not the bar shows, so the text
        // does not jump when click-through is toggled.
        let (bar, body) = rect.split_top_bottom_at_y(rect.top() + size::PANEL_BUTTON_HEIGHT);
        let reading_y = rect.top() + rect.height() * READING_LINE;
        let loop_height = f64::from(SCRIPT_LINE_HEIGHT) * SCRIPT_LINES as f64;
        let offset = (self.shown_at.elapsed().as_secs_f64() * SCROLL_SPEED % loop_height) as f32;
        let script = ui.painter().with_clip_rect(body);
        let font = ui_kit::fonts::medium(SCRIPT_TEXT_SIZE);
        // Three passes, so the loop's end runs straight into its start on both sides of the wrap.
        for pass in [-1.0_f32, 0.0, 1.0] {
            for line in 0..SCRIPT_LINES {
                let y = reading_y - offset + pass * loop_height as f32 + line as f32 * SCRIPT_LINE_HEIGHT;
                if y > body.top() - SCRIPT_LINE_HEIGHT && y < body.bottom() + SCRIPT_LINE_HEIGHT {
                    let text = format!("Line {}: keep a steady pace.", line + 1);
                    script.text(Pos2::new(rect.center().x, y), Align2::CENTER_CENTER, text, font.clone(), Color32::WHITE);
                }
            }
        }

        // The reading line is marked at its two ends, so it never strikes through the text.
        let accent = ui_kit::Theme::get(&ctx).accent.accent;
        let mark = Vec2::new(space::SCALE_24, space::SCALE_4);
        for x in [rect.left() + space::SCALE_16, rect.right() - space::SCALE_16 - mark.x] {
            ui.painter().rect_filled(Rect::from_min_size(Pos2::new(x, reading_y - mark.y / 2.0), mark), space::SCALE_2, accent);
        }

        if !click_through {
            self.drag_bar(ui, bar);
        }
    }

    /// The title bar that moves the window. egui-winit drops `StartDrag` for a window without
    /// focus, which a view never has, so the window is moved by hand.
    fn drag_bar(&mut self, ui: &mut egui::Ui, bar: Rect) {
        let response = ui.interact(bar, ui.id().with("drag-bar"), Sense::drag());
        ui.painter().text(bar.center(), Align2::CENTER_CENTER, "Spike script", ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_SECONDARY);
        if response.drag_started() {
            self.grab = ui.input(|i| i.pointer.press_origin());
        }
        if !response.dragged() {
            self.grab = None;
            return;
        }
        let (pointer, outer, moved) = ui.input(|i| {
            let moved = i.events.iter().any(|event| matches!(event, egui::Event::PointerMoved(_)));
            (i.pointer.latest_pos(), i.viewport().outer_rect, moved)
        });
        // Only on a fresh pointer position: a stale one would move the window a second time.
        if let (true, Some(grab), Some(pointer), Some(outer)) = (moved, self.grab, pointer, outer)
            && pointer != grab
        {
            ui.ctx().send_viewport_cmd(ViewportCommand::OuterPosition(outer.min + (pointer - grab)));
        }
    }
}

fn placement(ctx: &egui::Context, window: NativeWindow) -> Placement {
    Placement { monitor: window.monitor(), scale: ctx.input(|i| i.viewport().native_pixels_per_point) }
}

/// Every frame while a change is settling, then once per [`HEALTH_CHECK`].
fn check_due(recheck: u8, since_last: Duration) -> bool {
    recheck > 0 || since_last >= HEALTH_CHECK
}

/// Reads the flags back and re-applies any that dropped. winit rewrites the whole extended style
/// when one of its window flags changes, which removes the no-activate and tool-window styles;
/// Windows fills the backing white when it shows a window.
fn repair(window: NativeWindow) -> ViewStatus {
    let status = window.status();
    if status.excluded && status.no_activate && status.backing_is_clear {
        return status;
    }
    log::info!("view flags dropped, re-applying: {status:?}");
    if !status.no_activate {
        log_failure("set_no_activate", window.set_no_activate());
    }
    if !status.excluded {
        log_failure("exclude_from_capture", window.exclude_from_capture());
    }
    if !status.backing_is_clear {
        log_failure("clear_backing", window.clear_backing());
    }
    window.status()
}

/// A failed platform call leaves the view usable; the diagnostics line shows what is missing.
pub fn log_failure(call: &str, result: prompter_platform::Result<()>) {
    if let Err(err) = result {
        log::warn!("{call} failed: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settling_view_is_read_every_frame_then_once_a_second() {
        assert!(check_due(1, Duration::ZERO));
        assert!(!check_due(0, Duration::from_millis(999)));
        assert!(check_due(0, HEALTH_CHECK));
    }

    #[test]
    fn a_view_is_centered_in_the_top_third() {
        assert_eq!(default_position(Vec2::new(2560.0, 1440.0)), Pos2::new(900.0, 30.0));
    }

    #[test]
    fn a_view_never_starts_above_or_left_of_the_monitor() {
        assert_eq!(default_position(Vec2::new(1920.0, 1080.0)), Pos2::new(580.0, 0.0));
        assert_eq!(default_position(Vec2::new(640.0, 480.0)), Pos2::ZERO);
    }
}
