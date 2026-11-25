//! The `composite` block of a take's session.json: what is in the main video and how an editor rebuilds the look.
//!
//! The schema is "The composite block in session.json" in docs/suite/recorder-composite-mode.md. The camera's
//! rectangle comes from [`capture::place`], the same math the compositor draws with.

use serde::{Deserialize, Serialize};

use crate::config::{AnchorCorner, BackgroundFit, BackgroundKind, CompositeMode, LayoutKind, PipShape, Profile};

/// The schema version; it changes only on a breaking change.
const VERSION: u32 = 1;

/// The `composite` object of session.json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompositeBlock {
    /// The schema version.
    pub version: u32,
    /// The mode the take records in, after the profile's layout and webcam are taken into account.
    pub mode: CompositeMode,
    /// The output frame every rectangle is a fraction of.
    pub canvas: Canvas,
    /// The main video file.
    pub main: MainVideo,
    /// The layers over the main source; empty when the look has no camera.
    pub layers: Vec<Layer>,
    /// What is drawn behind the camera.
    pub background: Background,
}

/// The size and frame rate of the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canvas {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frames per second.
    pub fps: u32,
}

/// The main video of the take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainVideo {
    /// What the main video records.
    pub source: String,
    /// The file name, relative to the take folder.
    pub file: String,
}

/// One layer over the main source, as the compositor places it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// What the layer shows.
    pub source: String,
    /// The layer's own file, relative to the take folder; none for a baked camera with no camera file.
    pub file: Option<String>,
    /// Whether the layer is already in the main video.
    pub baked: bool,
    /// The camera's sync offset in ms.
    pub offset_ms: i32,
    /// The placed box, as fractions of the canvas.
    pub rect: Area,
    /// The corner the picture in picture is anchored to.
    pub anchor: AnchorCorner,
    /// How much of each side of the source frame is cut off, as fractions.
    pub crop: Edges,
    /// The shape of the layer.
    pub shape: PipShape,
    /// Corner radius in canvas pixels; 0 for a circle.
    pub corner_radius_px: u32,
    /// The layer's border.
    pub border: Border,
    /// The chroma key.
    pub key: Key,
}

/// A rectangle as fractions: the left and top edge, the width and the height.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Area {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

/// The part cut from each side of a source frame, as fractions of that frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Edges {
    /// Cut from the left.
    pub left: f64,
    /// Cut from the right.
    pub right: f64,
    /// Cut from the top.
    pub top: f64,
    /// Cut from the bottom.
    pub bottom: f64,
}

/// The border of a layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Border {
    /// Width in canvas pixels; 0 for none.
    pub width_px: u32,
    /// Colour as `#RRGGBB`.
    pub color: String,
}

/// The chroma key of a layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Whether the layer is keyed.
    pub enabled: bool,
    /// The key colour as `#RRGGBB`.
    pub color: String,
    /// How far from the key colour a pixel may be and still be transparent, 0 to 1.
    pub similarity: f64,
    /// The width of the soft edge, 0 to 1.
    pub smoothness: f64,
    /// How strongly the key colour is pulled out of the pixels that stay, 0 to 1.
    pub spill: f64,
    /// The garbage crop of the source frame.
    pub garbage_crop: Edges,
    /// The mask rectangle in fractions of the source frame, or none.
    pub mask: Option<Area>,
}

/// What is drawn behind the camera.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Background {
    /// Nothing.
    None,
    /// A solid colour.
    Color {
        /// The colour as `#RRGGBB`.
        color: String,
    },
    /// An image file.
    Image {
        /// The absolute path at record time.
        path: String,
        /// How the image fills its rectangle.
        fit: BackgroundFit,
    },
    /// A looping video file.
    Video {
        /// The absolute path at record time.
        path: String,
        /// How the video fills its rectangle.
        fit: BackgroundFit,
    },
}

impl CompositeBlock {
    /// The block of a take that starts now.
    ///
    /// `profile` has its output size resolved. `cam_size` is the open camera's frame size, or none when no camera
    /// is open. `main_file` and `cam_file` are the take's file names; `cam_file` is none when no camera file records.
    ///
    /// The camera layer is listed when the layout shows the camera, the profile's webcam is on and the camera is
    /// open. A Clean take also needs the camera file, since the layer is nowhere else.
    pub fn new(profile: &Profile, cam_size: Option<(u32, u32)>, main_file: &str, cam_file: Option<&str>) -> Self {
        let mode = profile.effective_composite_mode();
        let baked = mode == CompositeMode::Baked;
        let layer = cam_size
            .filter(|_| profile.layout != LayoutKind::ScreenOnly && (baked || cam_file.is_some()))
            .and_then(|size| cam_layer(profile, size, cam_file, baked));
        Self {
            version: VERSION,
            mode,
            canvas: Canvas { width: profile.width, height: profile.height, fps: profile.fps },
            main: MainVideo { source: "screen".to_owned(), file: main_file.to_owned() },
            layers: layer.into_iter().collect(),
            background: background(profile),
        }
    }
}

/// The camera's layer, or none when the profile has no webcam or the compositor places no camera.
fn cam_layer(profile: &Profile, cam_size: (u32, u32), cam_file: Option<&str>, baked: bool) -> Option<Layer> {
    let webcam = profile.webcam.as_ref()?;
    let pip = capture::PipSettings::from(&profile.pip);
    let key = capture::KeySettings::from(&webcam.key);
    let (out_w, out_h) = (profile.width, profile.height);
    let placed = capture::place(profile.layout.into(), None, Some(cam_size), &pip, out_w, out_h).cam?;
    let (canvas_w, canvas_h) = (f64::from(out_w.max(1)), f64::from(out_h.max(1)));
    let rect = Area {
        x: f64::from(placed.dst.left) / canvas_w,
        y: f64::from(placed.dst.top) / canvas_h,
        w: f64::from(placed.dst.width()) / canvas_w,
        h: f64::from(placed.dst.height()) / canvas_h,
    };
    // Only the picture in picture crops the camera; the Cam only layout shows the whole frame, with the same shape and border.
    let crop = match profile.layout {
        LayoutKind::Pip => edges(capture::cam_crop(&pip)),
        LayoutKind::ScreenOnly | LayoutKind::CamOnly => Edges { left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 },
    };
    let mask =
        capture::mask_rect(&key).map(|mask| Area { x: fraction(mask.left), y: fraction(mask.top), w: fraction(mask.width()), h: fraction(mask.height()) });
    Some(Layer {
        source: "cam".to_owned(),
        file: cam_file.map(str::to_owned),
        baked,
        offset_ms: webcam.offset_clamped(),
        rect,
        anchor: profile.pip.anchor,
        crop,
        shape: profile.pip.shape,
        corner_radius_px: match profile.pip.shape {
            PipShape::Rectangle => pixels(pip.radius_px),
            PipShape::Circle => 0,
        },
        border: Border { width_px: pixels(pip.border_px), color: hex(pip.border_color) },
        key: Key {
            enabled: key.enabled,
            color: hex(key.color),
            similarity: fraction(key.similarity),
            smoothness: fraction(key.smoothness),
            spill: fraction(key.spill),
            garbage_crop: edges(capture::garbage_rect(&key)),
            mask,
        },
    })
}

/// The profile's background.
fn background(profile: &Profile) -> Background {
    let config = &profile.background;
    match config.kind {
        BackgroundKind::None => Background::None,
        BackgroundKind::Color => Background::Color { color: hex(config.color) },
        BackgroundKind::Image => Background::Image { path: absolute(&config.image_path), fit: config.fit },
        BackgroundKind::Video => Background::Video { path: absolute(&config.video_path), fit: config.fit },
    }
}

/// The absolute form of a path; a path that has none, such as an empty one, stays as it is.
fn absolute(path: &str) -> String {
    std::path::absolute(path).map_or_else(|_| path.to_owned(), |full| full.to_string_lossy().into_owned())
}

/// What is cut from each side to leave the kept area, given as 0 to 1 coordinates.
fn edges(kept: capture::RectF) -> Edges {
    Edges { left: fraction(kept.left), right: fraction(1.0 - kept.right), top: fraction(kept.top), bottom: fraction(1.0 - kept.bottom) }
}

/// A 0 to 1 value rounded to six places, so the file shows 0.22 and not the f32's 0.2199999988; not a number is 0.
fn fraction(value: f32) -> f64 {
    if value.is_finite() { (f64::from(value) * 1e6).round() / 1e6 } else { 0.0 }
}

/// A length in whole pixels, never below 0.
fn pixels(value: f32) -> u32 {
    value.round().max(0.0) as u32
}

/// A colour as `#RRGGBB`.
fn hex([red, green, blue]: [u8; 3]) -> String {
    format!("#{red:02X}{green:02X}{blue:02X}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackgroundConfig, WebcamSettings};

    const CAM: (u32, u32) = (1920, 1080);

    /// A picture in picture profile at 2560x1440 with a webcam.
    fn pip_profile(composite_mode: CompositeMode) -> Profile {
        Profile { layout: LayoutKind::Pip, webcam: Some(WebcamSettings::default()), composite_mode, ..Profile::tutorial_av1_1440p60() }
    }

    fn block(profile: &Profile) -> CompositeBlock {
        CompositeBlock::new(profile, Some(CAM), "take.mkv", Some("take_cam.mkv"))
    }

    #[test]
    fn the_rect_matches_the_compositors_placed_rect() -> Result<(), &'static str> {
        for anchor in [AnchorCorner::BottomRight, AnchorCorner::TopLeft, AnchorCorner::BottomLeft] {
            for scale_pct in [12.0, 30.0] {
                let mut profile = pip_profile(CompositeMode::Clean);
                profile.pip.anchor = anchor;
                profile.pip.scale_pct = scale_pct;
                let rect = block(&profile).layers.first().ok_or("no camera layer")?.rect;
                let placed = capture::pip_placement(CAM.0, CAM.1, &(&profile.pip).into(), profile.width, profile.height).dst;
                let (w, h) = (f64::from(profile.width), f64::from(profile.height));
                let expected = [f64::from(placed.left) / w, f64::from(placed.top) / h, f64::from(placed.width()) / w, f64::from(placed.height()) / h];
                for (got, want) in [rect.x, rect.y, rect.w, rect.h].into_iter().zip(expected) {
                    assert!((got - want).abs() < 0.001, "{anchor:?} at {scale_pct}: {rect:?}, want {expected:?}");
                }
                // The default offsets are 2 percent, so the anchored edges sit that far in.
                let left = if anchor == AnchorCorner::BottomRight { 1.0 - 0.02 - f64::from(scale_pct) / 100.0 } else { 0.02 };
                let top = if anchor == AnchorCorner::TopLeft { 0.02 } else { 1.0 - 0.02 - rect.h };
                assert!((rect.x - left).abs() < 0.001 && (rect.y - top).abs() < 0.001, "{anchor:?} at {scale_pct}: {rect:?}");
            }
        }
        Ok(())
    }

    #[test]
    fn a_clean_take_is_not_baked_and_a_baked_take_is() {
        for (mode, baked) in [(CompositeMode::Clean, false), (CompositeMode::Baked, true)] {
            let block = block(&pip_profile(mode));
            assert_eq!(block.mode, mode);
            assert_eq!(block.layers.iter().map(|layer| layer.baked).collect::<Vec<_>>(), [baked]);
            assert_eq!(block.main.file, "take.mkv");
            assert_eq!(block.layers.first().and_then(|layer| layer.file.as_deref()), Some("take_cam.mkv"));
        }
    }

    #[test]
    fn a_webcam_that_is_off_or_absent_gives_no_layers() {
        for mode in [CompositeMode::Baked, CompositeMode::Clean] {
            let off = Profile { webcam: None, ..pip_profile(mode) };
            assert!(block(&off).layers.is_empty());
            assert!(CompositeBlock::new(&pip_profile(mode), None, "take.mkv", None).layers.is_empty());
        }
    }

    #[test]
    fn a_camera_with_no_file_is_listed_only_when_it_is_baked() {
        let baked = CompositeBlock::new(&pip_profile(CompositeMode::Baked), Some(CAM), "take.mkv", None);
        assert_eq!(baked.layers.iter().map(|layer| layer.file.clone()).collect::<Vec<_>>(), [None]);
        assert!(CompositeBlock::new(&pip_profile(CompositeMode::Clean), Some(CAM), "take.mkv", None).layers.is_empty());
    }

    #[test]
    fn the_screen_only_layout_has_no_layers_and_cam_only_has_no_crop() {
        let screen = Profile { layout: LayoutKind::ScreenOnly, ..pip_profile(CompositeMode::Baked) };
        assert!(block(&screen).layers.is_empty());
        let mut cam_only = Profile { layout: LayoutKind::CamOnly, ..pip_profile(CompositeMode::Clean) };
        cam_only.pip.crop_left_pct = 20.0;
        let block = block(&cam_only);
        assert_eq!(block.mode, CompositeMode::Baked);
        assert_eq!(block.layers.first().map(|layer| (layer.baked, layer.rect, layer.crop.left)), Some((true, Area { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, 0.0)));
    }

    #[test]
    fn the_layer_and_the_background_come_from_the_profile() -> Result<(), Box<dyn std::error::Error>> {
        let mut profile = pip_profile(CompositeMode::Clean);
        profile.pip.crop_left_pct = 22.0;
        profile.pip.crop_right_pct = 6.0;
        profile.pip.shape = PipShape::Circle;
        profile.pip.border_px = 5.0;
        profile.pip.border_color = [0x3D, 0x4B, 0xFF];
        let mut webcam = WebcamSettings { offset_ms: -50, ..WebcamSettings::default() };
        webcam.key.crop_top_pct = 10.0;
        webcam.key.mask_enabled = true;
        profile.webcam = Some(webcam);
        profile.background = BackgroundConfig { kind: BackgroundKind::Video, video_path: "loop.mp4".to_owned(), ..BackgroundConfig::default() };
        let value = serde_json::to_value(block(&profile))?;
        let layer = &value["layers"][0];
        assert_eq!(layer["offset_ms"], -50);
        assert_eq!(layer["anchor"], "bottom_right");
        assert_eq!(layer["shape"], "circle");
        assert_eq!(layer["corner_radius_px"], 0);
        assert_eq!(layer["crop"], serde_json::json!({ "left": 0.22, "right": 0.06, "top": 0.0, "bottom": 0.0 }));
        assert_eq!(layer["border"], serde_json::json!({ "width_px": 5, "color": "#3D4BFF" })); // style-lint: allow test fixture color, the profile value under test
        assert_eq!(layer["key"]["garbage_crop"], serde_json::json!({ "left": 0.0, "right": 0.0, "top": 0.1, "bottom": 0.0 }));
        assert_eq!(layer["key"]["mask"], serde_json::json!({ "x": 0.4, "y": 0.4, "w": 0.2, "h": 0.2 }));
        assert_eq!(value["background"]["kind"], "video");
        assert_eq!(value["background"]["fit"], "cover");
        assert!(value["background"]["path"].as_str().is_some_and(|path| std::path::Path::new(path).is_absolute() && path.ends_with("loop.mp4")));
        Ok(())
    }

    #[test]
    fn the_block_round_trips_through_serde_json() -> Result<(), serde_json::Error> {
        let block = block(&pip_profile(CompositeMode::Clean));
        assert_eq!(serde_json::from_str::<CompositeBlock>(&serde_json::to_string(&block)?)?, block);
        // The spec's example reads into the types and writes back with the same fields and values.
        let spec = include_str!("../../../../docs/suite/recorder-composite-mode.md");
        let example = spec.split("```json").nth(1).and_then(|rest| rest.split("```").next()).unwrap_or_default();
        let wrapped: serde_json::Value = serde_json::from_str(&format!("{{{example}}}"))?;
        let parsed: CompositeBlock = serde_json::from_value(wrapped["composite"].clone())?;
        assert_eq!(serde_json::to_value(parsed)?, wrapped["composite"]);
        Ok(())
    }
}
