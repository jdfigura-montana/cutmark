//! The menu bar (spec: Design Language rev 3 "Title bar", left zone): one text
//! button per top-level menu, each opening a glass popover under it.

use egui::{Align, Event, Key, Layout, Margin, Modifiers, Popup, PopupKind, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::fonts;
use crate::tokens::{color, radius, space, typography};
use crate::widgets::{focus, glass};

/// Height of a menu title button (spec rev 3 "Title bar": 28 tall).
const TITLE_HEIGHT: f32 = 28.0;
/// Horizontal padding inside a menu title button (spec: 10).
const TITLE_PADDING: f32 = 10.0;
/// Corner radius of a menu title button (spec: 7).
const TITLE_RADIUS: f32 = 7.0;
/// Gap between neighbouring menu titles.
const TITLE_GAP: f32 = space::SCALE_2;
/// Gap between a menu title and the popover under it.
const POPOVER_GAP: f32 = space::SCALE_4;

/// One top-level menu: the title in the bar and the closure that fills its
/// popover. The closure runs every frame the menu is open; put the items in it
/// with `ui.button(..)` (call `ui.close()` after a click to close the menu).
pub struct Menu<'a> {
    /// The title shown in the bar. Alt plus its first letter opens the menu.
    pub title: &'a str,
    /// Fills the popover.
    contents: Box<dyn FnMut(&mut Ui) + 'a>,
}

impl<'a> Menu<'a> {
    /// A menu with its title and the closure that fills its popover.
    pub fn new(title: &'a str, contents: impl FnMut(&mut Ui) + 'a) -> Self {
        Self { title, contents: Box::new(contents) }
    }
}

/// What a [`menu_bar`] call reports.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuBarOutput {
    /// The rectangle of every menu title, in menu order.
    pub titles: Vec<Rect>,
    /// The index of the open menu, if any.
    pub open: Option<usize>,
}

/// The bar's state between frames, kept in egui's temporary memory.
#[derive(Debug, Clone, Copy, Default)]
struct State {
    /// The menu whose popover is open.
    open: Option<usize>,
    /// The title the keyboard sits on: set by Alt, the arrow keys and while a
    /// menu is open.
    focus: Option<usize>,
    /// True while Alt is held.
    alt_down: bool,
    /// True while Alt is held and nothing else was pressed since it went down:
    /// releasing it then is a tap, which enters the bar.
    alt_armed: bool,
    /// Where the pointer was on the previous frame, to tell a moving pointer
    /// from one that merely rests on a title.
    pointer: Option<egui::Pos2>,
}

/// The menu bar: one text button per menu in `type.body`, `text.secondary`,
/// 28 tall with 10 horizontal padding and radius 7; hover (and the open menu)
/// show `bg.hover` with `text.primary`.
///
/// Clicking a title opens its menu as a glass popover under it; while one menu
/// is open, moving the pointer onto another title switches to it, and clicking
/// the open title closes it. Esc closes the open menu. From the keyboard, a
/// tap of Alt moves to the first title, Left and Right move between titles,
/// Enter or Down opens the focused menu, and Alt plus a letter opens the menu
/// whose title starts with that letter.
///
/// Call it in a horizontal layout (the left zone of
/// [`crate::widgets::main_toolbar`]); the popover closure of every menu runs
/// only while that menu is open.
///
/// Usage:
///
/// ```ignore
/// let mut menus = [
///     ui_kit::Menu::new("File", |ui| {
///         if ui.button("Open").clicked() {
///             ui.close();
///         }
///     }),
///     ui_kit::Menu::new("View", |ui| {
///         ui.button("Zoom in");
///     }),
/// ];
/// ui_kit::menu_bar(ui, &mut menus);
/// ```
pub fn menu_bar(ui: &mut Ui, menus: &mut [Menu<'_>]) -> MenuBarOutput {
    let id = ui.unique_id().with("menu-bar");
    let dim = ui.visuals().override_text_color == Some(color::TEXT_TERTIARY);
    let idle = if dim { color::TEXT_TERTIARY } else { color::TEXT_SECONDARY };

    // Allocate every title first, so the interactions can settle the state
    // before anything is painted.
    let saved_spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = TITLE_GAP;
    let mut rects: Vec<Rect> = Vec::with_capacity(menus.len());
    let mut responses: Vec<Response> = Vec::with_capacity(menus.len());
    let mut galleys: Vec<std::sync::Arc<egui::Galley>> = Vec::with_capacity(menus.len());
    for menu in menus.iter() {
        let galley = ui.painter().layout_no_wrap(menu.title.to_owned(), fonts::medium(typography::BODY_SIZE), idle);
        let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 2.0 * TITLE_PADDING, TITLE_HEIGHT), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, menu.title));
        rects.push(rect);
        responses.push(response);
        galleys.push(galley);
    }
    ui.spacing_mut().item_spacing.x = saved_spacing;
    // A measuring pass only needs the sizes; it never owns state or popovers.
    if ui.is_sizing_pass() {
        return MenuBarOutput { titles: rects, open: None };
    }

    let count = menus.len();
    let mut state: State = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    state.open = state.open.filter(|&index| index < count);
    state.focus = state.focus.filter(|&index| index < count);
    let started_open = state.open;
    let esc_pressed = ui.input(|i| i.key_pressed(Key::Escape));

    track_alt(ui, &mut state, count);
    if let Some(index) = alt_letter(ui, menus) {
        state.open = Some(index);
        state.focus = Some(index);
    }
    keyboard_move(ui, &mut state, started_open, count);

    // The pointer. A click wins over hover; while a menu is open the next
    // title under a moving pointer takes over.
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let pointer_moved = pointer != state.pointer;
    state.pointer = pointer;
    let clicked = responses.iter().position(Response::clicked);
    let hovered = responses.iter().position(Response::hovered);
    if let Some(index) = clicked {
        let next = if started_open == Some(index) { None } else { Some(index) };
        state.open = next;
        state.focus = next;
    } else if let Some(over) = hovered {
        if started_open.is_some() && state.open == started_open && started_open != Some(over) && pointer_moved {
            state.open = Some(over);
            state.focus = Some(over);
        }
    } else if state.open.is_none() && state.focus.is_some() && ui.input(|i| i.pointer.any_pressed()) {
        // A click anywhere else leaves the keyboard mode of the bar.
        state.focus = None;
    }

    // Paint the titles.
    for (index, galley) in galleys.into_iter().enumerate() {
        let (rect, response) = (rects[index], &responses[index]);
        let keyboard = state.focus == Some(index);
        let lit = state.open == Some(index) || keyboard || (response.hovered() && state.open.is_none());
        if lit {
            ui.painter().rect_filled(rect, TITLE_RADIUS, color::BG_HOVER);
        }
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley_with_override_text_color(pos, galley, if lit { color::TEXT_PRIMARY } else { idle });
        if (keyboard && state.open.is_none()) || response.has_focus() {
            focus::focus_ring(ui, rect, TITLE_RADIUS.into());
        }
    }

    // The open menu's glass popover, under its title.
    if let Some(index) = state.open {
        let mut open = true;
        let ctx = ui.ctx().clone();
        let popup_id = Popup::default_response_id(&responses[index]);
        let was_shown = ctx.read_response(popup_id).is_some();
        let frame = glass::glass_frame(&ctx, radius::GROUP).inner_margin(Margin::same(space::SCALE_6 as i8));
        let shown = Popup::from_response(&responses[index])
            .kind(PopupKind::Menu)
            .layout(Layout::top_down_justified(Align::Min))
            .style(egui::containers::menu::menu_style as fn(&mut egui::Style))
            .gap(POPOVER_GAP)
            .frame(frame)
            .open_bool(&mut open)
            .show(|ui| (menus[index].contents)(ui));
        // The first frame of a popover is an invisible sizing pass.
        if let (Some(shown), true) = (shown, was_shown) {
            glass::paint_glass_highlight(&ctx.layer_painter(shown.response.layer_id), shown.response.rect, radius::GROUP);
        }
        if !open {
            state.open = None;
            // Esc steps back to the title; any other close leaves the bar.
            state.focus = if esc_pressed { Some(index) } else { None };
        }
    }

    ui.data_mut(|d| d.insert_temp(id, state));
    MenuBarOutput { titles: rects, open: state.open }
}

/// Follows the Alt key: a tap (down and up with nothing pressed in between)
/// enters the bar on the first title, or leaves it when it is already entered.
fn track_alt(ui: &Ui, state: &mut State, count: usize) {
    let (alt, key_pressed, pointer_pressed) =
        ui.input(|i| (i.modifiers.alt, i.events.iter().any(|event| matches!(event, Event::Key { pressed: true, .. })), i.pointer.any_pressed()));
    if alt && !state.alt_down {
        state.alt_armed = true;
    }
    if alt && (key_pressed || pointer_pressed) {
        state.alt_armed = false;
    }
    if !alt && state.alt_down && state.alt_armed {
        state.alt_armed = false;
        if state.open.is_some() || state.focus.is_some() {
            state.open = None;
            state.focus = None;
        } else if count > 0 {
            state.focus = Some(0);
        }
    }
    state.alt_down = alt;
}

/// Alt plus a letter: the first menu whose title starts with that letter.
/// Consumes the key so nothing else reacts to it.
fn alt_letter(ui: &Ui, menus: &[Menu<'_>]) -> Option<usize> {
    menus.iter().position(|menu| {
        let Some(letter) = menu.title.chars().next() else {
            return false;
        };
        let Some(key) = Key::from_name(&letter.to_uppercase().to_string()) else {
            return false;
        };
        ui.input_mut(|i| i.consume_key(Modifiers::ALT, key))
    })
}

/// The arrow keys, Enter and Esc on the bar. Left and Right move between the
/// titles (also while a menu is open, which then switches menus); Enter and
/// Down open the focused menu; Esc leaves the bar when no menu is open (an
/// open menu closes itself on Esc).
fn keyboard_move(ui: &Ui, state: &mut State, started_open: Option<usize>, count: usize) {
    let Some(focus) = state.focus else {
        return;
    };
    if count == 0 {
        return;
    }
    let key = |key: Key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
    let mut moved = None;
    if key(Key::ArrowLeft) {
        moved = Some((focus + count - 1) % count);
    } else if key(Key::ArrowRight) {
        moved = Some((focus + 1) % count);
    }
    if let Some(next) = moved {
        state.focus = Some(next);
        if started_open.is_some() {
            state.open = Some(next);
        }
        return;
    }
    if started_open.is_none() {
        if key(Key::Enter) || key(Key::ArrowDown) {
            state.open = Some(focus);
        } else if key(Key::Escape) {
            state.focus = None;
        }
    }
}
