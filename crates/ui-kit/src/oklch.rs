//! The OKLab and OKLCH color math, on plain sRGB bytes. It is shared by the
//! runtime ([`crate::accent`], [`crate::theme`]) and by `build.rs`, which
//! includes this file by path, so the generated Blue token constants and the
//! runtime neutrals cannot drift apart. No egui types and no color crate here.
//!
//! Written by hand following Björn Ottosson's OKLab: sRGB to linear sRGB,
//! linear sRGB to OKLab, OKLab to OKLCH. Colors outside the sRGB gamut are
//! brought back by reducing chroma with hue and lightness fixed.

/// Slack around the sRGB gamut bounds, so float noise at the boundary does not trigger gamut mapping.
const GAMUT_EPS: f32 = 1e-4;

/// A color in OKLab: lightness and the two opponent axes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Oklab {
    /// Lightness, 0 for black to 1 for white.
    pub(crate) l: f32,
    /// Green-red opponent axis.
    pub(crate) a: f32,
    /// Blue-yellow opponent axis.
    pub(crate) b: f32,
}

/// A color in OKLCH: lightness, chroma and hue in degrees (0 to 360).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Oklch {
    /// Lightness, 0 for black to 1 for white.
    pub(crate) l: f32,
    /// Chroma: 0 for gray.
    pub(crate) c: f32,
    /// Hue in degrees, 0 to 360.
    pub(crate) h: f32,
}

/// Converts one sRGB u8 channel to linear light (0 to 1).
pub(crate) fn channel_to_linear(channel: u8) -> f32 {
    let c = f32::from(channel) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Converts linear light (0 to 1) back to one sRGB u8 channel, rounded.
fn linear_to_channel(linear: f32) -> u8 {
    let c = if linear <= 0.0031308 { linear * 12.92 } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 };
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Converts linear RGB to OKLab (Björn Ottosson's reference transform).
pub(crate) fn linear_to_oklab(rgb: [f32; 3]) -> Oklab {
    let [r, g, b] = rgb;
    let l = (0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_993 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    Oklab {
        l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    }
}

/// Converts OKLab to linear RGB (Björn Ottosson's reference transform).
fn oklab_to_linear(color: Oklab) -> [f32; 3] {
    let l_ = color.l + 0.396_337_78 * color.a + 0.215_803_76 * color.b;
    let m_ = color.l - 0.105_561_35 * color.a - 0.063_854_17 * color.b;
    let s_ = color.l - 0.089_484_18 * color.a - 1.291_485_5 * color.b;
    let (l, m, s) = (l_.powi(3), m_.powi(3), s_.powi(3));
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_93 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

/// Converts OKLab to OKLCH.
fn oklab_to_oklch(color: Oklab) -> Oklch {
    let c = (color.a * color.a + color.b * color.b).sqrt();
    let h = color.b.atan2(color.a).to_degrees().rem_euclid(360.0);
    Oklch { l: color.l, c, h }
}

/// Converts OKLCH to OKLab.
fn oklch_to_oklab(color: Oklch) -> Oklab {
    let radians = color.h.to_radians();
    Oklab { l: color.l, a: color.c * radians.cos(), b: color.c * radians.sin() }
}

/// Converts sRGB bytes to OKLCH.
pub(crate) fn rgb_to_oklch(rgb: [u8; 3]) -> Oklch {
    oklab_to_oklch(linear_to_oklab(rgb.map(channel_to_linear)))
}

/// Converts OKLCH to sRGB bytes, reducing chroma by bisection (hue and
/// lightness fixed) when the color falls outside the sRGB gamut.
pub(crate) fn oklch_to_rgb(color: Oklch) -> [u8; 3] {
    let fits = |chroma: f32| {
        let rgb = oklab_to_linear(oklch_to_oklab(Oklch { c: chroma, ..color }));
        rgb.iter().all(|&value| (-GAMUT_EPS..=1.0 + GAMUT_EPS).contains(&value))
    };
    let chroma = if fits(color.c) {
        color.c
    } else {
        // Bisect between 0 (always in gamut) and the requested chroma.
        let (mut lo, mut hi) = (0.0_f32, color.c);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;
            if fits(mid) { lo = mid } else { hi = mid }
        }
        lo
    };
    oklab_to_linear(oklch_to_oklab(Oklch { c: chroma, ..color })).map(linear_to_channel)
}

/// One neutral token as premultiplied sRGBA bytes: `(l, c, alpha)` from
/// `design/tokens.json` plus the accent hue. `None` (a gray accent pick) gives
/// the untinted neutral.
pub(crate) fn neutral_rgba((l, c, alpha): (f32, f32, f32), hue: Option<f32>) -> [u8; 4] {
    let rgb = oklch_to_rgb(match hue {
        Some(h) => Oklch { l, c, h },
        None => Oklch { l, c: 0.0, h: 0.0 },
    });
    let scaled = |value: f32| (value * alpha).round().clamp(0.0, 255.0) as u8;
    [scaled(f32::from(rgb[0])), scaled(f32::from(rgb[1])), scaled(f32::from(rgb[2])), scaled(255.0)]
}
