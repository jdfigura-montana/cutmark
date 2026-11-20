//! The Glass Deck components (spec: Design Language "Components"): one file
//! per widget, every widget reads only the design tokens and the runtime
//! [`Theme`](crate::Theme).
//!
//! Shared helpers: [`focus::focus_ring`] and [`capsule_radius`].

pub mod chip;
pub mod focus;
pub mod icon_button;
pub mod panel;
pub mod primary_button;
pub mod segmented;
pub mod switch;

pub use chip::chip;
pub use focus::focus_ring;
pub use icon_button::icon_button;
pub use panel::panel;
pub use primary_button::primary_button;
pub use segmented::segmented;
pub use switch::switch;

use egui::CornerRadius;

/// The capsule radius rule (spec "Radii": `radius.capsule` is height / 2,
/// a rule rather than a number, so it is computed from the widget height).
pub(crate) fn capsule_radius(height: f32) -> CornerRadius {
    (height / 2.0).into()
}
