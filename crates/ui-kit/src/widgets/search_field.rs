//! The search field (spec "Components": row "Search field").

use egui::Response;
use egui::Ui;

use crate::widgets::text_field::field;

/// The search field: `bg.group`, `radius.group`, a leading magnifier icon and
/// the placeholder in `text.tertiary`.
///
/// Usage:
///
/// ```ignore
/// let mut query = String::new();
/// ui_kit::search_field(ui, &mut query, "Search takes");
/// ```
pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    field(ui, text, hint, Some(egui_phosphor::variants::fill::MAGNIFYING_GLASS))
}
