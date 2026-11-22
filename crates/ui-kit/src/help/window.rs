//! The help window (spec: S1 Help "Help window"): a searchable topic list on
//! the left, the rendered topic on the right, Back and Forward above it. The
//! window is a flat panel, remembers its size and position, and closes with
//! Esc. Styling comes only from ui-kit tokens.

use egui::{Align, Context, CornerRadius, Key, Layout, Margin, Modifiers, Pos2, Rect, Response, Sense, Stroke, Ui, Window, vec2};

use crate::HAIRLINE;
use crate::Theme;
use crate::fonts;
use crate::icons::regular;
use crate::tokens::{color, radius, size, space, typography};

use super::markdown;
use super::search::{Hit, search};
use super::topic::{HelpBook, Topic};

/// Default window width (spec "Help window": 760 x 560).
const WINDOW_WIDTH: f32 = 760.0;
/// Default window height (spec "Help window": 760 x 560).
const WINDOW_HEIGHT: f32 = 560.0;
/// Width of the topic column on the left (spec "Help window": 240).
const LIST_WIDTH: f32 = 240.0;
/// Gap between the two columns.
const COLUMN_GAP: f32 = space::WINDOW_GAP;
/// Height of a section label row in the topic list.
const SECTION_HEIGHT: f32 = 24.0;
/// Padding inside a highlighted search result row, each side.
const RESULT_PAD: f32 = space::SCALE_4;
/// Vertical gap between two blocks in a topic.
const BLOCK_GAP: f32 = space::SCALE_4;
/// The search hint shown in the empty field.
const SEARCH_HINT: &str = "Search help";

/// The help window's open state and its Back and Forward history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpState {
    /// True while the window is open.
    pub open: bool,
    /// The id of the topic shown on the right.
    pub current: Option<String>,
    /// Topics visited before the current one, oldest first.
    pub back: Vec<String>,
    /// Topics Back walked away from, newest first.
    pub forward: Vec<String>,
    /// The search field's text.
    pub query: String,
    /// The selected row in the search result list.
    pub selected: usize,
}

/// The window's egui id, fixed so the size and position survive closing.
fn window_id() -> egui::Id {
    egui::Id::new("ui_kit_help_window")
}

/// The memory id marking that the window was open on the previous frame, so a
/// freshly opened window can put the caret in the search field.
fn shown_id() -> egui::Id {
    window_id().with("shown")
}

/// Opens the help window on `id`, pushing the current topic onto the Back
/// history.
pub fn open(state: &mut HelpState, id: &str) {
    state.open = true;
    navigate(state, id);
}

/// Draws the help window when it is open. `extra` may draw the content of a
/// generated topic instead of its Markdown: it gets the topic id and the
/// content ui, and returns true when it drew the page.
pub fn help_window(ctx: &Context, state: &mut HelpState, book: &HelpBook, extra: &mut dyn FnMut(&str, &mut Ui) -> bool) {
    if !state.open {
        return;
    }
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        state.open = false;
        ctx.data_mut(|data| data.remove::<bool>(shown_id()));
        return;
    }
    if ctx.input_mut(|input| input.consume_key(Modifiers::ALT, Key::ArrowLeft)) {
        go_back(state);
    }
    if ctx.input_mut(|input| input.consume_key(Modifiers::ALT, Key::ArrowRight)) {
        go_forward(state);
    }
    let focus_search = !ctx.data(|data| data.get_temp::<bool>(shown_id()).unwrap_or(false));
    ctx.data_mut(|data| data.insert_temp(shown_id(), true));

    let mut keep_open = true;
    let mut requested: Option<String> = None;
    let frame = egui::Frame::new()
        .fill(color::BG_PANEL)
        .corner_radius(radius::PANEL)
        .stroke(Stroke::new(HAIRLINE, color::STROKE_HAIRLINE))
        .inner_margin(Margin::same(space::PANEL_PADDING as i8));
    let default_pos = ctx.content_rect().center() - vec2(WINDOW_WIDTH, WINDOW_HEIGHT) / 2.0;
    Window::new("Help")
        .id(window_id())
        .default_pos(default_pos)
        .default_size(vec2(WINDOW_WIDTH, WINDOW_HEIGHT))
        .resizable(true)
        .collapsible(false)
        .open(&mut keep_open)
        .frame(frame)
        .show(ctx, |ui| {
            let height = ui.available_height();
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(LIST_WIDTH, height), Layout::top_down(Align::Min), |ui| {
                    topic_list(ui, state, book, focus_search, &mut requested);
                });
                ui.add_space(COLUMN_GAP);
                ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::top_down(Align::Min), |ui| {
                    topic_page(ui, state, book, extra, &mut requested);
                });
            });
        });
    if !keep_open {
        state.open = false;
        ctx.data_mut(|data| data.remove::<bool>(shown_id()));
    }
    if let Some(id) = requested {
        navigate(state, &id);
    }
}

/// Draws the search field and the topic or result list.
fn topic_list(ui: &mut Ui, state: &mut HelpState, book: &HelpBook, focus_search: bool, requested: &mut Option<String>) {
    let previous_query = state.query.clone();
    let response = crate::search_field(ui, &mut state.query, SEARCH_HINT);
    if focus_search {
        response.request_focus();
    }
    if state.query != previous_query {
        state.selected = 0;
    }
    ui.add_space(space::SCALE_8);

    let hits = search(book, &state.query);
    if !hits.is_empty() {
        state.selected = state.selected.min(hits.len() - 1);
    }
    if state.query.trim().is_empty() {
        ui.allocate_ui_with_layout(vec2(ui.available_width(), ui.available_height()), Layout::top_down(Align::Min), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("ui_kit_help_topics").show(ui, |ui| {
                for section in &book.sections {
                    section_label(ui, &section.name);
                    for &index in &section.topics {
                        let topic = &book.topics[index];
                        let selected = state.current.as_deref() == Some(topic.id.as_str());
                        if crate::list_row(ui, selected, None, &topic.title, None).clicked() {
                            *requested = Some(topic.id.clone());
                        }
                    }
                }
            });
        });
    } else {
        if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            state.selected = (state.selected + 1).min(hits.len().saturating_sub(1));
        }
        if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            state.selected = state.selected.saturating_sub(1);
        }
        if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
            && let Some(hit) = hits.get(state.selected)
        {
            *requested = Some(book.topics[hit.topic].id.clone());
        }
        ui.allocate_ui_with_layout(vec2(ui.available_width(), ui.available_height()), Layout::top_down(Align::Min), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("ui_kit_help_results").show(ui, |ui| {
                if hits.is_empty() {
                    ui.label(egui::RichText::new("No topics match").font(fonts::medium(typography::BODY_SIZE)).color(color::TEXT_TERTIARY));
                }
                for (index, hit) in hits.iter().enumerate() {
                    let topic = &book.topics[hit.topic];
                    if result_row(ui, topic, hit, index == state.selected).clicked() {
                        *requested = Some(topic.id.clone());
                    }
                }
            });
        });
    }
}

/// Draws a section name above its topic rows.
fn section_label(ui: &mut Ui, name: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), SECTION_HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let galley = ui.painter().layout_no_wrap(name.to_owned(), fonts::semibold(typography::CAPTION_SIZE), color::TEXT_TERTIARY);
        let position = Pos2::new(rect.left() + space::SCALE_4, rect.bottom() - galley.size().y - space::SCALE_4);
        ui.painter().galley(position, galley, color::TEXT_TERTIARY);
    }
}

/// Draws one search result row with its matched words in the accent color.
fn result_row(ui: &mut Ui, topic: &Topic, hit: &Hit, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::LIST_ROW_HEIGHT), Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            Some(color::BG_ROW_SELECTED)
        } else if response.hovered() {
            Some(color::BG_GROUP)
        } else {
            None
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, radius::SEGMENT, fill);
        }
        let text_color = if selected { color::TEXT_PRIMARY } else { color::TEXT_SECONDARY };
        paint_highlighted(ui, rect, &topic.title, &hit.words, text_color);
    }
    if response.has_focus() {
        crate::focus_ring(ui, rect, CornerRadius::from(radius::SEGMENT));
    }
    response
}

/// Paints a row title with the matched words in the accent color.
fn paint_highlighted(ui: &Ui, rect: Rect, title: &str, words: &[String], text_color: egui::Color32) {
    let accent = Theme::get(ui.ctx()).accent.accent;
    let painter = ui.painter().with_clip_rect(rect);
    let mut x = rect.left() + RESULT_PAD;
    for (text, highlighted) in highlight_runs(title, words) {
        let color = if highlighted { accent } else { text_color };
        let galley = painter.layout_no_wrap(text, fonts::medium(typography::BODY_SIZE), color);
        let size = galley.size();
        painter.galley(Pos2::new(x, rect.center().y - size.y / 2.0), galley, color);
        x += size.x;
        if x > rect.right() {
            break;
        }
    }
}

/// Splits a title into runs, marking the runs that hold a matched word.
fn highlight_runs(title: &str, words: &[String]) -> Vec<(String, bool)> {
    let chars: Vec<char> = title.chars().collect();
    let lowered: Vec<char> = title.to_lowercase().chars().collect();
    let mut marked = vec![false; chars.len()];
    if lowered.len() == chars.len() {
        for word in words {
            let needle: Vec<char> = word.chars().collect();
            if needle.is_empty() || needle.len() > lowered.len() {
                continue;
            }
            for start in 0..=lowered.len() - needle.len() {
                if lowered[start..start + needle.len()] == needle[..] {
                    for flag in &mut marked[start..start + needle.len()] {
                        *flag = true;
                    }
                }
            }
        }
    }
    let mut runs: Vec<(String, bool)> = Vec::new();
    for (index, character) in chars.iter().enumerate() {
        let highlighted = marked[index];
        match runs.last_mut() {
            Some((text, flag)) if *flag == highlighted => text.push(*character),
            _ => runs.push((character.to_string(), highlighted)),
        }
    }
    runs
}

/// Draws the Back and Forward buttons and the current topic.
fn topic_page(ui: &mut Ui, state: &mut HelpState, book: &HelpBook, extra: &mut dyn FnMut(&str, &mut Ui) -> bool, requested: &mut Option<String>) {
    ui.horizontal(|ui| {
        let back = ui.add_enabled_ui(!state.back.is_empty(), |ui| crate::icon_button(ui, regular::ARROW_LEFT, "Back (Alt+Left)")).inner;
        if back.clicked() {
            go_back(state);
        }
        let forward = ui.add_enabled_ui(!state.forward.is_empty(), |ui| crate::icon_button(ui, regular::ARROW_RIGHT, "Forward (Alt+Right)")).inner;
        if forward.clicked() {
            go_forward(state);
        }
    });
    ui.add_space(space::SCALE_8);
    // Read the topic after the buttons ran, so a Back or Forward click this
    // frame shows its topic instead of being overwritten by the old one.
    let current = state.current.clone().filter(|id| book.get(id).is_some()).or_else(|| book.first().map(|topic| topic.id.clone()));
    let Some(id) = current else {
        ui.label(egui::RichText::new("No help topics").font(fonts::medium(typography::BODY_SIZE)).color(color::TEXT_TERTIARY));
        return;
    };
    let Some(topic) = book.get(&id) else {
        return;
    };
    state.current = Some(id.clone());
    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("ui_kit_help_topic", id.as_str())).show(ui, |ui| {
        if extra(&topic.id, ui) {
            return;
        }
        for block in markdown::parse(&topic.body) {
            if let Some(target) = markdown::render(ui, &block) {
                *requested = Some(target);
            }
            ui.add_space(BLOCK_GAP);
        }
    });
}

/// Moves to `id`, pushing the current topic onto the Back history.
fn navigate(state: &mut HelpState, id: &str) {
    if state.current.as_deref() == Some(id) {
        return;
    }
    if let Some(current) = state.current.replace(id.to_string()) {
        state.back.push(current);
    }
    state.forward.clear();
}

/// Walks one step back through the history.
fn go_back(state: &mut HelpState) {
    if let Some(id) = state.back.pop()
        && let Some(current) = state.current.replace(id)
    {
        state.forward.push(current);
    }
}

/// Walks one step forward through the history.
fn go_forward(state: &mut HelpState) {
    if let Some(id) = state.forward.pop()
        && let Some(current) = state.current.replace(id)
    {
        state.back.push(current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opening topics pushes them onto Back and clears Forward.
    #[test]
    fn open_and_back_walk_the_history() {
        let mut state = HelpState::default();
        open(&mut state, "one");
        assert!(state.open);
        open(&mut state, "two");
        open(&mut state, "three");
        assert_eq!(state.back, ["one", "two"]);
        go_back(&mut state);
        assert_eq!(state.current.as_deref(), Some("two"));
        assert_eq!(state.forward, ["three"]);
        go_forward(&mut state);
        assert_eq!(state.current.as_deref(), Some("three"));
        assert_eq!(state.back, ["one", "two"]);
        assert!(state.forward.is_empty());
    }

    /// Opening the topic already shown changes nothing.
    #[test]
    fn opening_the_current_topic_keeps_the_history() {
        let mut state = HelpState::default();
        open(&mut state, "one");
        open(&mut state, "one");
        assert!(state.back.is_empty());
        assert!(state.forward.is_empty());
    }

    /// A title is split into plain and highlighted runs, ignoring case.
    #[test]
    fn highlight_runs_mark_matched_words() {
        let runs = highlight_runs("Recording takes", &["record".to_string()]);
        assert_eq!(runs, [("Record".to_string(), true), ("ing takes".to_string(), false)]);
        let runs = highlight_runs("Recording takes", &["record".to_string(), "takes".to_string()]);
        assert_eq!(runs, [("Record".to_string(), true), ("ing ".to_string(), false), ("takes".to_string(), true)]);
    }

    /// A title without a match stays one plain run.
    #[test]
    fn highlight_runs_without_a_match_keep_the_title_whole() {
        let runs = highlight_runs("Recording takes", &["trombone".to_string()]);
        assert_eq!(runs, [("Recording takes".to_string(), false)]);
    }

    /// The window draws a topic on a fresh context without panicking.
    #[test]
    fn help_window_draws_a_topic() {
        let source = "---\nid: one\ntitle: One\n---\n# One\n\nA **bold** word and a [link](topic:one).\n\n> Tip: press <kbd>F1</kbd>.\n";
        let book = HelpBook::from_sources(&[("one.md", source)]).unwrap();
        let mut state = HelpState::default();
        open(&mut state, "one");
        let mut extra = |_: &str, _: &mut egui::Ui| false;
        let ctx = Context::default();
        crate::apply(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            help_window(ui.ctx(), &mut state, &book, &mut extra);
        });
        output.textures_delta.clear();
        assert!(state.open);
    }
}
