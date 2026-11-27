//! The runtime theme: the accent and the neutrals tinted toward it, which
//! every widget reads instead of constants. It is stored in the egui context (spec: S1 Audio Editor, row
//! "Runtime theme"). Glass is always on and is not an input.

use egui::Color32;

use crate::accent;
use crate::oklch;
use crate::tokens::neutral;

/// The egui context data id under which the suite-wide theme is stored.
fn id() -> egui::Id {
    egui::Id::new("cutmark_theme")
}

/// Every neutral token (surfaces, strokes, text), tinted toward the accent's
/// hue (spec "Tinted neutrals"). The L and C values are in `design/tokens.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Neutrals {
    /// `bg.ground`.
    pub bg_ground: Color32,
    /// `bg.side`.
    pub bg_side: Color32,
    /// `bg.panel`.
    pub bg_panel: Color32,
    /// `bg.stage.inner`.
    pub bg_stage_inner: Color32,
    /// `bg.group`.
    pub bg_group: Color32,
    /// `bg.control`.
    pub bg_control: Color32,
    /// `bg.hover`.
    pub bg_hover: Color32,
    /// `bg.selected`.
    pub bg_selected: Color32,
    /// `bg.row.selected`.
    pub bg_row_selected: Color32,
    /// `bg.track.selected`.
    pub bg_track_selected: Color32,
    /// `bg.track.header`.
    pub bg_track_header: Color32,
    /// `bg.track.header.selected`.
    pub bg_track_header_selected: Color32,
    /// `bg.track.alt`.
    pub bg_track_alt: Color32,
    /// `bg.track.empty`.
    pub bg_track_empty: Color32,
    /// `bg.lane.selected`.
    pub bg_lane_selected: Color32,
    /// `bg.clip`.
    pub bg_clip: Color32,
    /// `stroke.hairline`.
    pub stroke_hairline: Color32,
    /// `stroke.separator`.
    pub stroke_separator: Color32,
    /// `stroke.separator.strong`.
    pub stroke_separator_strong: Color32,
    /// `stroke.track`.
    pub stroke_track: Color32,
    /// `stroke.track.header`.
    pub stroke_track_header: Color32,
    /// `stroke.clip`.
    pub stroke_clip: Color32,
    /// `text.primary`.
    pub text_primary: Color32,
    /// `text.secondary`.
    pub text_secondary: Color32,
    /// `text.tertiary`.
    pub text_tertiary: Color32,
    /// `text.disabled`.
    pub text_disabled: Color32,
}

impl Neutrals {
    /// The neutrals for a picked accent color. A gray pick (chroma below 0.02)
    /// gives untinted neutrals.
    pub fn derive(pick: Color32) -> Self {
        let hue = accent::neutral_hue(pick);
        let tint = |lca| {
            let [r, g, b, a] = oklch::neutral_rgba(lca, hue);
            Color32::from_rgba_premultiplied(r, g, b, a)
        };
        Self {
            bg_ground: tint(neutral::BG_GROUND),
            bg_side: tint(neutral::BG_SIDE),
            bg_panel: tint(neutral::BG_PANEL),
            bg_stage_inner: tint(neutral::BG_STAGE_INNER),
            bg_group: tint(neutral::BG_GROUP),
            bg_control: tint(neutral::BG_CONTROL),
            bg_hover: tint(neutral::BG_HOVER),
            bg_selected: tint(neutral::BG_SELECTED),
            bg_row_selected: tint(neutral::BG_ROW_SELECTED),
            bg_track_selected: tint(neutral::BG_TRACK_SELECTED),
            bg_track_header: tint(neutral::BG_TRACK_HEADER),
            bg_track_header_selected: tint(neutral::BG_TRACK_HEADER_SELECTED),
            bg_track_alt: tint(neutral::BG_TRACK_ALT),
            bg_track_empty: tint(neutral::BG_TRACK_EMPTY),
            bg_lane_selected: tint(neutral::BG_LANE_SELECTED),
            bg_clip: tint(neutral::BG_CLIP),
            stroke_hairline: tint(neutral::STROKE_HAIRLINE),
            stroke_separator: tint(neutral::STROKE_SEPARATOR),
            stroke_separator_strong: tint(neutral::STROKE_SEPARATOR_STRONG),
            stroke_track: tint(neutral::STROKE_TRACK),
            stroke_track_header: tint(neutral::STROKE_TRACK_HEADER),
            stroke_clip: tint(neutral::STROKE_CLIP),
            text_primary: tint(neutral::TEXT_PRIMARY),
            text_secondary: tint(neutral::TEXT_SECONDARY),
            text_tertiary: tint(neutral::TEXT_TERTIARY),
            text_disabled: tint(neutral::TEXT_DISABLED),
        }
    }
}

/// The runtime theme: the derived accent set and the neutrals that follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// The derived accent set: the user's pick plus the six accent tokens.
    pub accent: accent::Accent,
    /// True when the OS asks for reduced motion: ui-kit animations become instant.
    pub reduce_motion: bool,
    /// The neutral colors tinted toward the accent. [`Theme::set`] recomputes them from `accent`.
    pub neutrals: Neutrals,
}

impl Default for Theme {
    /// The default Blue preset with full motion.
    fn default() -> Self {
        Self::new(accent::default_accent(), false)
    }
}

impl Theme {
    /// A theme for `accent`, with the neutrals that follow it.
    pub fn new(accent: accent::Accent, reduce_motion: bool) -> Theme {
        Theme { accent, reduce_motion, neutrals: Neutrals::derive(accent.pick) }
    }

    /// Returns the theme stored in the egui context, or the default.
    pub fn get(ctx: &egui::Context) -> Theme {
        ctx.data(|data| data.get_temp::<Theme>(id())).unwrap_or_default()
    }

    /// Stores the theme in the egui context and requests a repaint. The
    /// neutrals are recomputed from the accent, so they never go stale.
    pub fn set(ctx: &egui::Context, theme: Theme) {
        let theme = Theme::new(theme.accent, theme.reduce_motion);
        ctx.data_mut(|data| data.insert_temp(id(), theme));
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens;

    /// A fresh context reads back the default theme.
    #[test]
    fn get_on_a_fresh_context_returns_the_default() {
        let ctx = egui::Context::default();
        assert_eq!(Theme::get(&ctx), Theme::default());
    }

    /// The Teal neutrals match the spec table's hex values within 2 per channel.
    #[test]
    fn teal_neutrals_match_the_spec_table() {
        let n = Neutrals::derive(tokens::accent::PRESET_TEAL);
        let table = [
            ("bg.ground", n.bg_ground, [0x07, 0x0A, 0x0A]),
            ("bg.side", n.bg_side, [0x10, 0x14, 0x13]),
            ("bg.panel", n.bg_panel, [0x12, 0x17, 0x16]),
            ("bg.stage.inner", n.bg_stage_inner, [0x0C, 0x10, 0x0F]),
            ("bg.group", n.bg_group, [0x19, 0x20, 0x1F]),
            ("bg.control", n.bg_control, [0x1E, 0x26, 0x25]),
            ("bg.hover", n.bg_hover, [0x26, 0x31, 0x30]),
            ("bg.selected", n.bg_selected, [0x30, 0x40, 0x3E]),
            ("text.primary", n.text_primary, [0xED, 0xF4, 0xF3]),
            ("text.secondary", n.text_secondary, [0xB7, 0xC4, 0xC2]),
            ("text.tertiary", n.text_tertiary, [0x7F, 0x8D, 0x8B]),
            ("text.disabled", n.text_disabled, [0x4E, 0x5B, 0x59]),
        ];
        for (token, got, want) in table {
            let diff = [got.r().abs_diff(want[0]), got.g().abs_diff(want[1]), got.b().abs_diff(want[2])].into_iter().max().unwrap_or(0);
            assert!(diff <= 2, "{token}: got #{:02X}{:02X}{:02X}, want {want:02X?}", got.r(), got.g(), got.b());
        }
    }

    /// Readable text keeps 4.5:1 on `bg.panel` and `bg.group` for every preset
    /// and for a gray pick.
    #[test]
    fn readable_text_meets_the_contrast_rule_for_presets_and_gray() {
        let mut picks = accent::presets().to_vec();
        picks.push(("gray", Color32::from_rgb(0x80, 0x80, 0x80)));
        for (name, pick) in picks {
            let n = Neutrals::derive(pick);
            for (text_name, text) in [("text.secondary", n.text_secondary), ("text.tertiary", n.text_tertiary)] {
                for (bg_name, bg) in [("bg.panel", n.bg_panel), ("bg.group", n.bg_group)] {
                    let ratio = accent::contrast(text, bg);
                    assert!(ratio >= 4.5, "{name}: {text_name} on {bg_name} is {ratio:.2}:1");
                }
            }
        }
    }

    /// A gray pick gives untinted neutrals.
    #[test]
    fn a_gray_pick_gives_untinted_neutrals() {
        let n = Neutrals::derive(Color32::from_rgb(0x80, 0x80, 0x80));
        for color in [n.bg_panel, n.bg_selected, n.text_primary, n.text_tertiary, n.stroke_hairline] {
            assert!(color.r().abs_diff(color.g()) <= 1 && color.g().abs_diff(color.b()) <= 1, "{color:?} is tinted");
        }
    }

    /// The generated Blue constants (the fallbacks) equal the default theme's neutrals.
    #[test]
    fn the_token_constants_are_the_blue_neutrals() {
        let n = Theme::default().neutrals;
        assert_eq!(n.bg_panel, tokens::color::BG_PANEL);
        assert_eq!(n.bg_track_alt, tokens::color::BG_TRACK_ALT);
        assert_eq!(n.stroke_hairline, tokens::color::STROKE_HAIRLINE);
        assert_eq!(n.text_tertiary, tokens::color::TEXT_TERTIARY);
    }

    /// `set` recomputes neutrals that no longer match the accent.
    #[test]
    fn set_recomputes_the_neutrals_from_the_accent() {
        let ctx = egui::Context::default();
        let teal = accent::derive(tokens::accent::PRESET_TEAL);
        Theme::set(&ctx, Theme { accent: teal, ..Theme::default() });
        assert_eq!(Theme::get(&ctx).neutrals, Neutrals::derive(teal.pick));
    }

    /// A set theme reads back unchanged.
    #[test]
    fn set_then_get_round_trips() {
        let ctx = egui::Context::default();
        let theme = Theme::new(accent::derive(accent::presets()[1].1), false);
        Theme::set(&ctx, theme);
        assert_eq!(Theme::get(&ctx), theme);
    }
}
