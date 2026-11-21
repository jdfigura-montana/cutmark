//! Toasts (spec "Components": row "Toasts"): a glass capsule at the top
//! center, four second auto-dismiss, one at a time. [`toast`] queues,
//! [`show_toasts`] draws once per frame from any app.

use std::collections::VecDeque;

use egui::{Align2, Context, Id, Margin, Order, vec2};

use crate::fonts;
use crate::tokens::{color, space, typography};
use crate::widgets::{capsule_radius, glass};

/// Seconds a toast stays on screen (spec "Toasts": 4 second auto-dismiss).
const TOAST_SECONDS: f64 = 4.0;

/// The context-data id of the toast queue.
fn queue_id() -> Id {
    Id::new("cutmark_toast_queue")
}

/// The context-data id of the toast clock override (test hook).
fn time_id() -> Id {
    Id::new("cutmark_toast_time")
}

/// The toast queue and slot bookkeeping, stored in the egui context data.
#[derive(Debug, Clone, Default)]
struct Toasts {
    /// Queued texts with their serial numbers (front shows first).
    items: VecDeque<(u64, String)>,
    /// The serial number the next queued toast gets.
    next_serial: u64,
    /// Clock reading the front toast started showing, if one is showing.
    started: Option<f64>,
}

/// Queues a toast. It shows when it reaches the front of the queue and the
/// app calls [`show_toasts`].
pub fn toast(ctx: &Context, text: impl Into<String>) {
    ctx.data_mut(|data| {
        let mut toasts = data.get_temp::<Toasts>(queue_id()).unwrap_or_default();
        let serial = toasts.next_serial;
        toasts.next_serial += 1;
        toasts.items.push_back((serial, text.into()));
        data.insert_temp(queue_id(), toasts);
    });
    ctx.request_repaint();
}

/// Overrides the toast clock, in seconds. Test hook only: apps let the hook
/// stay unset and the clock follows the egui input time.
pub fn set_toast_time(ctx: &Context, seconds: f64) {
    ctx.data_mut(|data| data.insert_temp(time_id(), seconds));
    ctx.request_repaint();
}

/// The toast clock: the test override when set, the egui input time otherwise.
fn now(ctx: &Context) -> f64 {
    let overridden = ctx.data(|data| data.get_temp::<f64>(time_id()));
    overridden.unwrap_or_else(|| ctx.input(|input| input.time))
}

/// Draws the front toast, if any, as a glass capsule at the top center, and
/// expires it after [`TOAST_SECONDS`] so the next queued toast shows. One at
/// a time. Returns the response of the toast shown this frame, if any.
pub fn show_toasts(ctx: &Context) -> Option<egui::Response> {
    let now = now(ctx);
    // Expire the front toast once its four seconds are up, so the next one shows.
    ctx.data_mut(|data| {
        let mut toasts = data.get_temp::<Toasts>(queue_id()).unwrap_or_default();
        loop {
            if toasts.items.is_empty() {
                toasts.started = None;
                break;
            }
            match toasts.started {
                Some(started) if now - started < TOAST_SECONDS => break,
                Some(_) => {
                    toasts.items.pop_front();
                    toasts.started = None;
                }
                None => {
                    toasts.started = Some(now);
                    break;
                }
            }
        }
        data.insert_temp(queue_id(), toasts);
    });

    let front = ctx.data(|data| data.get_temp::<Toasts>(queue_id()).and_then(|toasts| toasts.items.front().cloned()));
    front.map(|(serial, text)| {
        let font = fonts::medium(typography::BODY_SIZE);
        let job = egui::text::LayoutJob::simple(text, font, color::TEXT_PRIMARY, f32::INFINITY);
        let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
        let pad_x = space::SCALE_12;
        let pad_y = space::SCALE_8;
        let height = galley.size().y + 2.0 * pad_y;
        egui::Area::new(Id::new("cutmark_toast").with(serial))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_TOP, vec2(0.0, space::SCALE_16))
            .show(ctx, |ui| {
                glass::glass_frame(ctx, capsule_radius(height)).inner_margin(Margin::symmetric(pad_x as i8, pad_y as i8)).show(ui, |ui| {
                    ui.add(egui::Label::new(galley));
                });
            })
            .response
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one pass (with fonts installed from the first pass on) and
    /// reports which serial is at the queue front (a toast drew for it).
    fn shown(ctx: &Context, first_pass: &std::cell::Cell<bool>) -> Option<u64> {
        let mut serial = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if first_pass.get() {
                first_pass.set(false);
                ui.ctx().set_fonts(fonts::definitions());
                return;
            }
            let drew = show_toasts(ui.ctx()).is_some();
            serial = ui.ctx().data(|data| data.get_temp::<Toasts>(queue_id()).and_then(|toasts| toasts.items.front().map(|(serial, _)| *serial)));
            assert_eq!(drew, serial.is_some(), "a drawn toast must match the queue front");
        });
        output.textures_delta.clear();
        serial
    }

    /// The second toast shows only after the first expires.
    #[test]
    fn the_second_toast_shows_only_after_the_first_expires() {
        let ctx = Context::default();
        let first_pass = std::cell::Cell::new(true);
        shown(&ctx, &first_pass); // install the fonts

        toast(&ctx, "first");
        toast(&ctx, "second");
        assert_eq!(shown(&ctx, &first_pass), Some(0), "the first toast must show");
        assert_eq!(shown(&ctx, &first_pass), Some(0), "the first toast must keep showing");
        assert_eq!(shown(&ctx, &first_pass), Some(0), "the second toast must wait for the first");

        set_toast_time(&ctx, TOAST_SECONDS + 0.1);
        assert_eq!(shown(&ctx, &first_pass), Some(1), "the second toast must show after the first expires");
    }

    /// Just before the four seconds are up the first toast stays on screen.
    #[test]
    fn the_first_toast_stays_until_it_expires() {
        let ctx = Context::default();
        let first_pass = std::cell::Cell::new(true);
        shown(&ctx, &first_pass);
        toast(&ctx, "first");
        shown(&ctx, &first_pass);

        set_toast_time(&ctx, TOAST_SECONDS - 0.1);
        assert_eq!(shown(&ctx, &first_pass), Some(0), "the first toast must still show just before four seconds");
    }
}
