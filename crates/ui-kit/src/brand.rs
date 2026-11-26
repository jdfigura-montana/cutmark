//! The CutMark brand art (spec: Brand-Mark.md, sections 2 to 5): the Splice
//! mark, the wordmark lockup and the app icon.
//!
//! The SVG files in `assets/brand` are embedded and rasterized once per size in
//! physical pixels, so they stay crisp at any scale factor. The textures are
//! kept in the egui context. The files carry their own colors: the mark and
//! the wordmark show the app's brand accent, not the user's accent pick.

use std::sync::Arc;

use egui::emath::GuiRounding as _;
use egui::{Color32, ColorImage, Context, Id, Rect, Response, Sense, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};
use resvg::{tiny_skia, usvg};

/// The mark's side in a title bar (spec section 5).
pub const TITLE_BAR_MARK: f32 = 18.0;

/// The side of the window and taskbar icon, in pixels.
const ICON_PIXELS: u32 = 256;

/// A CutMark app, which picks the brand files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum App {
    /// The audio editor.
    Audio,
    /// The Recorder.
    Recorder,
    /// The video editor.
    Video,
    /// The Prompter.
    Prompter,
}

impl App {
    /// Every app.
    pub const ALL: [App; 4] = [App::Audio, App::Recorder, App::Video, App::Prompter];

    /// The name the wordmark lockup spells out.
    pub fn name(self) -> &'static str {
        match self {
            App::Audio => "CutMark Audio",
            App::Recorder => "CutMark Recorder",
            App::Video => "CutMark Video",
            App::Prompter => "CutMark Prompter",
        }
    }
}

/// The embedded files of one app.
struct Files {
    /// The mark for dark surfaces.
    mark_dark: &'static [u8],
    /// The mark for light surfaces.
    mark_light: &'static [u8],
    /// The lockup for dark surfaces.
    wordmark_dark: &'static [u8],
    /// The lockup for light surfaces.
    wordmark_light: &'static [u8],
    /// The app icon tile.
    icon: &'static [u8],
}

/// The [`Files`] of the app whose file names end in `$app`.
macro_rules! files {
    ($app:literal) => {
        Files {
            mark_dark: include_bytes!(concat!("../assets/brand/mark-", $app, ".svg")),
            mark_light: include_bytes!(concat!("../assets/brand/mark-", $app, "-light.svg")),
            wordmark_dark: include_bytes!(concat!("../assets/brand/wordmark-", $app, ".svg")),
            wordmark_light: include_bytes!(concat!("../assets/brand/wordmark-", $app, "-light.svg")),
            icon: include_bytes!(concat!("../assets/brand/icon-", $app, ".svg")),
        }
    };
}

/// The embedded files of `app`.
fn files(app: App) -> Files {
    match app {
        App::Audio => files!("audio"),
        App::Recorder => files!("recorder"),
        App::Video => files!("video"),
        App::Prompter => files!("prompter"),
    }
}

/// Which piece of in-app art is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Art {
    /// The mark alone.
    Mark,
    /// The mark, the word and the app name.
    Wordmark,
}

/// The file for `art`: the dark-surface one when `dark`, else the light one.
fn source(app: App, art: Art, dark: bool) -> &'static [u8] {
    let files = files(app);
    match (art, dark) {
        (Art::Mark, true) => files.mark_dark,
        (Art::Mark, false) => files.mark_light,
        (Art::Wordmark, true) => files.wordmark_dark,
        (Art::Wordmark, false) => files.wordmark_light,
    }
}

/// Parses one embedded SVG, logging a file that does not parse.
fn parse(data: &[u8]) -> Option<usvg::Tree> {
    match usvg::Tree::from_data(data, &usvg::Options::default()) {
        Ok(tree) => Some(tree),
        Err(err) => {
            log::error!("a brand SVG does not parse: {err}");
            None
        }
    }
}

/// The parsed tree of one file, parsed once per context.
fn tree(ctx: &Context, app: App, art: Art, dark: bool) -> Option<Arc<usvg::Tree>> {
    let id = Id::new(("ui_kit_brand_tree", app, art, dark));
    if let Some(cached) = ctx.data(|data| data.get_temp::<Option<Arc<usvg::Tree>>>(id)) {
        return cached;
    }
    let parsed = parse(source(app, art, dark)).map(Arc::new);
    ctx.data_mut(|data| data.insert_temp(id, parsed.clone()));
    parsed
}

/// Renders `tree` scaled to `height` pixels, in a bitmap just wide enough for it.
fn rasterize(tree: &usvg::Tree, height: u32) -> Option<tiny_skia::Pixmap> {
    let size = tree.size();
    let scale = height as f32 / size.height();
    let width = (size.width() * scale).ceil() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(pixmap)
}

/// The texture of one file at `height` pixels, rasterized on first use.
fn texture(ctx: &Context, tree: &usvg::Tree, key: (App, Art, bool), height: u32) -> Option<TextureHandle> {
    let id = Id::new(("ui_kit_brand_texture", key, height));
    if let Some(cached) = ctx.data(|data| data.get_temp::<Option<TextureHandle>>(id)) {
        return cached;
    }
    // Outside the data lock: load_texture takes the context lock itself.
    let texture = rasterize(tree, height).map(|pixmap| {
        let image = ColorImage::from_rgba_premultiplied([pixmap.width() as usize, pixmap.height() as usize], pixmap.data());
        ctx.load_texture(format!("cutmark-brand-{key:?}-{height}"), image, TextureOptions::LINEAR)
    });
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

/// Paints one file into `rect`, which has the file's shape. The bitmap is made
/// for the rect's physical height and drawn on whole pixels, one to one.
fn paint(ui: &Ui, tree: &usvg::Tree, key: (App, Art, bool), rect: Rect) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let pixels_per_point = ui.ctx().pixels_per_point();
    let height = (rect.height() * pixels_per_point).round().max(1.0) as u32;
    let Some(texture) = texture(ui.ctx(), tree, key, height) else {
        return;
    };
    let image = Rect::from_min_size(rect.min.round_to_pixels(pixels_per_point), texture.size_vec2() / pixels_per_point);
    // White leaves the texture's own colors unchanged.
    ui.painter().image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

/// Allocates a `height` tall space in the art's shape and paints it there.
fn show(ui: &mut Ui, app: App, art: Art, height: f32) -> Response {
    let dark = ui.visuals().dark_mode;
    let tree = tree(ui.ctx(), app, art, dark);
    let aspect = tree.as_ref().map_or(1.0, |tree| tree.size().width() / tree.size().height());
    let size: Vec2 = vec2(height * aspect, height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if let Some(tree) = tree {
        paint(ui, &tree, (app, art, dark), rect);
    }
    response
}

/// The width the [`wordmark`] takes for each point of its height.
pub fn wordmark_aspect(ui: &Ui, app: App) -> f32 {
    tree(ui.ctx(), app, Art::Wordmark, ui.visuals().dark_mode).map_or(1.0, |tree| tree.size().width() / tree.size().height())
}

/// Draws `app`'s mark in a `size` by `size` square, picking the file for the
/// current dark or light theme.
///
/// Usage:
///
/// ```ignore
/// ui_kit::brand::mark(ui, ui_kit::brand::App::Audio, ui_kit::brand::TITLE_BAR_MARK);
/// ```
pub fn mark(ui: &mut Ui, app: App, size: f32) -> Response {
    show(ui, app, Art::Mark, size)
}

/// Draws `app`'s wordmark lockup (mark, "CutMark" and the app name) `height`
/// tall, picking the file for the current dark or light theme. Screen readers
/// get the name it spells.
pub fn wordmark(ui: &mut Ui, app: App, height: f32) -> Response {
    let response = show(ui, app, Art::Wordmark, height);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, app.name()));
    response
}

/// `app`'s window and taskbar icon: the icon tile at 256 pixels. An empty icon,
/// which leaves the window's default, if the art cannot be rendered.
pub fn window_icon(app: App) -> egui::IconData {
    let Some(pixmap) = parse(files(app).icon).and_then(|tree| rasterize(&tree, ICON_PIXELS)) else {
        return egui::IconData::default();
    };
    // IconData wants straight alpha; the bitmap is premultiplied.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    egui::IconData { rgba, width: pixmap.width(), height: pixmap.height() }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// Every embedded file parses and renders something, so a bad asset fails
    /// here and not as an empty space in an app.
    #[test]
    fn every_file_renders_visible_pixels() {
        for app in App::ALL {
            let files = files(app);
            for data in [files.mark_dark, files.mark_light, files.wordmark_dark, files.wordmark_light, files.icon] {
                let pixmap = rasterize(&parse(data).expect("the SVG parses"), 64).expect("a bitmap");
                assert!(pixmap.pixels().iter().any(|pixel| pixel.alpha() == 255), "{app:?} renders nothing");
            }
        }
    }

    /// The mark is square, and the theme picks the ink: light on dark
    /// surfaces, dark on light ones.
    #[test]
    fn the_theme_picks_the_dark_or_light_file() {
        for app in App::ALL {
            for (dark, ink) in [(true, [0xED, 0xF4, 0xF3]), (false, [0x14, 0x1A, 0x19])] {
                let pixmap = rasterize(&parse(source(app, Art::Mark, dark)).expect("the SVG parses"), 64).expect("a bitmap");
                assert_eq!((pixmap.width(), pixmap.height()), (64, 64));
                // A point well inside the left (ink) piece.
                let pixel = pixmap.pixel(16, 30).expect("inside the bitmap").demultiply();
                assert_eq!([pixel.red(), pixel.green(), pixel.blue()], ink, "{app:?} dark={dark}");
            }
        }
    }

    /// The window icon is the 256 pixel tile with a transparent corner.
    #[test]
    fn the_window_icon_is_the_256_tile() {
        for app in App::ALL {
            let icon = window_icon(app);
            assert_eq!((icon.width, icon.height), (256, 256));
            assert_eq!(icon.rgba.len(), 256 * 256 * 4);
            assert_eq!(icon.rgba[3], 0, "{app:?}: the rounded corner is transparent");
            let centre = (128 * 256 + 64) * 4;
            assert_eq!(icon.rgba[centre + 3], 255, "{app:?}: the tile is opaque inside");
        }
    }
}
