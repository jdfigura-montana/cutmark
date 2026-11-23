//! Window chrome without a native title bar (spec: Design Language "Window
//! chrome", S1 Audio Editor row "Window chrome").
//!
//! The window keeps its native frame: [`install`] subclasses it on Windows so
//! `WM_NCCALCSIZE` removes only the caption. The OS keeps the shadow, the
//! resize borders, Aero Snap and the Win+arrow shortcuts, and the Snap Layouts
//! flyout appears while the pointer rests on the maximize button. Every frame
//! the app publishes its drag and button areas with [`publish`]; the Windows
//! half ([`windows`]) turns them into `WM_NCHITTEST` answers. Everywhere else
//! the API compiles and does nothing, so apps stay portable.

#[cfg(windows)]
pub mod windows;

use egui::Rect;

/// The invisible resize hit area on every edge, in points (spec: 6px).
pub const RESIZE_BORDER_PT: f32 = 6.0;

/// The minimum window size `viewport` applies, in points.
pub const MIN_SIZE: egui::Vec2 = egui::Vec2::new(640.0, 400.0);

/// A non-client zone the pointer is over, as answered by the hit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// Normal UI: egui receives the mouse.
    Client,
    /// Empty toolbar space: drags the window, double-click maximizes.
    Caption,
    /// The maximize or restore button: the OS shows the Snap Layouts flyout.
    MaximizeButton,
    /// A resize edge or corner of the window.
    Resize(ResizeEdge),
}

/// One resize edge or corner of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    /// Left edge.
    Left,
    /// Right edge.
    Right,
    /// Top edge.
    Top,
    /// Bottom edge.
    Bottom,
    /// Top left corner.
    TopLeft,
    /// Top right corner.
    TopRight,
    /// Bottom left corner.
    BottomLeft,
    /// Bottom right corner.
    BottomRight,
}

/// The chrome areas of the current frame, in egui points.
///
/// The app builds one per frame and hands it to [`publish`]; everything not
/// published answers [`Zone::Client`].
#[derive(Debug, Clone, Default)]
pub struct Regions {
    /// Drag areas: empty toolbar space.
    caption: Vec<Rect>,
    /// The maximize or restore button rectangle.
    maximize_button: Option<Rect>,
}

impl Regions {
    /// Regions whose caption (drag) areas are `caption`.
    pub fn new(caption: Vec<Rect>) -> Self {
        Self { caption, maximize_button: None }
    }

    /// Adds the maximize or restore button rectangle.
    pub fn with_maximize_button(mut self, rect: Rect) -> Self {
        self.maximize_button = Some(rect);
        self
    }

    /// The published caption rectangles.
    pub fn caption(&self) -> &[Rect] {
        &self.caption
    }

    /// The published maximize button rectangle, if any.
    pub fn maximize_button(&self) -> Option<Rect> {
        self.maximize_button
    }
}

/// The published regions converted to physical pixels for the hit test.
#[derive(Debug, Clone, Default)]
pub(crate) struct HitRegions {
    /// Caption rectangles in physical pixels.
    pub caption: Vec<Rect>,
    /// Maximize button rectangle in physical pixels.
    pub maximize: Option<Rect>,
}

/// Converts a points rect to physical pixels at `pixels_per_point`.
fn to_physical(rect: Rect, pixels_per_point: f32) -> Rect {
    Rect::from_min_size(rect.min * pixels_per_point, rect.size() * pixels_per_point)
}

/// Publishes the frame's chrome regions so the hit test can use them.
///
/// Call once per frame, after the toolbar layout is known. On Windows this
/// feeds the `WM_NCHITTEST` subclass; elsewhere it does nothing.
pub fn publish(ctx: &egui::Context, regions: &Regions) {
    #[cfg(windows)]
    {
        windows::publish(ctx, regions);
    }
    #[cfg(not(windows))]
    {
        let _ = (ctx, regions);
    }
}

/// Hooks the window so the chrome takes over the caption. Call once from the
/// `eframe::AppCreator` closure, before the first frame.
///
/// On Windows this subclasses the native frame; if anything fails it logs and
/// leaves the native title bar in place (the recorded fallback). Elsewhere it
/// does nothing.
pub fn install(cc: &eframe::CreationContext) {
    #[cfg(windows)]
    {
        windows::install(cc);
    }
    #[cfg(not(windows))]
    {
        let _ = cc;
    }
}

/// Applies what the chrome needs to the viewport builder: the native frame
/// stays (only the caption is removed) and the window gets the suite minimum
/// size of 640 x 400.
pub fn viewport(builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    builder.with_decorations(true).with_min_inner_size(MIN_SIZE)
}

/// True when [`install`] hooked the window and the native title bar is gone.
pub fn is_custom() -> bool {
    #[cfg(windows)]
    {
        windows::is_custom()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// True while the pointer is over the published maximize button, as tracked by
/// the Windows subclass. Apps use this to draw the button's hover state,
/// because egui never sees non-client mouse moves.
pub fn is_maximized_button_hovered(ctx: &egui::Context) -> bool {
    let _ = ctx;
    #[cfg(windows)]
    {
        windows::is_maximized_button_hovered()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Answers the hit test for one pointer position. Pure so every platform can
/// unit-test it: `point` and `window_size` are physical pixels relative to the
/// window's top left corner, `maximized` suppresses the resize edges and `dpi`
/// is the window's pixels per point.
pub(crate) fn hit_test(point: egui::Pos2, window_size: egui::Vec2, maximized: bool, regions: &HitRegions, dpi: f32) -> Zone {
    let border = RESIZE_BORDER_PT * dpi;
    // Resize edges first (the caption may reach into them); none when
    // maximized. The window has to be wider than both borders for a corner
    // test to make sense.
    if !maximized && window_size.x > 2.0 * border && window_size.y > 2.0 * border {
        let left = point.x < border;
        let right = point.x > window_size.x - border;
        let top = point.y < border;
        let bottom = point.y > window_size.y - border;
        let edge = match (left, right, top, bottom) {
            (true, false, true, false) => Some(ResizeEdge::TopLeft),
            (false, true, true, false) => Some(ResizeEdge::TopRight),
            (true, false, false, true) => Some(ResizeEdge::BottomLeft),
            (false, true, false, true) => Some(ResizeEdge::BottomRight),
            (true, false, false, false) => Some(ResizeEdge::Left),
            (false, true, false, false) => Some(ResizeEdge::Right),
            (false, false, true, false) => Some(ResizeEdge::Top),
            (false, false, false, true) => Some(ResizeEdge::Bottom),
            _ => None,
        };
        if let Some(edge) = edge {
            return Zone::Resize(edge);
        }
    }
    if regions.maximize.is_some_and(|rect| rect.contains(point)) {
        return Zone::MaximizeButton;
    }
    if regions.caption.iter().any(|rect| rect.contains(point)) {
        return Zone::Caption;
    }
    Zone::Client
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 800 x 600 window at DPI 1.0 with a full-width 52px caption bar and a
    /// maximize button at the right.
    fn regions() -> HitRegions {
        HitRegions {
            caption: vec![Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 52.0))],
            maximize: Some(Rect::from_min_size(egui::pos2(746.0, 3.0), egui::vec2(46.0, 46.0))),
        }
    }

    /// Hits each edge and corner at DPI 1.0.
    #[test]
    fn edges_and_corners_at_dpi_1() {
        let regions = regions();
        let size = egui::vec2(800.0, 600.0);
        let cases = [
            (egui::pos2(3.0, 300.0), ResizeEdge::Left),
            (egui::pos2(797.0, 300.0), ResizeEdge::Right),
            (egui::pos2(400.0, 3.0), ResizeEdge::Top),
            (egui::pos2(400.0, 597.0), ResizeEdge::Bottom),
            (egui::pos2(3.0, 3.0), ResizeEdge::TopLeft),
            (egui::pos2(797.0, 3.0), ResizeEdge::TopRight),
            (egui::pos2(3.0, 597.0), ResizeEdge::BottomLeft),
            (egui::pos2(797.0, 597.0), ResizeEdge::BottomRight),
        ];
        for (point, edge) in cases {
            assert_eq!(hit_test(point, size, false, &regions, 1.0), Zone::Resize(edge), "at {point:?}");
        }
    }

    /// The maximize button wins over the caption but not over the corners.
    #[test]
    fn maximize_button_beats_caption_but_not_corners() {
        let regions = regions();
        let size = egui::vec2(800.0, 600.0);
        assert_eq!(hit_test(egui::pos2(769.0, 26.0), size, false, &regions, 1.0), Zone::MaximizeButton);
        // The button's own top strip still resizes the window.
        assert_eq!(hit_test(egui::pos2(769.0, 1.0), size, false, &regions, 1.0), Zone::Resize(ResizeEdge::Top));
        // The caption bar around the button drags.
        assert_eq!(hit_test(egui::pos2(300.0, 26.0), size, false, &regions, 1.0), Zone::Caption);
        // A point in the bar left of the button is still caption.
        assert_eq!(hit_test(egui::pos2(740.0, 26.0), size, false, &regions, 1.0), Zone::Caption);
        // Points below the bar, outside the borders, are client area.
        assert_eq!(hit_test(egui::pos2(400.0, 300.0), size, false, &regions, 1.0), Zone::Client);
        assert_eq!(hit_test(egui::pos2(400.0, 550.0), size, false, &regions, 1.0), Zone::Client);
    }

    /// A maximized window has no resize edges: the same border points turn
    /// into caption or client.
    #[test]
    fn maximized_has_no_resize_edges() {
        let regions = regions();
        let size = egui::vec2(800.0, 600.0);
        assert_eq!(hit_test(egui::pos2(3.0, 3.0), size, true, &regions, 1.0), Zone::Caption);
        assert_eq!(hit_test(egui::pos2(797.0, 597.0), size, true, &regions, 1.0), Zone::Client);
        assert_eq!(hit_test(egui::pos2(400.0, 3.0), size, true, &regions, 1.0), Zone::Caption);
    }

    /// The 6px border scales with the DPI: 9px at 1.5 and 12px at 2.0.
    #[test]
    fn border_scales_with_dpi() {
        let regions = regions();
        let size = egui::vec2(800.0, 600.0);
        assert_eq!(hit_test(egui::pos2(8.0, 26.0), size, false, &regions, 1.5), Zone::Resize(ResizeEdge::Left));
        assert_eq!(hit_test(egui::pos2(10.0, 26.0), size, false, &regions, 1.5), Zone::Caption);
        assert_eq!(hit_test(egui::pos2(11.0, 26.0), size, false, &regions, 2.0), Zone::Resize(ResizeEdge::Left));
        assert_eq!(hit_test(egui::pos2(13.0, 26.0), size, false, &regions, 2.0), Zone::Caption);
        // At 1.0 the same points sit well inside the caption.
        assert_eq!(hit_test(egui::pos2(11.0, 26.0), size, false, &regions, 1.0), Zone::Caption);
    }

    /// Empty regions answer client everywhere except the resize edges.
    #[test]
    fn empty_regions_are_all_client() {
        let regions = HitRegions::default();
        let size = egui::vec2(800.0, 600.0);
        assert_eq!(hit_test(egui::pos2(400.0, 26.0), size, false, &regions, 1.0), Zone::Client);
        assert_eq!(hit_test(egui::pos2(3.0, 26.0), size, false, &regions, 1.0), Zone::Resize(ResizeEdge::Left));
    }

    /// `to_physical` scales positions and sizes, which is what publish feeds
    /// the subclass.
    #[test]
    fn to_physical_scales_the_rect() {
        let rect = Rect::from_min_size(egui::pos2(10.0, 2.0), egui::vec2(46.0, 52.0));
        let scaled = to_physical(rect, 1.5);
        assert_eq!(scaled.min, egui::pos2(15.0, 3.0));
        assert_eq!(scaled.size(), egui::vec2(69.0, 78.0));
    }
}
