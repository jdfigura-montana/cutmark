//! What the frame looks like: the Studio "Frame" settings, as the renderer takes them.

/// The smallest padding, as a fraction of the output's shorter side.
pub const MIN_PADDING: f32 = 0.0;

/// The largest padding, as a fraction of the output's shorter side.
pub const MAX_PADDING: f32 = 0.15;

/// The largest corner radius, in pixels at 1080p.
pub const MAX_CORNER_RADIUS: f32 = 32.0;

/// The six gradient presets, preset 1 first: the sRGB colour of the top left corner, then of the
/// bottom right corner.
pub const GRADIENT_PRESETS: [[[u8; 3]; 2]; 6] = [
    [[0x1e, 0x3c, 0x72], [0x45, 0xd3, 0xc7]],
    [[0x3a, 0x1c, 0x71], [0xd7, 0x6d, 0x77]],
    [[0xf1, 0x27, 0x11], [0xf5, 0xaf, 0x19]],
    [[0x13, 0x4e, 0x5e], [0x71, 0xb2, 0x80]],
    [[0x23, 0x25, 0x26], [0x41, 0x43, 0x45]],
    [[0xd7, 0xdd, 0xe8], [0x75, 0x7f, 0x9a]],
];

/// What is behind the picture.
#[derive(Clone, Debug)]
pub enum Background {
    /// One sRGB colour.
    Solid([u8; 3]),
    /// A gradient preset, 1 to 6 (see [`GRADIENT_PRESETS`]). A number outside the range is the
    /// nearest preset.
    Gradient(u8),
    /// A picture that covers the output: scaled to fill it, with the overflow cut off evenly. Its
    /// alpha is ignored. The texture has the same needs as a source texture, which
    /// [`crate::StudioRenderer::upload_rgba`] meets.
    Image(wgpu::Texture),
}

/// How strong the shadow under the picture is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shadow {
    /// No shadow.
    Off,
    /// A light shadow.
    #[default]
    Soft,
    /// A wide, dark shadow.
    Strong,
}

/// The shadow's shape, in pixels at 1080p.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShadowShape {
    /// How far the edge fades, to each side of it.
    pub blur: f32,
    /// How far down the shadow sits.
    pub offset: f32,
    /// How dark it is in its middle, 0 to 1.
    pub opacity: f32,
}

impl Shadow {
    pub(crate) fn shape(self) -> ShadowShape {
        match self {
            Shadow::Off => ShadowShape { blur: 0.0, offset: 0.0, opacity: 0.0 },
            Shadow::Soft => ShadowShape { blur: 24.0, offset: 8.0, opacity: 0.35 },
            Shadow::Strong => ShadowShape { blur: 40.0, offset: 16.0, opacity: 0.65 },
        }
    }
}

/// The frame around the picture. The default is the spec's: off, and once on, gradient preset 1,
/// 6% padding, a corner radius of 12 and a soft shadow.
#[derive(Clone, Debug)]
pub struct FrameStyle {
    /// Off means the picture fills the output and nothing else here is used.
    pub enabled: bool,
    /// What is behind the picture.
    pub background: Background,
    /// The space kept free on every side, as a fraction of the output's shorter side,
    /// [`MIN_PADDING`] to [`MAX_PADDING`]. A value outside the range counts as the nearest end.
    pub padding: f32,
    /// The radius of the picture's corners in pixels at 1080p, 0 to [`MAX_CORNER_RADIUS`]. It
    /// scales with the output's shorter side.
    pub corner_radius: f32,
    /// The shadow under the picture.
    pub shadow: Shadow,
}

impl Default for FrameStyle {
    fn default() -> Self {
        Self { enabled: false, background: Background::Gradient(1), padding: 0.06, corner_radius: 12.0, shadow: Shadow::Soft }
    }
}
