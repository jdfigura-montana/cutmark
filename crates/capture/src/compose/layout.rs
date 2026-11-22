//! Pure layout math for the compositor: where the screen and the camera go in the output frame.
//!
//! Nothing here touches the GPU. Every rectangle is in output pixels, rounded to whole pixels, so
//! the same settings give the same picture at any output size. See the recorder spec: Layouts.

/// Which pictures the composite shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// Only the screen, fitted into the frame.
    #[default]
    ScreenOnly,
    /// The camera over the screen.
    Pip,
    /// Only the camera, fitted into the frame over a background colour.
    CamOnly,
}

/// The corner of the frame that a picture in picture camera is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Corner {
    /// Top left.
    TopLeft,
    /// Top right.
    TopRight,
    /// Bottom left.
    BottomLeft,
    /// Bottom right.
    #[default]
    BottomRight,
}

/// A rectangle with float edges; output pixels for a destination, 0 to 1 for a source area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectF {
    /// Left edge.
    pub left: f32,
    /// Top edge.
    pub top: f32,
    /// Right edge.
    pub right: f32,
    /// Bottom edge.
    pub bottom: f32,
}

impl RectF {
    /// A rectangle from its four edges.
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self { left, top, right, bottom }
    }

    /// Right edge minus left edge.
    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    /// Bottom edge minus top edge.
    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }
}

/// The shape of the camera picture in the picture in picture layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PipShape {
    /// A rectangle that follows the cropped camera's aspect ratio, with optional rounded corners.
    #[default]
    Rectangle,
    /// A circle cut from the largest centred square of the cropped camera.
    Circle,
}

/// How the camera looks in the picture in picture layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PipSettings {
    /// The corner of the frame the camera sits in.
    pub anchor: Corner,
    /// Distance from the left or right edge, in percent of the output width.
    pub offset_x_pct: f32,
    /// Distance from the top or bottom edge, in percent of the output height.
    pub offset_y_pct: f32,
    /// Width of the camera picture, in percent of the output width.
    pub scale_pct: f32,
    /// How much of the camera's left side is cut off, in percent.
    pub crop_left_pct: f32,
    /// How much of the camera's right side is cut off, in percent.
    pub crop_right_pct: f32,
    /// How much of the camera's top is cut off, in percent.
    pub crop_top_pct: f32,
    /// How much of the camera's bottom is cut off, in percent.
    pub crop_bottom_pct: f32,
    /// Border width in output pixels, drawn inside the camera rectangle, or as a ring outside the circle; 0 for none.
    pub border_px: f32,
    /// Border colour as red, green and blue.
    pub border_color: [u8; 3],
    /// Corner radius in output pixels; 0 for square corners. Ignored for a circle.
    pub radius_px: f32,
    /// The shape of the camera picture.
    pub shape: PipShape,
}

impl Default for PipSettings {
    fn default() -> Self {
        Self {
            anchor: Corner::BottomRight,
            offset_x_pct: 2.0,
            offset_y_pct: 2.0,
            scale_pct: 25.0,
            crop_left_pct: 0.0,
            crop_right_pct: 0.0,
            crop_top_pct: 0.0,
            crop_bottom_pct: 0.0,
            border_px: 0.0,
            border_color: [255, 255, 255],
            radius_px: 12.0,
            shape: PipShape::Rectangle,
        }
    }
}

/// The chroma key of the camera: the key colour, the three sliders, the garbage matte and an optional mask.
///
/// The crop and mask percentages are of the whole camera frame, before any picture in picture crop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeySettings {
    /// Whether the camera is keyed.
    pub enabled: bool,
    /// The colour that becomes transparent, as red, green and blue.
    pub color: [u8; 3],
    /// How far from the key colour a pixel may be and still be fully transparent, 0 to 1.
    pub similarity: f32,
    /// How wide the soft edge beyond the similarity is, 0 to 1.
    pub smoothness: f32,
    /// How strongly the key colour is pulled out of the pixels that stay, 0 to 1.
    pub spill: f32,
    /// The part of the camera's left side that is thrown away, in percent.
    pub crop_left_pct: f32,
    /// The part of the camera's right side that is thrown away, in percent.
    pub crop_right_pct: f32,
    /// The part of the camera's top that is thrown away, in percent.
    pub crop_top_pct: f32,
    /// The part of the camera's bottom that is thrown away, in percent.
    pub crop_bottom_pct: f32,
    /// Whether the mask rectangle is cut out of the picture.
    pub mask_enabled: bool,
    /// Left edge of the mask rectangle, in percent of the camera frame's width.
    pub mask_x_pct: f32,
    /// Top edge of the mask rectangle, in percent of the camera frame's height.
    pub mask_y_pct: f32,
    /// Width of the mask rectangle, in percent of the camera frame's width.
    pub mask_w_pct: f32,
    /// Height of the mask rectangle, in percent of the camera frame's height.
    pub mask_h_pct: f32,
}

impl Default for KeySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            color: [0, 177, 64],
            similarity: 0.40,
            smoothness: 0.08,
            spill: 0.10,
            crop_left_pct: 0.0,
            crop_right_pct: 0.0,
            crop_top_pct: 0.0,
            crop_bottom_pct: 0.0,
            mask_enabled: false,
            mask_x_pct: 40.0,
            mask_y_pct: 40.0,
            mask_w_pct: 20.0,
            mask_h_pct: 20.0,
        }
    }
}

/// The part of the camera frame that the garbage crop keeps, as 0 to 1 texture coordinates; at least 5 percent stays.
pub fn garbage_rect(key: &KeySettings) -> RectF {
    let (left, right) = crop_axis(key.crop_left_pct, key.crop_right_pct);
    let (top, bottom) = crop_axis(key.crop_top_pct, key.crop_bottom_pct);
    RectF::new(left, top, right, bottom)
}

/// The mask rectangle as 0 to 1 texture coordinates, kept inside the frame, or None when it is off or has no area.
pub fn mask_rect(key: &KeySettings) -> Option<RectF> {
    if !key.mask_enabled {
        return None;
    }
    let x = finite(key.mask_x_pct).clamp(0.0, 100.0);
    let y = finite(key.mask_y_pct).clamp(0.0, 100.0);
    let w = finite(key.mask_w_pct).clamp(0.0, 100.0 - x);
    let h = finite(key.mask_h_pct).clamp(0.0, 100.0 - y);
    (w > 0.0 && h > 0.0).then(|| RectF::new(x / 100.0, y / 100.0, (x + w) / 100.0, (y + h) / 100.0))
}

/// Where the camera picture comes from and where it goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CamPlacement {
    /// The part of the camera texture that is shown, as 0 to 1 texture coordinates.
    pub src: RectF,
    /// The rectangle the camera fills, in output pixels.
    pub dst: RectF,
}

/// The rectangles of one composite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Where the screen goes, or None when the layout shows no screen or there is none.
    pub screen: Option<RectF>,
    /// Where the camera goes, or None when the layout shows no camera or there is none.
    pub cam: Option<CamPlacement>,
}

/// The largest centred rectangle with the source's aspect ratio that fits the output, on whole pixels.
pub fn fit_rect(src_w: u32, src_h: u32, out_w: u32, out_h: u32) -> RectF {
    if src_w == 0 || src_h == 0 || out_w == 0 || out_h == 0 {
        return RectF::new(0.0, 0.0, out_w as f32, out_h as f32);
    }
    // Compare with u64 math so wide monitors cannot overflow.
    let wide = u64::from(src_w) * u64::from(out_h) > u64::from(out_w) * u64::from(src_h);
    let (fit_w, fit_h) = if wide {
        (out_w, ((u64::from(out_w) * u64::from(src_h) + u64::from(src_w) / 2) / u64::from(src_w)).clamp(1, u64::from(out_h)) as u32)
    } else {
        (((u64::from(out_h) * u64::from(src_w) + u64::from(src_h) / 2) / u64::from(src_h)).clamp(1, u64::from(out_w)) as u32, out_h)
    };
    let left = (out_w - fit_w) / 2;
    let top = (out_h - fit_h) / 2;
    RectF::new(left as f32, top as f32, (left + fit_w) as f32, (top + fit_h) as f32)
}

/// Turns a percentage into a fraction, treating a value that is not a number as 0.
fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// The two edges of one axis of the crop as 0 to 1 coordinates; at least 5 percent of the picture stays.
fn crop_axis(near_pct: f32, far_pct: f32) -> (f32, f32) {
    let near = finite(near_pct).clamp(0.0, 90.0);
    let far = finite(far_pct).clamp(0.0, 95.0 - near);
    (near / 100.0, 1.0 - far / 100.0)
}

/// The part of the camera texture a picture in picture shows, as 0 to 1 texture coordinates.
pub fn cam_crop(pip: &PipSettings) -> RectF {
    let (left, right) = crop_axis(pip.crop_left_pct, pip.crop_right_pct);
    let (top, bottom) = crop_axis(pip.crop_top_pct, pip.crop_bottom_pct);
    RectF::new(left, top, right, bottom)
}

/// The cropped camera source and its destination rectangle for a picture in picture.
///
/// The width is the scale percentage of the output width; the height follows the cropped camera's
/// aspect ratio. A picture that would not fit is shrunk, and one that sticks out is moved back in.
///
/// For a circle the destination is a square as wide as the scale percentage of the output width (at most
/// the output height), and the source is the largest centred square inside the cropped camera region.
pub fn pip_placement(cam_w: u32, cam_h: u32, pip: &PipSettings, out_w: u32, out_h: u32) -> CamPlacement {
    let crop = cam_crop(pip);
    let out_wf = out_w as f32;
    let out_hf = out_h as f32;
    let cam_wf = (cam_w as f32).max(1.0);
    let cam_hf = (cam_h as f32).max(1.0);
    let crop_w = (cam_wf * crop.width()).max(1.0);
    let crop_h = (cam_hf * crop.height()).max(1.0);
    let mut width = (out_wf * finite(pip.scale_pct).clamp(1.0, 100.0) / 100.0).max(1.0);
    let mut height = width * crop_h / crop_w;
    let src = match pip.shape {
        PipShape::Rectangle => {
            if height > out_hf {
                height = out_hf;
                width = height * crop_w / crop_h;
            }
            crop
        }
        PipShape::Circle => {
            let diameter = width.min(out_wf.min(out_hf).max(1.0));
            width = diameter;
            height = diameter;
            let side = crop_w.min(crop_h);
            let half_u = side / cam_wf / 2.0;
            let half_v = side / cam_hf / 2.0;
            let centre_u = (crop.left + crop.right) / 2.0;
            let centre_v = (crop.top + crop.bottom) / 2.0;
            RectF::new(centre_u - half_u, centre_v - half_v, centre_u + half_u, centre_v + half_v)
        }
    };
    let width = width.round().clamp(1.0, out_wf.max(1.0));
    let height = height.round().clamp(1.0, out_hf.max(1.0));
    let offset_x = out_wf * finite(pip.offset_x_pct) / 100.0;
    let offset_y = out_hf * finite(pip.offset_y_pct) / 100.0;
    let (left, top) = match pip.anchor {
        Corner::TopLeft => (offset_x, offset_y),
        Corner::TopRight => (out_wf - offset_x - width, offset_y),
        Corner::BottomLeft => (offset_x, out_hf - offset_y - height),
        Corner::BottomRight => (out_wf - offset_x - width, out_hf - offset_y - height),
    };
    let left = left.clamp(0.0, (out_wf - width).max(0.0)).round();
    let top = top.clamp(0.0, (out_hf - height).max(0.0)).round();
    CamPlacement { src, dst: RectF::new(left, top, left + width, top + height) }
}

/// The rectangles for a layout, from the sizes of the pictures that exist.
///
/// A picture that is missing gets no rectangle, so the composite draws only what is available.
pub fn place(layout: Layout, screen: Option<(u32, u32)>, cam: Option<(u32, u32)>, pip: &PipSettings, out_w: u32, out_h: u32) -> Placement {
    let screen = screen.filter(|&(w, h)| w > 0 && h > 0);
    let cam = cam.filter(|&(w, h)| w > 0 && h > 0);
    match layout {
        Layout::ScreenOnly => Placement { screen: screen.map(|(w, h)| fit_rect(w, h, out_w, out_h)), cam: None },
        Layout::Pip => Placement { screen: screen.map(|(w, h)| fit_rect(w, h, out_w, out_h)), cam: cam.map(|(w, h)| pip_placement(w, h, pip, out_w, out_h)) },
        Layout::CamOnly => {
            Placement { screen: None, cam: cam.map(|(w, h)| CamPlacement { src: RectF::new(0.0, 0.0, 1.0, 1.0), dst: fit_rect(w, h, out_w, out_h) }) }
        }
    }
}

/// What a layout draws behind the pictures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundKind {
    /// Nothing; a keyed camera floats over the screen, and the Cam only layout uses the colour.
    #[default]
    None,
    /// A solid colour.
    Color,
    /// A PNG or JPG image from a file.
    Image,
}

/// How an image background fills the rectangle it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundFit {
    /// Fills the rectangle and crops whatever does not fit.
    #[default]
    Cover,
    /// Fits whole inside the rectangle, letterboxed with the background colour.
    Contain,
    /// Stretches over the rectangle, ignoring the aspect ratio.
    Stretch,
}

/// The background settings of a layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundSettings {
    /// What the background shows.
    pub kind: BackgroundKind,
    /// The solid colour as red, green and blue; also the letterbox colour of Contain.
    pub color: [u8; 3],
    /// The path of the image file, used when the kind is Image.
    pub image_path: String,
    /// How an image fills its rectangle.
    pub fit: BackgroundFit,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self { kind: BackgroundKind::None, color: [32, 32, 32], image_path: String::new(), fit: BackgroundFit::Cover }
    }
}

/// The source and destination rectangles of a background image fitted into a target rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundPlacement {
    /// The part of the image that is shown, as 0 to 1 texture coordinates.
    pub src: RectF,
    /// The part of the target the image fills, in output pixels.
    pub dst: RectF,
}

/// Fits a source image into `target` for the given fit: Cover fills and crops, Contain fits with a
/// letterbox that the caller paints in the background colour, and Stretch fills without cropping.
pub fn fit_background(src_w: u32, src_h: u32, target: RectF, fit: BackgroundFit) -> BackgroundPlacement {
    let whole = RectF::new(0.0, 0.0, 1.0, 1.0);
    let (target_w, target_h) = (target.width(), target.height());
    if src_w == 0 || src_h == 0 || target_w <= 0.0 || target_h <= 0.0 {
        return BackgroundPlacement { src: whole, dst: target };
    }
    let (src_wf, src_hf) = (src_w as f32, src_h as f32);
    match fit {
        BackgroundFit::Stretch => BackgroundPlacement { src: whole, dst: target },
        BackgroundFit::Cover => {
            let scale = (target_w / src_wf).max(target_h / src_hf);
            let keep_w = (target_w / scale / src_wf).min(1.0);
            let keep_h = (target_h / scale / src_hf).min(1.0);
            let left = (1.0 - keep_w) / 2.0;
            let top = (1.0 - keep_h) / 2.0;
            BackgroundPlacement { src: RectF::new(left, top, left + keep_w, top + keep_h), dst: target }
        }
        BackgroundFit::Contain => {
            let scale = (target_w / src_wf).min(target_h / src_hf);
            let width = (src_wf * scale).min(target_w);
            let height = (src_hf * scale).min(target_h);
            let left = target.left + (target_w - width) / 2.0;
            let top = target.top + (target_h - height) / 2.0;
            BackgroundPlacement { src: whole, dst: RectF::new(left, top, left + width, top + height) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> RectF {
        RectF::new(left, top, right, bottom)
    }

    #[test]
    fn a_screen_of_the_output_aspect_fills_the_frame() {
        assert_eq!(fit_rect(2560, 1440, 1920, 1080), rect(0.0, 0.0, 1920.0, 1080.0));
    }

    #[test]
    fn an_ultrawide_screen_is_letterboxed_top_and_bottom() {
        // 3440x1440 in 1920x1080: 1920 wide, 804 high (803.5 rounds up), centred.
        let fitted = fit_rect(3440, 1440, 1920, 1080);
        assert_eq!((fitted.left, fitted.right), (0.0, 1920.0));
        assert_eq!(fitted.height(), 804.0);
        assert_eq!(fitted.top, 138.0);
    }

    #[test]
    fn a_tall_screen_is_pillarboxed_left_and_right() {
        let fitted = fit_rect(1080, 1920, 1920, 1080);
        assert_eq!((fitted.top, fitted.bottom), (0.0, 1080.0));
        assert_eq!(fitted.width(), 608.0);
        assert_eq!(fitted.left, 656.0);
    }

    #[test]
    fn a_zero_size_falls_back_to_the_whole_frame() {
        assert_eq!(fit_rect(0, 0, 1280, 720), rect(0.0, 0.0, 1280.0, 720.0));
    }

    #[test]
    fn the_default_pip_sits_bottom_right_at_a_quarter_of_the_width() {
        let placed = pip_placement(1920, 1080, &PipSettings::default(), 1920, 1080);
        // 25 percent of 1920 is 480 wide and 270 high; the 2 percent offsets are 38.4 and 21.6 pixels.
        assert_eq!(placed.dst.width(), 480.0);
        assert_eq!(placed.dst.height(), 270.0);
        assert_eq!(placed.dst.right, 1882.0);
        assert_eq!(placed.dst.bottom, 1058.0);
        assert_eq!(placed.src, rect(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn the_same_settings_scale_with_the_output_size() {
        let small = pip_placement(1280, 720, &PipSettings::default(), 1280, 720);
        let large = pip_placement(1280, 720, &PipSettings::default(), 2560, 1440);
        assert_eq!(large.dst.width(), small.dst.width() * 2.0);
        assert_eq!(large.dst.height(), small.dst.height() * 2.0);
        assert!((large.dst.left - small.dst.left * 2.0).abs() <= 1.0);
        assert!((large.dst.top - small.dst.top * 2.0).abs() <= 1.0);
    }

    #[test]
    fn every_corner_keeps_its_offsets() {
        let mut pip = PipSettings { scale_pct: 20.0, offset_x_pct: 5.0, offset_y_pct: 10.0, ..PipSettings::default() };
        // 20 percent of 1000 is 200 wide, 112.5 high which rounds to 113; offsets are 50 and 100.
        pip.anchor = Corner::TopLeft;
        let top_left = pip_placement(1600, 900, &pip, 1000, 1000).dst;
        assert_eq!((top_left.left, top_left.top), (50.0, 100.0));
        pip.anchor = Corner::TopRight;
        let top_right = pip_placement(1600, 900, &pip, 1000, 1000).dst;
        assert_eq!((top_right.right, top_right.top), (950.0, 100.0));
        pip.anchor = Corner::BottomLeft;
        let bottom_left = pip_placement(1600, 900, &pip, 1000, 1000).dst;
        assert_eq!((bottom_left.left, bottom_left.bottom), (50.0, 900.0));
        pip.anchor = Corner::BottomRight;
        let bottom_right = pip_placement(1600, 900, &pip, 1000, 1000).dst;
        assert_eq!((bottom_right.right, bottom_right.bottom), (950.0, 900.0));
    }

    #[test]
    fn a_crop_changes_the_source_area_and_the_aspect_ratio() {
        let pip = PipSettings { crop_left_pct: 25.0, crop_right_pct: 25.0, scale_pct: 20.0, ..PipSettings::default() };
        let placed = pip_placement(1920, 1080, &pip, 1920, 1080);
        assert_eq!(placed.src, rect(0.25, 0.0, 0.75, 1.0));
        // The cropped camera is 960x1080, so a 384 pixel wide picture is 432 high.
        assert_eq!(placed.dst.width(), 384.0);
        assert_eq!(placed.dst.height(), 432.0);
    }

    #[test]
    fn a_crop_always_leaves_part_of_the_picture() {
        let pip = PipSettings { crop_left_pct: 200.0, crop_right_pct: 200.0, crop_top_pct: f32::NAN, crop_bottom_pct: -5.0, ..PipSettings::default() };
        let src = cam_crop(&pip);
        assert!(src.width() >= 0.049 && src.height() > 0.99, "{src:?}");
        assert!(src.left >= 0.0 && src.right <= 1.0);
    }

    #[test]
    fn a_picture_that_is_too_tall_is_shrunk_to_fit() {
        // A 100 percent wide, very tall crop would be taller than the frame.
        let pip = PipSettings { scale_pct: 100.0, crop_left_pct: 0.0, crop_right_pct: 0.0, ..PipSettings::default() };
        let placed = pip_placement(1080, 1920, &pip, 1920, 1080);
        assert_eq!(placed.dst.height(), 1080.0);
        assert!(placed.dst.width() < 1920.0);
        assert!(placed.dst.top >= 0.0 && placed.dst.bottom <= 1080.0);
    }

    #[test]
    fn huge_offsets_keep_the_picture_inside_the_frame() {
        let pip = PipSettings { offset_x_pct: 90.0, offset_y_pct: 90.0, ..PipSettings::default() };
        let placed = pip_placement(1920, 1080, &pip, 1920, 1080).dst;
        assert!(placed.left >= 0.0 && placed.top >= 0.0 && placed.right <= 1920.0 && placed.bottom <= 1080.0);
    }

    fn circle(scale_pct: f32) -> PipSettings {
        PipSettings { shape: PipShape::Circle, scale_pct, radius_px: 40.0, border_px: 6.0, ..PipSettings::default() }
    }

    fn assert_close(actual: RectF, expected: RectF) {
        let pairs = [(actual.left, expected.left), (actual.top, expected.top), (actual.right, expected.right), (actual.bottom, expected.bottom)];
        for (a, e) in pairs {
            assert!((a - e).abs() < 1e-5, "{actual:?} vs {expected:?}");
        }
    }

    #[test]
    fn a_circle_at_1080p_is_a_square_cut_from_the_middle_of_the_camera() {
        let placed = pip_placement(1920, 1080, &circle(25.0), 1920, 1080);
        // The diameter is 25 percent of 1920; the largest centred square of a 1920x1080 camera is 1080 wide.
        assert_eq!(placed.dst, rect(1402.0, 578.0, 1882.0, 1058.0));
        assert_close(placed.src, rect(0.21875, 0.0, 0.78125, 1.0));
    }

    #[test]
    fn a_circle_at_1440p_scales_with_the_output() {
        let placed = pip_placement(1920, 1080, &circle(25.0), 2560, 1440);
        assert_eq!(placed.dst, rect(1869.0, 771.0, 2509.0, 1411.0));
        assert_close(placed.src, rect(0.21875, 0.0, 0.78125, 1.0));
    }

    #[test]
    fn a_circle_takes_its_square_from_inside_the_crop() {
        // Cutting 25 percent from each side leaves 960x1080, so the square is 960 wide and 44.4 percent
        // of the height either side of the middle.
        let pip = PipSettings { crop_left_pct: 25.0, crop_right_pct: 25.0, ..circle(25.0) };
        let placed = pip_placement(1920, 1080, &pip, 1920, 1080);
        assert_eq!(placed.dst, rect(1402.0, 578.0, 1882.0, 1058.0));
        assert_close(placed.src, rect(0.25, 0.5 - 480.0 / 1080.0, 0.75, 0.5 + 480.0 / 1080.0));
        // Cutting 10 percent from the top and bottom leaves 1920x864, so the square is 864 wide.
        let pip = PipSettings { crop_top_pct: 10.0, crop_bottom_pct: 10.0, ..circle(25.0) };
        let placed = pip_placement(1920, 1080, &pip, 2560, 1440);
        assert_eq!(placed.dst, rect(1869.0, 771.0, 2509.0, 1411.0));
        assert_close(placed.src, rect(0.5 - 432.0 / 1920.0, 0.1, 0.5 + 432.0 / 1920.0, 0.9));
    }

    #[test]
    fn a_circle_never_grows_past_the_frame_height() {
        let placed = pip_placement(1920, 1080, &circle(100.0), 1920, 1080).dst;
        assert_eq!((placed.width(), placed.height()), (1080.0, 1080.0));
        assert!(placed.left >= 0.0 && placed.right <= 1920.0);
    }

    #[test]
    fn screen_only_ignores_the_camera() {
        let placed = place(Layout::ScreenOnly, Some((1920, 1080)), Some((1280, 720)), &PipSettings::default(), 1920, 1080);
        assert!(placed.screen.is_some());
        assert!(placed.cam.is_none());
    }

    #[test]
    fn pip_draws_whatever_is_available() {
        let pip = PipSettings::default();
        let both = place(Layout::Pip, Some((1920, 1080)), Some((1280, 720)), &pip, 1920, 1080);
        assert!(both.screen.is_some() && both.cam.is_some());
        let screen_only = place(Layout::Pip, Some((1920, 1080)), None, &pip, 1920, 1080);
        assert!(screen_only.screen.is_some() && screen_only.cam.is_none());
        let cam_only = place(Layout::Pip, None, Some((1280, 720)), &pip, 1920, 1080);
        assert!(cam_only.screen.is_none() && cam_only.cam.is_some());
    }

    #[test]
    fn cam_only_fits_the_whole_camera_into_the_frame() {
        let placed = place(Layout::CamOnly, Some((1920, 1080)), Some((640, 480)), &PipSettings::default(), 1920, 1080);
        assert!(placed.screen.is_none());
        let cam = placed.cam;
        assert_eq!(cam.map(|c| c.src), Some(rect(0.0, 0.0, 1.0, 1.0)));
        // 4:3 in 16:9: full height, 1440 wide, centred.
        assert_eq!(cam.map(|c| c.dst), Some(rect(240.0, 0.0, 1680.0, 1080.0)));
    }

    #[test]
    fn no_garbage_crop_keeps_the_whole_frame() {
        assert_eq!(garbage_rect(&KeySettings::default()), rect(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn the_garbage_crop_cuts_each_side_by_its_percentage() {
        let key = KeySettings { crop_left_pct: 10.0, crop_right_pct: 20.0, crop_top_pct: 5.0, crop_bottom_pct: 30.0, ..KeySettings::default() };
        assert_close(garbage_rect(&key), rect(0.10, 0.05, 0.80, 0.70));
    }

    #[test]
    fn the_garbage_crop_always_leaves_part_of_the_frame() {
        let key = KeySettings { crop_left_pct: 200.0, crop_right_pct: 200.0, crop_top_pct: f32::NAN, crop_bottom_pct: -5.0, ..KeySettings::default() };
        let kept = garbage_rect(&key);
        assert!(kept.width() >= 0.049 && kept.height() > 0.99, "{kept:?}");
        assert!(kept.left >= 0.0 && kept.right <= 1.0);
    }

    #[test]
    fn a_mask_that_is_off_cuts_nothing() {
        assert_eq!(mask_rect(&KeySettings::default()), None);
    }

    #[test]
    fn a_mask_is_given_as_a_fraction_of_the_frame() {
        let key = KeySettings { mask_enabled: true, mask_x_pct: 10.0, mask_y_pct: 20.0, mask_w_pct: 30.0, mask_h_pct: 40.0, ..KeySettings::default() };
        assert_eq!(mask_rect(&key).map(|mask| (mask.left, mask.top)), Some((0.10, 0.20)));
        let mask = mask_rect(&key).unwrap_or(rect(0.0, 0.0, 0.0, 0.0));
        assert_close(mask, rect(0.10, 0.20, 0.40, 0.60));
    }

    #[test]
    fn a_mask_that_sticks_out_is_cut_at_the_frame_edge() {
        let key = KeySettings { mask_enabled: true, mask_x_pct: 80.0, mask_y_pct: 90.0, mask_w_pct: 50.0, mask_h_pct: 50.0, ..KeySettings::default() };
        let mask = mask_rect(&key).unwrap_or(rect(0.0, 0.0, 0.0, 0.0));
        assert_close(mask, rect(0.80, 0.90, 1.0, 1.0));
    }

    #[test]
    fn a_mask_without_area_or_with_junk_values_cuts_nothing() {
        let flat = KeySettings { mask_enabled: true, mask_w_pct: 0.0, ..KeySettings::default() };
        assert_eq!(mask_rect(&flat), None);
        let junk = KeySettings { mask_enabled: true, mask_x_pct: f32::NAN, mask_y_pct: 100.0, ..KeySettings::default() };
        assert_eq!(mask_rect(&junk), None);
    }

    #[test]
    fn a_picture_of_size_zero_counts_as_missing() {
        let placed = place(Layout::Pip, Some((0, 0)), Some((0, 720)), &PipSettings::default(), 1920, 1080);
        assert!(placed.screen.is_none() && placed.cam.is_none());
    }

    fn target() -> RectF {
        rect(100.0, 50.0, 700.0, 410.0)
    }

    #[test]
    fn cover_fills_the_target_and_crops_the_long_source_axis() {
        // A 2000x1000 source in a 600x360 target: the target is 0.3 of the source width but 0.36 of
        // the height, so the height fills and the sides of the source are cropped away.
        let placed = fit_background(2000, 1000, target(), BackgroundFit::Cover);
        assert_eq!(placed.dst, target());
        let keep_w = 600.0 / (600.0_f32 / 2000.0).max(360.0 / 1000.0) / 2000.0;
        assert_close(placed.src, rect((1.0 - keep_w) / 2.0, 0.0, (1.0 + keep_w) / 2.0, 1.0));
    }

    #[test]
    fn cover_crops_the_other_axis_when_the_source_is_tall() {
        // A 1000x2000 source: the width fills and the top and bottom are cropped away.
        let placed = fit_background(1000, 2000, target(), BackgroundFit::Cover);
        assert_eq!(placed.dst, target());
        let keep_h = 360.0 / (600.0_f32 / 1000.0).max(360.0 / 2000.0) / 2000.0;
        assert_close(placed.src, rect(0.0, (1.0 - keep_h) / 2.0, 1.0, (1.0 + keep_h) / 2.0));
    }

    #[test]
    fn contain_fits_the_whole_picture_and_letterboxes_the_rest() {
        // A 2000x1000 source in a 600x360 target fits at 0.18 of the source, 360 high and 720 wide,
        // but at most 600 wide, so it is 600x300 and centred with 30 pixels of letterbox each way.
        let placed = fit_background(2000, 1000, target(), BackgroundFit::Contain);
        assert_eq!(placed.src, rect(0.0, 0.0, 1.0, 1.0));
        assert_eq!(placed.dst, rect(100.0, 80.0, 700.0, 380.0));
    }

    #[test]
    fn stretch_fills_the_target_and_keeps_the_whole_picture() {
        let placed = fit_background(2000, 1000, target(), BackgroundFit::Stretch);
        assert_eq!(placed.src, rect(0.0, 0.0, 1.0, 1.0));
        assert_eq!(placed.dst, target());
    }

    #[test]
    fn a_degenerate_source_or_target_crops_nothing() {
        let zero_source = fit_background(0, 100, target(), BackgroundFit::Cover);
        assert_eq!(zero_source.src, rect(0.0, 0.0, 1.0, 1.0));
        assert_eq!(zero_source.dst, target());
        let empty_target = fit_background(2000, 1000, rect(10.0, 10.0, 10.0, 10.0), BackgroundFit::Stretch);
        assert_eq!(empty_target.src, rect(0.0, 0.0, 1.0, 1.0));
        assert_eq!(empty_target.dst, rect(10.0, 10.0, 10.0, 10.0));
    }

    #[test]
    fn the_background_defaults_are_none_over_dark_grey() {
        let settings = BackgroundSettings::default();
        assert_eq!(settings.kind, BackgroundKind::None);
        assert_eq!(settings.color, [32, 32, 32]);
        assert_eq!(settings.fit, BackgroundFit::Cover);
        assert_eq!(settings.image_path, "");
    }
}
