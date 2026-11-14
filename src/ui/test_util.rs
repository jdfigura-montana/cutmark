//! Headless egui helpers shared by the ui tests: run frames without a window,
//! send pointer clicks and read back what was painted.

use eframe::egui::epaint::{CircleShape, RectShape};
use eframe::egui::{
    self, Color32, Event, FullOutput, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2,
    ViewportEvent, ViewportId,
};

/// Runs `work` on its own thread and returns its result, failing the test if
/// it takes longer than `seconds`: an endless loop in UI code (a frozen app)
/// then fails fast instead of hanging the whole test run. Build everything the
/// work needs inside the closure, so nothing has to be `Send`.
pub fn within_secs<T: Send + 'static>(
    seconds: u64,
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    match receiver.recv_timeout(std::time::Duration::from_secs(seconds)) {
        Ok(value) => value,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("still running after {seconds} s: the UI would be frozen")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("the work panicked; see the panic message above")
        }
    }
}

/// A context with the app theme applied, driven one frame at a time.
pub struct Headless {
    /// The egui context shared by every frame.
    pub ctx: egui::Context,
    /// Screen size in points.
    size: Vec2,
    /// Fake clock in seconds, advanced 1/60 s per frame.
    time: f64,
}

impl Headless {
    /// Creates the context on a screen of `width` x `height` points and runs
    /// one frame so the theme fonts are loaded.
    pub fn new(width: f32, height: f32) -> Self {
        let ctx = egui::Context::default();
        crate::ui::theme::apply(&ctx);
        let mut headless = Headless {
            ctx,
            size: egui::vec2(width, height),
            time: 0.0,
        };
        headless.frame(Vec::new(), |_| {});
        headless
    }

    /// Runs one frame with `events`; returns what was painted. Multi-pass
    /// frames run `add_contents` once per pass, like the real app.
    pub fn frame(&mut self, events: Vec<Event>, add_contents: impl FnMut(&mut egui::Ui)) -> Out {
        self.frame_with(events, false, add_contents)
    }

    /// Like [`Headless::frame`] but with the window's close button pressed.
    pub fn frame_close_requested(&mut self, add_contents: impl FnMut(&mut egui::Ui)) -> Out {
        self.frame_with(Vec::new(), true, add_contents)
    }

    fn frame_with(
        &mut self,
        events: Vec<Event>,
        close_requested: bool,
        add_contents: impl FnMut(&mut egui::Ui),
    ) -> Out {
        self.time += 1.0 / 60.0;
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        if close_requested {
            input
                .viewports
                .entry(ViewportId::ROOT)
                .or_default()
                .events
                .push(ViewportEvent::Close);
        }
        let mut output = self.ctx.run_ui(input, add_contents);
        output.textures_delta.clear();
        Out(output)
    }

    /// Runs ten idle frames (a sixth of a second) so first-frame-hidden and
    /// fading-in widgets (modals) reach their final layout and opacity;
    /// returns the last frame.
    pub fn settle(&mut self, mut add_contents: impl FnMut(&mut egui::Ui)) -> Out {
        let mut last = self.frame(Vec::new(), &mut add_contents);
        for _ in 0..9 {
            last = self.frame(Vec::new(), &mut add_contents);
        }
        last
    }

    /// Moves the pointer to `pos`, presses and releases the primary button,
    /// one frame each. Returns the release frame.
    pub fn click(&mut self, pos: Pos2, mut add_contents: impl FnMut(&mut egui::Ui)) -> Out {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let _ = self.frame(vec![Event::PointerMoved(pos)], &mut add_contents);
        let _ = self.frame(vec![button(true)], &mut add_contents);
        self.frame(vec![button(false)], &mut add_contents)
    }
}

/// One painted text run: its string and screen rectangle.
pub struct TextItem {
    /// The laid-out string.
    pub text: String,
    /// Where it was painted.
    pub rect: Rect,
    /// The text color of its first section.
    pub color: Color32,
}

/// The shapes painted in one frame, flattened for assertions.
pub struct Out(FullOutput);

impl Out {
    fn shapes(&self) -> Vec<&Shape> {
        fn walk<'a>(shape: &'a Shape, into: &mut Vec<&'a Shape>) {
            if let Shape::Vec(inner) = shape {
                for shape in inner {
                    walk(shape, into);
                }
            } else {
                into.push(shape);
            }
        }
        let mut flat = Vec::new();
        for clipped in &self.0.shapes {
            walk(&clipped.shape, &mut flat);
        }
        flat
    }

    /// Every painted text run, in paint order.
    pub fn texts(&self) -> Vec<TextItem> {
        self.shapes()
            .into_iter()
            .filter_map(|shape| match shape {
                Shape::Text(text) => Some(TextItem {
                    text: text.galley.text().to_string(),
                    rect: text.galley.rect.translate(text.pos.to_vec2()),
                    color: text
                        .galley
                        .job
                        .sections
                        .first()
                        .map_or(text.fallback_color, |section| section.format.color),
                }),
                _ => None,
            })
            .collect()
    }

    /// The rectangle of the first text run equal to `label`.
    pub fn text_rect(&self, label: &str) -> Option<Rect> {
        self.texts()
            .into_iter()
            .find(|item| item.text == label)
            .map(|item| item.rect)
    }

    /// Every painted filled circle.
    pub fn circles(&self) -> Vec<CircleShape> {
        self.shapes()
            .into_iter()
            .filter_map(|shape| match shape {
                Shape::Circle(circle) => Some(*circle),
                _ => None,
            })
            .collect()
    }

    /// Every painted rectangle.
    pub fn rects(&self) -> Vec<RectShape> {
        self.shapes()
            .into_iter()
            .filter_map(|shape| match shape {
                Shape::Rect(rect) => Some(rect.clone()),
                _ => None,
            })
            .collect()
    }

    /// The smallest filled rectangle containing `point`: the button or field
    /// background under a text run's center.
    pub fn filled_rect_at(&self, point: Pos2) -> Option<Rect> {
        self.rects()
            .into_iter()
            .filter(|r| r.fill.a() > 0 && r.rect.contains(point))
            .map(|r| r.rect)
            .min_by(|a, b| a.area().total_cmp(&b.area()))
    }

    /// The dialog footer's 1 px divider, whose edges are the card's content
    /// edges: the widest filled rect that is one point tall.
    pub fn footer_line(&self) -> Option<Rect> {
        self.rects()
            .into_iter()
            .filter(|r| r.fill.a() > 0 && r.rect.height() == 1.0)
            .map(|r| r.rect)
            .max_by(|a, b| a.width().total_cmp(&b.width()))
    }

    /// True when the app asked the window to stay open this frame.
    pub fn cancelled_close(&self) -> bool {
        self.0
            .viewport_output
            .get(&ViewportId::ROOT)
            .is_some_and(|v| v.commands.contains(&egui::ViewportCommand::CancelClose))
    }

    /// True when the app asked the window to close this frame.
    pub fn sent_close(&self) -> bool {
        self.0
            .viewport_output
            .get(&ViewportId::ROOT)
            .is_some_and(|v| v.commands.contains(&egui::ViewportCommand::Close))
    }
}
