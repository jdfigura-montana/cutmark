//! The suite's one menu item: the label on the left and the shortcut
//! right-aligned, for the popovers of [`crate::widgets::menu_bar`].

use egui::{Id, Response, Sense, StrokeKind, Ui, Widget, WidgetInfo, WidgetType, pos2, vec2};

use crate::Theme;
use crate::fonts;
use crate::icons::regular;
use crate::tokens::{space, typography};

/// Opacity of a disabled item.
const DISABLED_OPACITY: f32 = 0.4;
/// Width of the leading slot that holds the check mark.
const CHECK_SLOT: f32 = space::SCALE_16;
/// Gap between the check slot and the label.
const CHECK_GAP: f32 = space::SCALE_4;
/// Smallest gap between the label and the shortcut.
const SHORTCUT_GAP: f32 = space::SCALE_16;

/// The temp-memory key that says the menu on this ui's layer has check marks.
fn check_slot_id(ui: &Ui) -> Id {
    Id::new("ui-kit-menu-check-slot").with(ui.layer_id())
}

/// Makes every [`MenuItem`] of this menu keep the leading check slot, so the
/// labels line up. Call it before the first item of a menu that has a
/// checkable item.
pub fn menu_check_slot(ui: &Ui) {
    let id = check_slot_id(ui);
    ui.data_mut(|data| data.insert_temp(id, true));
}

/// One menu item. The label is on the left; the shortcut is right-aligned in
/// the mono caption style in `text.tertiary`, so in a justified popover all
/// shortcuts share one right edge. A disabled item draws at 40%.
///
/// Usage:
///
/// ```ignore
/// if ui.add(ui_kit::MenuItem::new("Save").shortcut("Ctrl+S")).clicked() {
///     ui.close();
/// }
/// ```
#[must_use = "add it with `ui.add`"]
pub struct MenuItem {
    /// The item's text.
    label: String,
    /// The shortcut text; empty for none.
    shortcut: String,
    /// `Some` for a checkable item: whether it is on.
    checked: Option<bool>,
}

impl MenuItem {
    /// An item with a label, no shortcut, not checkable.
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), shortcut: String::new(), checked: None }
    }

    /// Sets the shortcut text; an empty text means none.
    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = shortcut.into();
        self
    }

    /// Makes the item checkable: a check mark shows in the leading slot while
    /// `on`. The item does not change `on` itself; act on the click.
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
}

impl Widget for MenuItem {
    fn ui(self, ui: &mut Ui) -> Response {
        let Self { label, shortcut, checked } = self;
        let neutrals = Theme::get(ui.ctx()).neutrals;
        let slot_id = check_slot_id(ui);
        let has_slot = checked.is_some() || ui.data(|data| data.get_temp::<bool>(slot_id)).unwrap_or(false);
        let leading = if has_slot { CHECK_SLOT + CHECK_GAP } else { 0.0 };

        let text = ui.painter().layout_no_wrap(label.clone(), fonts::medium(typography::BODY_SIZE), neutrals.text_primary);
        let keys = (!shortcut.is_empty()).then(|| ui.painter().layout_no_wrap(shortcut, fonts::mono(typography::CAPTION_SIZE), neutrals.text_tertiary));

        let padding = ui.spacing().button_padding;
        let keys_width = keys.as_ref().map_or(0.0, |keys| SHORTCUT_GAP + keys.size().x);
        let height = (text.size().y + 2.0 * padding.y).max(ui.spacing().interact_size.y);
        let (rect, response) = ui.allocate_at_least(vec2(2.0 * padding.x + leading + text.size().x + keys_width, height), Sense::click());

        let enabled = ui.is_enabled();
        response.widget_info(|| match checked {
            Some(on) => WidgetInfo::selected(WidgetType::Checkbox, enabled, on, &label),
            None => WidgetInfo::labeled(WidgetType::Button, enabled, &label),
        });

        if ui.is_rect_visible(rect) {
            let mut painter = ui.painter().clone();
            if !enabled {
                // egui has already faded a disabled ui by its own factor.
                painter.multiply_opacity(DISABLED_OPACITY / ui.visuals().disabled_alpha().max(f32::EPSILON));
            }
            let visuals = ui.style().interact(&response);
            painter.rect(rect.expand(visuals.expansion), visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, StrokeKind::Inside);

            let left = rect.left() + padding.x;
            if checked == Some(true) {
                let check = painter.layout_no_wrap(regular::CHECK.to_owned(), fonts::icon_regular(typography::BODY_SIZE), neutrals.text_primary);
                let pos = pos2(left + (CHECK_SLOT - check.size().x) / 2.0, rect.center().y - check.size().y / 2.0);
                painter.galley(pos, check, neutrals.text_primary);
            }
            let pos = pos2(left + leading, rect.center().y - text.size().y / 2.0);
            painter.galley(pos, text, neutrals.text_primary);
            if let Some(keys) = keys {
                let pos = pos2(rect.right() - padding.x - keys.size().x, rect.center().y - keys.size().y / 2.0);
                painter.galley(pos, keys, neutrals.text_tertiary);
            }
        }
        response
    }
}
