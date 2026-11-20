//! The Glass Deck components (spec: Design Language "Components"): one file
//! per widget, every widget reads only the design tokens and the runtime
//! [`Theme`](crate::Theme).
//!
//! Shared helpers: [`focus::focus_ring`] and [`capsule_radius`].

pub mod chip;
pub mod focus;
pub mod icon_button;
pub mod inspector;
pub mod list_row;
pub mod main_toolbar;
pub mod panel;
pub mod primary_button;
pub mod scroll_thumb;
pub mod search_field;
pub mod segmented;
pub mod slider;
pub mod status_bar;
pub mod switch;
pub mod text_field;

pub use chip::chip;
pub use focus::focus_ring;
pub use icon_button::icon_button;
pub use inspector::{inspector_group, inspector_header, inspector_row};
pub use list_row::list_row;
pub use main_toolbar::{MainToolbar, main_toolbar};
pub use panel::panel;
pub use primary_button::primary_button;
pub use scroll_thumb::scroll_thumb;
pub use search_field::search_field;
pub use segmented::segmented;
pub use slider::{SliderKind, slider};
pub use status_bar::status_bar;
pub use switch::switch;
pub use text_field::text_field;

use egui::CornerRadius;

/// The capsule radius rule (spec "Radii": `radius.capsule` is height / 2,
/// a rule rather than a number, so it is computed from the widget height).
pub(crate) fn capsule_radius(height: f32) -> CornerRadius {
    (height / 2.0).into()
}
