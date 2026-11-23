//! Accent derivation: turns any user-picked color into the six accent tokens
//! (`accent`, `hover`, `pressed`, `on`, `tint`, `content`) in OKLCH, with the
//! readability clamp from the Design Language spec ("Color: accent").
//!
//! The color math is written by hand following Björn Ottosson's OKLab: sRGB
//! to linear sRGB, linear sRGB to OKLab, OKLab to OKLCH. Colors outside the
//! sRGB gamut are brought back by reducing chroma with hue and lightness
//! fixed. No color crate.

use egui::Color32;

use crate::tokens;

/// Clamp lower bound for the accent lightness.
const L_MIN: f32 = 0.68;
/// Clamp upper bound for the accent lightness.
const L_MAX: f32 = 0.86;
/// Maximum accent chroma, so neon picks do not vibrate against the chrome.
const C_MAX: f32 = 0.19;
/// Picks with a chroma below this are gray: the chroma is kept, the hue is ignored.
const GRAY_MAX_C: f32 = 0.02;
/// Hover lightness lift.
const HOVER_LIFT: f32 = 0.06;
/// Pressed lightness drop.
const PRESSED_DROP: f32 = 0.07;
/// Fixed lightness of the tint token (selected clip body).
const TINT_L: f32 = 0.27;
/// Tint chroma scale relative to the accent chroma.
const TINT_C_SCALE: f32 = 0.25;
/// Fixed lightness of the content token (waveform inside a selected clip).
const CONTENT_L: f32 = 0.88;
/// Content chroma scale relative to the accent chroma.
const CONTENT_C_SCALE: f32 = 0.35;
/// Hue distance in degrees at or below which a pick counts as near the record color.
const NEAR_RECORD_DEG: f32 = 20.0;
/// Slack around the sRGB gamut bounds, so float noise at the boundary does not trigger gamut mapping.
const GAMUT_EPS: f32 = 1e-4;

/// A derived accent set: the user's pick, the six accent tokens and the two picker notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accent {
    /// The color the user picked, unchanged.
    pub pick: Color32,
    /// `accent`: the pick after the readability clamp.
    pub accent: Color32,
    /// `accent.hover`: accent lightness plus 0.06.
    pub hover: Color32,
    /// `accent.pressed`: accent lightness minus 0.07.
    pub pressed: Color32,
    /// `accent.on`: fixed near-black text and icon color on the accent.
    pub on: Color32,
    /// `accent.tint`: lightness 0.27, chroma times 0.25 (selected clip body).
    pub tint: Color32,
    /// `accent.content`: lightness 0.88, chroma times 0.35 (waveform in a selected clip).
    pub content: Color32,
    /// True when the clamp changed the pick by more than 1 on any channel: the picker shows "Adjusted for readability".
    pub adjusted: bool,
    /// True for chromatic picks whose hue is within 20 degrees of the record color: allowed, but the picker shows a note.
    pub near_record: bool,
}

/// A color in OKLab: lightness and the two opponent axes.
#[derive(Debug, Clone, Copy)]
struct Oklab {
    /// Lightness, 0 for black to 1 for white.
    l: f32,
    /// Green-red opponent axis.
    a: f32,
    /// Blue-yellow opponent axis.
    b: f32,
}

/// A color in OKLCH: lightness, chroma and hue in degrees (0 to 360).
#[derive(Debug, Clone, Copy)]
struct Oklch {
    /// Lightness, 0 for black to 1 for white.
    l: f32,
    /// Chroma: 0 for gray.
    c: f32,
    /// Hue in degrees, 0 to 360.
    h: f32,
}

/// Converts one sRGB u8 channel to linear light (0 to 1).
fn channel_to_linear(channel: u8) -> f32 {
    let c = f32::from(channel) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Converts linear light (0 to 1) back to one sRGB u8 channel, rounded.
fn linear_to_channel(linear: f32) -> u8 {
    let c = if linear <= 0.0031308 { linear * 12.92 } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 };
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Converts an sRGB color to linear RGB.
fn srgb_to_linear(color: Color32) -> [f32; 3] {
    [channel_to_linear(color.r()), channel_to_linear(color.g()), channel_to_linear(color.b())]
}

/// Converts linear RGB to OKLab (Björn Ottosson's reference transform).
fn linear_to_oklab(rgb: [f32; 3]) -> Oklab {
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

/// Converts an sRGB color to OKLCH.
fn srgb_to_oklch(color: Color32) -> Oklch {
    oklab_to_oklch(linear_to_oklab(srgb_to_linear(color)))
}

/// Converts OKLCH to an sRGB `Color32`, reducing chroma by bisection (hue and
/// lightness fixed) when the color falls outside the sRGB gamut.
fn oklch_to_color32(color: Oklch) -> Color32 {
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
    let rgb = oklab_to_linear(oklch_to_oklab(Oklch { c: chroma, ..color }));
    Color32::from_rgb(linear_to_channel(rgb[0]), linear_to_channel(rgb[1]), linear_to_channel(rgb[2]))
}

/// Derives the six accent tokens from a user-picked color, applying the
/// readability clamp first: lightness clamped to 0.68 to 0.86, chroma capped
/// at 0.19, gray picks (chroma below 0.02) kept gray with their hue ignored.
pub fn derive(pick: Color32) -> Accent {
    let picked = srgb_to_oklch(pick);
    let gray = picked.c < GRAY_MAX_C;
    let l = picked.l.clamp(L_MIN, L_MAX);
    let (c, h) = if gray { (picked.c, 0.0) } else { (picked.c.min(C_MAX), picked.h) };
    let clamped = Oklch { l, c, h };

    let accent = oklch_to_color32(clamped);
    let hover = oklch_to_color32(Oklch { l: l + HOVER_LIFT, ..clamped });
    let pressed = oklch_to_color32(Oklch { l: l - PRESSED_DROP, ..clamped });
    let tint = oklch_to_color32(Oklch { l: TINT_L, c: c * TINT_C_SCALE, h });
    let content = oklch_to_color32(Oklch { l: CONTENT_L, c: c * CONTENT_C_SCALE, h });
    let adjusted = differs_by_more_than(pick, accent, 1);
    let near_record = !gray && hue_distance(h, srgb_to_oklch(tokens::color::SEM_RECORD).h) <= NEAR_RECORD_DEG;

    Accent { pick, accent, hover, pressed, on: tokens::accent::ON, tint, content, adjusted, near_record }
}

/// The four picker presets, straight from the token values: Blue (default), Violet, Teal and Orange.
pub fn presets() -> [(&'static str, Color32); 4] {
    [
        ("Blue", tokens::accent::PRESET_BLUE),
        ("Violet", tokens::accent::PRESET_VIOLET),
        ("Teal", tokens::accent::PRESET_TEAL),
        ("Orange", tokens::accent::PRESET_ORANGE),
    ]
}

/// Derives the accent set for the default Blue preset.
pub fn default_accent() -> Accent {
    derive(tokens::accent::PRESET_BLUE)
}

/// Converts an sRGB color to OKLCH as `[l, c, h]`: lightness 0 to 1, chroma,
/// hue in degrees (0 to 360). Used by the accent picker's wheel and slider.
pub fn to_oklch(pick: Color32) -> [f32; 3] {
    let color = srgb_to_oklch(pick);
    [color.l, color.c, color.h]
}

/// The sRGB color for OKLCH `(l, c, h)`: lightness 0 to 1, chroma, hue in
/// degrees. Colors outside the sRGB gamut come back with chroma reduced and
/// hue and lightness fixed (the wheel paints out-of-gamut areas this way).
pub fn from_oklch(l: f32, c: f32, h: f32) -> Color32 {
    oklch_to_color32(Oklch { l, c, h })
}

/// Returns the WCAG 2.x contrast ratio between two sRGB colors (1.0 to 21.0).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let luminance = |color: Color32| 0.212_6 * channel_to_linear(color.r()) + 0.715_2 * channel_to_linear(color.g()) + 0.072_2 * channel_to_linear(color.b());
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// True when any RGB channel of the two colors differs by more than `tolerance`.
fn differs_by_more_than(a: Color32, b: Color32, tolerance: u8) -> bool {
    [a.r().abs_diff(b.r()), a.g().abs_diff(b.g()), a.b().abs_diff(b.b())].into_iter().any(|diff| diff > tolerance)
}

/// Smallest distance in degrees between two hues, wrapping at 360 (0 to 180).
fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 { 360.0 - d } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values: OKLab of white is (1, 0, 0).
    #[test]
    fn oklab_of_white_is_the_reference_point() {
        let white = linear_to_oklab([1.0, 1.0, 1.0]);
        assert!((white.l - 1.0).abs() < 1e-3, "L = {}", white.l);
        assert!(white.a.abs() < 1e-3, "a = {}", white.a);
        assert!(white.b.abs() < 1e-3, "b = {}", white.b);
    }

    /// Reference values: OKLab of #FF0000 is about (0.62796, 0.22486, 0.12585).
    #[test]
    fn oklab_of_pure_red_matches_the_reference() {
        let red = linear_to_oklab([1.0, 0.0, 0.0]);
        assert!((red.l - 0.627_96).abs() < 1e-3, "L = {}", red.l);
        assert!((red.a - 0.224_86).abs() < 1e-3, "a = {}", red.a);
        assert!((red.b - 0.125_85).abs() < 1e-3, "b = {}", red.b);
    }

    /// 200 fixed sample colors survive the sRGB -> OKLCH -> sRGB round trip within 1 per channel.
    #[test]
    fn round_trip_stays_within_one_channel() {
        for i in 0..200u32 {
            let color = Color32::from_rgb((i * 97 % 256) as u8, (i * 173 % 256) as u8, (i * 61 % 256) as u8);
            let back = oklch_to_color32(srgb_to_oklch(color));
            assert!(
                !differs_by_more_than(color, back, 1),
                "#{:02X}{:02X}{:02X} became #{:02X}{:02X}{:02X}",
                color.r(),
                color.g(),
                color.b(),
                back.r(),
                back.g(),
                back.b()
            );
        }
    }

    /// The presets and the edge picks all meet the readability guarantees:
    /// `accent.on` at least 4.5:1 on the accent, the accent at least 3:1 on `bg.panel`.
    #[test]
    fn clamp_meets_the_contrast_guarantees_for_presets_and_edge_picks() {
        let [blue, violet, teal, orange] = presets().map(|(_, color)| color);
        let picks = [
            blue,
            violet,
            teal,
            orange,
            Color32::BLACK,
            Color32::WHITE,
            Color32::from_rgb(0x80, 0x80, 0x80),
            Color32::from_rgb(0xFF, 0x00, 0x00),
            Color32::from_rgb(0x39, 0xFF, 0x14),
        ];
        for pick in picks {
            let accent = derive(pick);
            let on = contrast(accent.on, accent.accent);
            let panel = contrast(accent.accent, tokens::color::BG_PANEL);
            assert!(on >= 4.5, "#{:02X}{:02X}{:02X}: accent.on contrast {on}", pick.r(), pick.g(), pick.b());
            assert!(panel >= 3.0, "#{:02X}{:02X}{:02X}: bg.panel contrast {panel}", pick.r(), pick.g(), pick.b());
        }
    }

    /// The extreme picks are reported as adjusted; the Blue preset is not.
    #[test]
    fn adjusted_flag_marks_the_clamped_picks() {
        for pick in [Color32::BLACK, Color32::WHITE, Color32::from_rgb(0xFF, 0x00, 0x00), Color32::from_rgb(0x39, 0xFF, 0x14)] {
            assert!(derive(pick).adjusted, "#{:02X}{:02X}{:02X} must report adjusted", pick.r(), pick.g(), pick.b());
        }
        assert!(!derive(tokens::accent::PRESET_BLUE).adjusted, "Blue must not report adjusted");
    }

    /// The near-record note fires for red picks, not for Blue and not for gray.
    #[test]
    fn near_record_flag_tracks_the_hue_distance() {
        assert!(derive(Color32::from_rgb(0xFF, 0x00, 0x00)).near_record, "pure red is near the record hue");
        assert!(!derive(tokens::accent::PRESET_BLUE).near_record, "Blue is not near the record hue");
        assert!(!derive(Color32::from_rgb(0x80, 0x80, 0x80)).near_record, "gray is never near the record hue");
    }

    /// A gray pick stays gray: the derived accent has equal channels within 1.
    #[test]
    fn a_gray_pick_stays_gray() {
        let accent = derive(Color32::from_rgb(0x80, 0x80, 0x80)).accent;
        let (r, g, b) = (accent.r(), accent.g(), accent.b());
        assert!(r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1, "gray became RGB ({r}, {g}, {b})");
    }

    /// The OKLCH helpers round-trip a preset color within 1 per channel.
    #[test]
    fn oklch_helpers_round_trip_a_preset() {
        for (_, pick) in presets() {
            let [l, c, h] = to_oklch(pick);
            let back = from_oklch(l, c, h);
            let within = back.r().abs_diff(pick.r()) <= 1 && back.g().abs_diff(pick.g()) <= 1 && back.b().abs_diff(pick.b()) <= 1;
            assert!(within, "#{:02X}{:02X}{:02X} round-tripped to #{:02X}{:02X}{:02X}", pick.r(), pick.g(), pick.b(), back.r(), back.g(), back.b());
        }
    }

    /// `from_oklch` maps huge out-of-gamut chroma back into sRGB: the result
    /// reads back with a smaller chroma, hue and lightness preserved.
    #[test]
    fn from_oklch_maps_out_of_gamut_into_the_srgb_gamut() {
        let mapped = from_oklch(0.5, 0.4, 30.0);
        let [l, c, h] = to_oklch(mapped);
        assert!(c < 0.4, "chroma {c} must be reduced below the requested 0.4");
        assert!((l - 0.5).abs() < 0.02, "lightness {l} must stay near 0.5");
        assert!((h - 30.0).abs() < 2.0, "hue {h} must stay near 30 degrees");
    }

    /// Blue's derived values match the defaults in design/tokens.json within 1
    /// on every channel. Derivation is the source of truth: when this fails,
    /// write the derived values into design/tokens.json and regenerate
    /// design/tokens.css with UPDATE_TOKENS_CSS=1.
    #[test]
    fn blue_derivation_matches_the_token_defaults() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/tokens.json");
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        assert!(json.is_object(), "cannot read or parse {}", path.display());

        let derived = default_accent();
        let pairs = [
            ("accent", derived.accent),
            ("accent.hover", derived.hover),
            ("accent.pressed", derived.pressed),
            ("accent.on", derived.on),
            ("accent.tint", derived.tint),
            ("accent.content", derived.content),
        ];
        let mut worst = 0u8;
        let mut report = String::new();
        for (name, got) in pairs {
            let text = json["accent"][name]["value"].as_str().unwrap_or("");
            let want = parse_hex(text);
            let diff = [want.r().abs_diff(got.r()), want.g().abs_diff(got.g()), want.b().abs_diff(got.b())].into_iter().max().unwrap_or(0);
            worst = worst.max(diff);
            report.push_str(&format!("\n  {name}: tokens.json {text} vs derived #{:02X}{:02X}{:02X} (max diff {diff})", got.r(), got.g(), got.b()));
        }
        assert!(worst <= 1, "Blue derivation drifted from design/tokens.json:{report}");
    }

    /// Parses a `#RRGGBB` token value into a `Color32` (black on bad input).
    fn parse_hex(text: &str) -> Color32 {
        let hex = text.strip_prefix('#').unwrap_or(text);
        let byte = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range).unwrap_or(""), 16).unwrap_or(0);
        Color32::from_rgb(byte(0..2), byte(2..4), byte(4..6))
    }
}
