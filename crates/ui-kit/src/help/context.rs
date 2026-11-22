//! The F1 context tags (spec: S1 Help "Help window" and "Topic files"). An
//! area registers its rectangle with a tag every frame; F1 opens the topic
//! whose tag matches the area under the pointer, else the focused area, else
//! "Getting started".

use egui::{Context, Key, Modifiers, Pos2, Rect, Ui};

use super::topic::HelpBook;
use super::window::{HelpState, open};

/// The topic F1 opens when no area is tagged.
const FALLBACK_TOPIC: &str = "getting-started";

/// The egui context data id holding this frame's registered areas.
fn registry_id() -> egui::Id {
    egui::Id::new("ui_kit_help_contexts")
}

/// Registers the area of `ui` as the F1 area `tag`. A more specific tag wins,
/// for example `timeline.record` over `timeline`. Register once per frame in
/// the area's own ui; the tag is looked up while handling F1 that frame.
pub fn context(ui: &Ui, tag: &str) {
    let frame = ui.ctx().cumulative_frame_nr();
    let rect = area_rect(ui);
    ui.ctx().data_mut(|data| {
        let key = registry_id();
        if data.get_temp::<u64>(key.with("frame")) != Some(frame) {
            data.insert_temp(key.with("frame"), frame);
            data.insert_temp(key, Vec::<(String, Rect)>::new());
        }
        let mut areas = data.get_temp::<Vec<(String, Rect)>>(key).unwrap_or_default();
        areas.push((tag.to_string(), rect));
        data.insert_temp(key, areas);
    });
}

/// The area a tag should cover: the smaller of the ui's own rectangle and its
/// clip rectangle. A scroll area's own rectangle slides with the content while
/// its clip rectangle is the visible viewport, so the smaller one keeps the tag
/// on the part the user can see. On a tie the clip rectangle wins.
fn area_rect(ui: &Ui) -> Rect {
    let own = ui.max_rect();
    let clip = ui.clip_rect();
    if clip.width() * clip.height() <= own.width() * own.height() { clip } else { own }
}

/// Opens the topic for the area under the pointer or the focus on F1. Returns
/// true when F1 was handled, so the app can stop looking at the key.
pub fn handle_f1(ctx: &Context, state: &mut HelpState, book: &HelpBook) -> bool {
    if !ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::F1)) {
        return false;
    }
    let areas = registered(ctx);
    let pointer = ctx.input(|input| input.pointer.hover_pos());
    let focused = ctx.memory(|memory| memory.focused()).and_then(|id| ctx.read_response(id)).map(|response| response.rect.center());
    let tag = pointer.and_then(|point| tag_at(&areas, point)).or_else(|| focused.and_then(|point| tag_at(&areas, point)));
    let topic = tag.as_deref().and_then(|tag| book.topic_for_context(tag)).or_else(|| book.get(FALLBACK_TOPIC));
    let Some(topic) = topic else {
        return false;
    };
    open(state, &topic.id);
    true
}

/// This frame's registered areas, in registration order.
fn registered(ctx: &Context) -> Vec<(String, Rect)> {
    ctx.data(|data| data.get_temp::<Vec<(String, Rect)>>(registry_id()).unwrap_or_default())
}

/// The most specific tag whose area holds `point`: the longest matching tag
/// wins, and between two unrelated tags the smaller area wins.
fn tag_at(areas: &[(String, Rect)], point: Pos2) -> Option<String> {
    let mut best: Option<&(String, Rect)> = None;
    for area in areas.iter().filter(|(_, rect)| rect.contains(point)) {
        best = match best {
            Some(current) if !more_specific(area, current) => Some(current),
            _ => Some(area),
        };
    }
    best.map(|(tag, _)| tag.clone())
}

/// True when `candidate` is the more specific of two areas holding the point.
fn more_specific(candidate: &(String, Rect), current: &(String, Rect)) -> bool {
    let (candidate_tag, candidate_rect) = candidate;
    let (current_tag, current_rect) = current;
    if candidate_tag.strip_prefix(current_tag.as_str()).is_some_and(|rest| rest.starts_with('.')) {
        return true;
    }
    if current_tag.strip_prefix(candidate_tag.as_str()).is_some_and(|rest| rest.starts_with('.')) {
        return false;
    }
    candidate_rect.width() * candidate_rect.height() < current_rect.width() * current_rect.height()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2, vec2};

    /// A three topic book: the timeline, its record area and the fallback.
    fn book() -> HelpBook {
        HelpBook::from_sources(&[
            ("timeline.md", "---\nid: timeline\ntitle: Timeline\ncontext: timeline\n---\n"),
            ("record.md", "---\nid: record\ntitle: Record\ncontext: timeline.record\n---\n"),
            ("start.md", "---\nid: getting-started\ntitle: Getting started\n---\n"),
        ])
        .expect("the test book is valid")
    }

    /// A more specific tag beats its parent, and the plain parent still
    /// matches outside the child area.
    #[test]
    fn tag_at_prefers_the_most_specific_area() {
        let areas = [
            ("timeline".to_string(), Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
            ("timeline.record".to_string(), Rect::from_min_size(Pos2::ZERO, vec2(100.0, 40.0))),
            ("master".to_string(), Rect::from_min_size(pos2(500.0, 500.0), vec2(20.0, 20.0))),
        ];
        assert_eq!(tag_at(&areas, pos2(10.0, 10.0)).as_deref(), Some("timeline.record"));
        assert_eq!(tag_at(&areas, pos2(200.0, 200.0)).as_deref(), Some("timeline"));
        assert_eq!(tag_at(&areas, pos2(505.0, 505.0)).as_deref(), Some("master"));
        assert_eq!(tag_at(&areas, pos2(700.0, 700.0)), None);
    }

    /// The registered area is the smaller of the ui's own rectangle and its
    /// clip rectangle, so a scrolled page keeps its visible viewport.
    #[test]
    fn area_rect_takes_the_smaller_visible_area() {
        let ctx = Context::default();
        let mut own = Rect::NOTHING;
        let mut clipped = Rect::NOTHING;
        let mut output = ctx.run_ui(raw_input(None), |ui| {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0))));
            child.set_clip_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0)));
            own = area_rect(&child);
            child.set_clip_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0, 40.0)));
            clipped = area_rect(&child);
        });
        output.textures_delta.clear();
        assert_eq!(own, Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)));
        assert_eq!(clipped, Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0, 40.0)));
    }

    /// Two unrelated tags over the same point: the smaller area wins.
    #[test]
    fn tag_at_prefers_the_smaller_unrelated_area() {
        let areas = [
            ("timeline".to_string(), Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
            ("master".to_string(), Rect::from_min_size(Pos2::ZERO, vec2(40.0, 40.0))),
        ];
        assert_eq!(tag_at(&areas, pos2(10.0, 10.0)).as_deref(), Some("master"));
    }

    /// F1 with no registered area opens Getting started.
    #[test]
    fn f1_without_a_tag_opens_getting_started() {
        let ctx = Context::default();
        let mut state = HelpState::default();
        let mut handled = false;
        let mut output = ctx.run_ui(raw_input(None), |ui| {
            handled = handle_f1(ui.ctx(), &mut state, &book());
        });
        output.textures_delta.clear();
        assert!(handled);
        assert!(state.open);
        assert_eq!(state.current.as_deref(), Some("getting-started"));
    }

    /// F1 under a tagged area opens that area's topic.
    #[test]
    fn f1_under_a_tagged_area_opens_its_topic() {
        let ctx = Context::default();
        let mut state = HelpState::default();
        let mut output = ctx.run_ui(raw_input(Some(pos2(20.0, 20.0))), |ui| {
            context(ui, "timeline.record");
            handle_f1(ui.ctx(), &mut state, &book());
        });
        output.textures_delta.clear();
        assert_eq!(state.current.as_deref(), Some("record"));
    }

    /// A raw input frame with F1 pressed and an optional pointer position.
    fn raw_input(pointer: Option<Pos2>) -> egui::RawInput {
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        if let Some(pointer) = pointer {
            input.events.push(egui::Event::PointerMoved(pointer));
        }
        input.events.push(egui::Event::Key { key: Key::F1, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        input
    }
}
