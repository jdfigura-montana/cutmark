//! The runtime theme: the two inputs (`accent`, `glass`) every widget reads
//! instead of constants, stored in the egui context (spec: S1 Audio Editor,
//! row "Runtime theme").

use crate::accent;

/// The egui context data id under which the suite-wide theme is stored.
fn id() -> egui::Id {
    egui::Id::new("cutmark_theme")
}

/// The runtime theme: the derived accent set and the glass switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// The derived accent set: the user's pick plus the six accent tokens.
    pub accent: accent::Accent,
    /// True for painted glass, false for the solid `bg.group` fallback.
    pub glass: bool,
    /// True when the OS asks for reduced motion: ui-kit animations become instant.
    pub reduce_motion: bool,
}

impl Default for Theme {
    /// The default Blue preset with glass on and full motion.
    fn default() -> Self {
        Self { accent: accent::default_accent(), glass: true, reduce_motion: false }
    }
}

impl Theme {
    /// Returns the theme stored in the egui context, or the default.
    pub fn get(ctx: &egui::Context) -> Theme {
        ctx.data(|data| data.get_temp::<Theme>(id())).unwrap_or_default()
    }

    /// Stores the theme in the egui context and requests a repaint.
    pub fn set(ctx: &egui::Context, theme: Theme) {
        ctx.data_mut(|data| data.insert_temp(id(), theme));
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh context reads back the default theme.
    #[test]
    fn get_on_a_fresh_context_returns_the_default() {
        let ctx = egui::Context::default();
        assert_eq!(Theme::get(&ctx), Theme::default());
        assert!(Theme::get(&ctx).glass);
    }

    /// A set theme reads back unchanged.
    #[test]
    fn set_then_get_round_trips() {
        let ctx = egui::Context::default();
        let theme = Theme { accent: accent::derive(accent::presets()[1].1), glass: false, reduce_motion: false };
        Theme::set(&ctx, theme);
        assert_eq!(Theme::get(&ctx), theme);
    }
}
