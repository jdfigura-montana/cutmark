//! The transport deck (spec "Components": row "Transport deck"): a glass
//! capsule at the bottom center of the content area with the timecode and the
//! transport buttons, draggable sideways, fading while scrubbing near it.

use egui::{Align2, Color32, Context, Id, InnerResponse, Margin, Order, Pos2, Rect, Response, Sense, Ui, UiBuilder, WidgetInfo, WidgetType, pos2, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{color, motion, size, space, typography};
use crate::widgets::{capsule_radius, focus, glass};

/// Sideways reach of the scrub fade (spec "Transport deck": fades while the
/// pointer is near it during scrubbing; 80 px).
const NEAR_RADIUS: f32 = 80.0;
/// The deck's faded opacity while scrubbing near the pointer (spec
/// "Transport deck": fades to 40%).
const FADED_OPACITY: f32 = 0.4;
/// Width of the play capsule (spec "Transport deck": a 56 x 44 capsule).
const PLAY_WIDTH: f32 = 56.0;
/// Glyph size inside the deck buttons: between the headline and title steps,
/// since no icon-size token exists (spec "Icons").
const DECK_ICON_SIZE: f32 = 20.0;

/// The transport deck's state, stored in the egui context data by the deck id.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeckState {
    /// The deck's sideways offset in logical px from the content bottom center.
    pub offset: f32,
    /// The deck rect as drawn on the last frame, for the fade proximity check.
    last_rect: Option<Rect>,
}

impl DeckState {
    /// Reads the deck state from the context data, or the default.
    fn load(ctx: &Context, id: Id) -> Self {
        ctx.data(|data| data.get_temp::<DeckState>(id)).unwrap_or_default()
    }

    /// Writes the deck state back to the context data.
    fn store(self, ctx: &Context, id: Id) {
        ctx.data_mut(|data| data.insert_temp(id, self));
    }
}

/// Clamps a sideways offset so the deck of `deck_width` stays fully inside
/// `content_rect` (0 when the deck is wider than the area).
fn clamp_offset(offset: f32, deck_width: f32, content_rect: Rect) -> f32 {
    let half = deck_width / 2.0;
    let min = content_rect.left() + half - content_rect.center().x;
    let max = content_rect.right() - half - content_rect.center().x;
    if min > max {
        return 0.0;
    }
    offset.clamp(min, max)
}

/// The deck's opacity for this frame: it fades to [`FADED_OPACITY`] over the
/// transport-deck fade motion token (instant when the theme asks for reduced
/// motion) while scrubbing and the pointer is within [`NEAR_RADIUS`] of the
/// deck rect, and stays at full opacity otherwise.
fn deck_opacity(ctx: &Context, id: Id, deck: Option<Rect>, pointer: Option<Pos2>, scrubbing: bool, reduce_motion: bool) -> f32 {
    let near = deck.is_some_and(|deck| pointer.is_some_and(|pointer| deck.distance_to_pos(pointer) <= NEAR_RADIUS));
    let target = if scrubbing && near { FADED_OPACITY } else { 1.0 };
    let duration = if reduce_motion { 0.0 } else { motion::TRANSPORT_DECK_FADE_MS as f32 / 1000.0 };
    ctx.animate_value_with_time(id, target, duration)
}

/// The transport deck: a glass capsule 56 tall, anchored to the bottom center
/// of `content_rect`, 16 px from its bottom, drawn in a foreground
/// [`egui::Area`]. `add_contents` lays the timecode and the transport buttons
/// ([`deck_button`], [`play_button`]) out left to right.
///
/// Dragging anywhere on the deck that is not a button moves it sideways,
/// clamped inside `content_rect`. While `scrubbing` is true and the pointer
/// is within 80 px of the deck, the deck fades to 40% (200 ms, instant when
/// [`Theme::reduce_motion`] is set). The sideways offset persists in the
/// context data under `id`.
///
/// Usage:
///
/// ```ignore
/// ui_kit::transport_deck(ui, ui.max_rect(), egui::Id::new("transport"), scrubbing, |ui| {
///     ui_kit::timecode(ui, "00:01:23", ".45");
///     ui_kit::deck_button(ui, egui_phosphor::variants::fill::SKIP_BACK, "Go to start");
///     if ui_kit::play_button(ui, playing, ui_kit::PlayKind::Play).clicked() {
///         playing = !playing;
///     }
/// });
/// ```
pub fn transport_deck<R>(ui: &Ui, content_rect: Rect, id: Id, scrubbing: bool, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let ctx = ui.ctx();
    let theme = Theme::get(ctx);
    let mut state = DeckState::load(ctx, id);
    let opacity = deck_opacity(ctx, id.with("fade"), state.last_rect, ctx.pointer_latest_pos(), scrubbing, theme.reduce_motion);

    let anchor = pos2(content_rect.center().x + state.offset, content_rect.bottom() - space::SCALE_16);
    let area = egui::Area::new(id).order(Order::Foreground).pivot(Align2::CENTER_BOTTOM).fixed_pos(anchor).show(ctx, |ui| {
        ui.set_opacity(opacity);
        glass::glass_frame(ctx, capsule_radius(size::TRANSPORT_DECK_HEIGHT))
            .inner_margin(Margin::same(space::SCALE_6 as i8))
            .show(ui, |ui| {
                // The drag surface sits behind the controls: it is
                // registered first, so the buttons on top keep their
                // clicks while any press-drag on the empty deck moves it.
                let inner_rect = ui.max_rect();
                let drag = {
                    let mut child = ui.new_child(UiBuilder::new().id_salt(id.with("drag")).max_rect(inner_rect).sense(Sense::click_and_drag()));
                    child.allocate_exact_size(inner_rect.size(), Sense::click_and_drag()).1
                };
                if drag.dragged() {
                    state.offset += drag.drag_delta().x;
                }
                ui.horizontal(add_contents).inner
            })
            .inner
    });

    let deck_rect = area.response.rect;
    state.offset = clamp_offset(state.offset, deck_rect.width(), content_rect);
    state.last_rect = Some(deck_rect);
    state.store(ctx, id);
    area
}

/// A transport deck button: a 44 px transparent circle, `bg.hover` on hover,
/// the phosphor glyph in `text.secondary` (`text.primary` hovered). The
/// tooltip doubles as the accessible label.
pub fn deck_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    let side = size::TRANSPORT_BUTTON;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let response = response.on_hover_text(tooltip);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, tooltip));
    let hovered = response.hovered();
    if ui.is_rect_visible(rect) {
        if hovered {
            ui.painter().circle_filled(rect.center(), side / 2.0, color::BG_HOVER);
        }
        let galley =
            ui.painter().layout_no_wrap(icon.to_owned(), fonts::icon_fill(DECK_ICON_SIZE), if hovered { color::TEXT_PRIMARY } else { color::TEXT_SECONDARY });
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::PLACEHOLDER);
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, capsule_radius(side));
    }
    response
}

/// Which transport action the play capsule performs (spec "Transport deck":
/// Play in the middle; app-specific buttons around it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayKind {
    /// Play and pause, on the theme accent.
    Play,
    /// Record and stop, on `sem.record`.
    Record,
}

/// The play capsule: 56 x 44, [`PlayKind::Play`] on the accent with a play or
/// pause glyph in `accent.on`, [`PlayKind::Record`] on `sem.record` with a
/// record or stop glyph in white. The accessible name follows the state
/// (Play/Pause, Record/Stop).
pub fn play_button(ui: &mut Ui, playing: bool, kind: PlayKind) -> Response {
    let width = PLAY_WIDTH;
    let height = size::TRANSPORT_BUTTON;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let theme = Theme::get(ui.ctx());
    let (icon, label, fill, glyph) = match kind {
        PlayKind::Play => (
            if playing { crate::icons::fill::PAUSE } else { crate::icons::fill::PLAY },
            if playing { "Pause" } else { "Play" },
            if response.is_pointer_button_down_on() {
                theme.accent.pressed
            } else if response.hovered() {
                theme.accent.hover
            } else {
                theme.accent.accent
            },
            theme.accent.on,
        ),
        PlayKind::Record => (
            if playing { crate::icons::fill::STOP } else { crate::icons::fill::RECORD },
            if playing { "Stop" } else { "Record" },
            color::SEM_RECORD,
            Color32::WHITE,
        ),
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, capsule_radius(height), fill);
        let galley = ui.painter().layout_no_wrap(icon.to_owned(), fonts::icon_fill(DECK_ICON_SIZE), glyph);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::PLACEHOLDER);
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, capsule_radius(height));
    }
    response
}

/// The large transport timecode (spec "Typography": `type.time.large`, the
/// JetBrains Mono step): the main readout in `text.primary`, the sub-second
/// fraction in `text.tertiary`.
pub fn timecode(ui: &mut Ui, main: &str, fraction: &str) -> InnerResponse<()> {
    let font = fonts::mono_semibold(typography::TIME_LARGE_SIZE);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SCALE_2;
        ui.add(egui::Label::new(egui::RichText::new(main).font(font.clone()).color(color::TEXT_PRIMARY)));
        if !fraction.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(fraction).font(font).color(color::TEXT_TERTIARY)));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offset clamps so the deck stays inside the content rect.
    #[test]
    fn clamp_offset_keeps_the_deck_inside_the_content_rect() {
        let content = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        // Centered: offset 0.
        assert_eq!(clamp_offset(0.0, 172.0, content), 0.0);
        // 800 - 172 = 628 free px, so 314 either way.
        assert_eq!(clamp_offset(9_999.0, 172.0, content), 314.0);
        assert_eq!(clamp_offset(-9_999.0, 172.0, content), -314.0);
        // A deck wider than the content area stays centered.
        assert_eq!(clamp_offset(50.0, 1_000.0, content), 0.0);
    }

    /// The fade only targets 40% while scrubbing near the deck, and reaches
    /// its target in one frame with reduced motion.
    #[test]
    fn deck_opacity_fades_only_while_scrubbing_near_the_deck() {
        let ctx = Context::default();
        let id = Id::new("fade-test");
        let deck = Rect::from_min_size(Pos2::ZERO, vec2(172.0, 56.0));
        let mut time = 0.0f64;
        let mut sample = |scrubbing: bool, pointer: Option<Pos2>, reduce_motion: bool| {
            time += 0.05;
            let mut value = 1.0;
            let raw = egui::RawInput { time: Some(time), ..Default::default() };
            let mut output = ctx.run_ui(raw, |ui| {
                value = deck_opacity(ui.ctx(), id, Some(deck), pointer, scrubbing, reduce_motion);
            });
            output.textures_delta.clear();
            value
        };
        // The first frame registers the value at full opacity.
        sample(false, None, false);
        // Scrubbing far away keeps full opacity.
        assert_eq!(sample(true, Some(Pos2::new(1_000.0, 0.0)), false), 1.0);
        // Scrubbing near fades over the motion token: the change frame holds
        // the old opacity, the next frame is mid-travel.
        assert_eq!(sample(true, Some(Pos2::new(80.0, 20.0)), false), 1.0);
        assert!(sample(true, Some(Pos2::new(80.0, 20.0)), false) < 1.0, "the fade must animate with full motion");
        // With reduced motion the fade is instant.
        assert_eq!(sample(true, Some(Pos2::new(80.0, 20.0)), true), FADED_OPACITY);
        // Not scrubbing returns to full opacity (instant with reduced motion).
        assert_eq!(sample(false, Some(Pos2::new(80.0, 20.0)), true), 1.0);
    }
}
