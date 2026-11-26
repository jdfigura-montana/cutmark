//! The first-run spotlight tour (spec: S1 Help "First-run tour"): an app
//! registers the rectangle of each named target every frame, then calls
//! [`show_tour`], which dims the window, cuts a rounded hole around the
//! current step's target and draws a glass callout card beside it.
//! Styling comes only from ui-kit tokens.

use egui::{Context, Id, Key, Margin, Modifiers, Order, Pos2, Rect, RichText, Stroke, Ui, Vec2, pos2, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{motion, radius, space, typography};
use crate::widgets::glass_frame;

/// Alpha of the dimmed window behind the spotlight (spec "First-run tour":
/// `bg.ground` at 70%).
const DIM_ALPHA: f32 = 0.7;
/// Corner radius of the spotlight cutout (spec "First-run tour": 12).
const CUTOUT_RADIUS: f32 = 12.0;
/// Padding of the cutout around its target (spec "First-run tour": 6).
const CUTOUT_PAD: f32 = 6.0;
/// Segments per rounded corner of the cutout.
const CORNER_SEGMENTS: usize = 6;
/// Gap between the target and the callout card.
const CARD_GAP: f32 = space::SCALE_12;
/// Width of the callout card, so its text wraps predictably.
const CARD_WIDTH: f32 = 300.0;
/// Padding inside the callout card.
const CARD_PADDING: f32 = space::SCALE_16;
/// Gap between the callout card's lines.
const CARD_LINE_GAP: f32 = space::SCALE_8;

/// Registers the rectangle of `target` for this frame, so a tour step whose
/// target is on screen can spotlight it. Call once per frame in the target's
/// own ui; a step whose target is not registered this frame is skipped.
pub fn anchor(ui: &Ui, target: &str, rect: Rect) {
    let frame = ui.ctx().cumulative_frame_nr();
    let key = registry_id();
    ui.ctx().data_mut(|data| {
        if data.get_temp::<u64>(key.with("frame")) != Some(frame) {
            data.insert_temp(key.with("frame"), frame);
            data.insert_temp(key, Vec::<(String, Rect)>::new());
        }
        let mut anchors = data.get_temp::<Vec<(String, Rect)>>(key).unwrap_or_default();
        anchors.push((target.to_string(), rect));
        data.insert_temp(key, anchors);
    });
}

/// The egui context data id holding this frame's registered targets.
fn registry_id() -> Id {
    Id::new("ui_kit_tour_anchors")
}

/// This frame's registered targets, in registration order.
fn anchors(ctx: &Context) -> Vec<(String, Rect)> {
    ctx.data(|data| data.get_temp::<Vec<(String, Rect)>>(registry_id()).unwrap_or_default())
}

/// The rectangle registered for `target` this frame, if any.
fn anchor_rect(anchors: &[(String, Rect)], target: &str) -> Option<Rect> {
    anchors.iter().find(|(name, _)| name == target).map(|(_, rect)| *rect)
}

/// One tour step: the target it spotlights and its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TourStep {
    /// The anchor id the step spotlights.
    pub target: &'static str,
    /// The card's title, drawn in `type.headline`.
    pub title: &'static str,
    /// The card's body text.
    pub body: &'static str,
}

/// One app's tour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tour {
    /// The app folder name, for the seen flag.
    pub app: &'static str,
    /// The tour id, for the seen flag.
    pub id: &'static str,
    /// The tour version, so a new tour version can show again later.
    pub version: u32,
    /// The steps, in order.
    pub steps: &'static [TourStep],
}

/// What ended a tour this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourEvent {
    /// The last step was finished.
    Finished,
    /// The user skipped the tour.
    Skipped,
}

/// The tour's running state: whether it is open and which step it is on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TourState {
    /// True while the tour is showing.
    pub open: bool,
    /// The index of the current step.
    pub step: usize,
}

impl TourState {
    /// Starts the tour on its first step.
    pub fn start(&mut self) {
        self.open = true;
        self.step = 0;
    }
}

/// What a callout button asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Go to the previous step.
    Back,
    /// Go to the next step, or finish.
    Next,
    /// Skip the tour.
    Skip,
}

/// Draws the tour while it is open, and reports what ended it this frame.
///
/// Esc skips, Enter or Right is Next, Left is Back; a step whose target is not
/// registered this frame is skipped. Each step dims the window, cuts a rounded
/// hole around its target and draws a glass callout card beside it.
pub fn show_tour(ctx: &Context, state: &mut TourState, tour: &Tour) -> Option<TourEvent> {
    if !state.open || tour.steps.is_empty() {
        return None;
    }
    let registered = anchors(ctx);
    let current = current_step(tour, &registered, state.step)?;
    state.step = current;

    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        state.open = false;
        return Some(TourEvent::Skipped);
    }
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter) || input.consume_key(Modifiers::NONE, Key::ArrowRight)) {
        return advance(tour, &registered, state, 1);
    }
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowLeft)) {
        return advance(tour, &registered, state, -1);
    }

    let target = anchor_rect(&registered, tour.steps[state.step].target)?;
    let screen = ctx.content_rect();
    let cutout = animate_cutout(ctx, target.expand(CUTOUT_PAD));
    // The dim is a foreground area created before the card, so it paints below
    // it; it is not interactable, so clicks reach the app behind it.
    egui::Area::new(Id::new("ui_kit_tour_dim")).order(Order::Foreground).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
        ui.set_min_size(screen.size());
        paint_spotlight(ui.painter(), screen, cutout);
    });
    let response = egui::Area::new(Id::new("ui_kit_tour_callout"))
        .order(Order::Foreground)
        .fixed_pos(card_position(cutout, screen))
        .constrain(true)
        .show(ctx, |ui| card(ui, tour, state.step, &registered));
    match response.inner {
        Some(Action::Back) => advance(tour, &registered, state, -1),
        Some(Action::Next) => advance(tour, &registered, state, 1),
        Some(Action::Skip) => {
            state.open = false;
            Some(TourEvent::Skipped)
        }
        None => None,
    }
}

/// Moves the tour one step forward (finishing at the end) or back.
fn advance(tour: &Tour, anchors: &[(String, Rect)], state: &mut TourState, direction: isize) -> Option<TourEvent> {
    if direction > 0 {
        match next_step(tour, anchors, state.step) {
            Some(index) => {
                state.step = index;
                None
            }
            None => {
                state.open = false;
                Some(TourEvent::Finished)
            }
        }
    } else {
        if let Some(index) = previous_step(tour, anchors, state.step) {
            state.step = index;
        }
        None
    }
}

/// The first step at or after `from` whose target is registered this frame.
fn current_step(tour: &Tour, anchors: &[(String, Rect)], from: usize) -> Option<usize> {
    (from.min(tour.steps.len())..tour.steps.len()).find(|&index| anchor_rect(anchors, tour.steps[index].target).is_some())
}

/// The next step after `from` whose target is registered this frame.
fn next_step(tour: &Tour, anchors: &[(String, Rect)], from: usize) -> Option<usize> {
    ((from + 1).min(tour.steps.len())..tour.steps.len()).find(|&index| anchor_rect(anchors, tour.steps[index].target).is_some())
}

/// The last step before `from` whose target is registered this frame.
fn previous_step(tour: &Tour, anchors: &[(String, Rect)], from: usize) -> Option<usize> {
    (0..from.min(tour.steps.len())).rev().find(|&index| anchor_rect(anchors, tour.steps[index].target).is_some())
}
/// Moves the cutout to its target over the panel motion token; instant when
/// the theme asks for reduced motion.
fn animate_cutout(ctx: &Context, target: Rect) -> Rect {
    let duration = if Theme::get(ctx).reduce_motion { 0.0 } else { motion::PANEL_OPEN_CLOSE_MS as f32 / 1000.0 };
    let id = Id::new("ui_kit_tour_cutout");
    let x = ctx.animate_value_with_time(id.with("x"), target.left(), duration);
    let y = ctx.animate_value_with_time(id.with("y"), target.top(), duration);
    let width = ctx.animate_value_with_time(id.with("w"), target.width(), duration);
    let height = ctx.animate_value_with_time(id.with("h"), target.height(), duration);
    Rect::from_min_size(pos2(x, y), vec2(width, height))
}

/// Where the callout card goes: beside the cutout, right when it fits, else
/// left, in both cases clamped to the screen's left edge.
fn card_position(cutout: Rect, screen: Rect) -> Pos2 {
    let right = cutout.right() + CARD_GAP;
    if right + CARD_WIDTH <= screen.right() {
        pos2(right, cutout.top())
    } else {
        pos2((cutout.left() - CARD_GAP - CARD_WIDTH).max(screen.left()), cutout.top())
    }
}

/// Paints the dim over the window with a rounded cutout around the target.
fn paint_spotlight(painter: &egui::Painter, screen: Rect, cutout: Rect) {
    let neutrals = Theme::get(painter.ctx()).neutrals;
    let dim = neutrals.bg_ground.gamma_multiply(DIM_ALPHA);
    let cut = cutout.intersect(screen);
    if !cut.is_positive() {
        painter.rect_filled(screen, 0.0, dim);
        return;
    }
    let (left, top, right, bottom) = (cut.left(), cut.top(), cut.right(), cut.bottom());
    // Four strips around the cutout's bounding box, then one wedge per rounded
    // corner: the dim covers everything except the rounded cutout.
    painter.rect_filled(Rect::from_min_max(screen.min, pos2(screen.right(), top)), 0.0, dim);
    painter.rect_filled(Rect::from_min_max(pos2(screen.left(), bottom), screen.max), 0.0, dim);
    painter.rect_filled(Rect::from_min_max(pos2(screen.left(), top), pos2(left, bottom)), 0.0, dim);
    painter.rect_filled(Rect::from_min_max(pos2(right, top), pos2(screen.right(), bottom)), 0.0, dim);
    let corner = CUTOUT_RADIUS.min(cut.width() / 2.0).min(cut.height() / 2.0);
    if corner <= 0.0 {
        return;
    }
    use std::f32::consts::{FRAC_PI_2, PI};
    let wedges = [
        (pos2(left, top), pos2(left + corner, top + corner), PI, 1.5 * PI),
        (pos2(right, top), pos2(right - corner, top + corner), 1.5 * PI, 2.0 * PI),
        (pos2(right, bottom), pos2(right - corner, bottom - corner), 0.0, FRAC_PI_2),
        (pos2(left, bottom), pos2(left + corner, bottom - corner), FRAC_PI_2, PI),
    ];
    for (corner_point, center, start, end) in wedges {
        painter.add(egui::Shape::convex_polygon(corner_wedge(corner_point, center, corner, start, end), dim, Stroke::NONE));
    }
}

/// The wedge a rounded corner leaves outside the cutout: the corner point plus
/// the arc of that corner's quarter circle.
fn corner_wedge(corner: Pos2, center: Pos2, radius: f32, start: f32, end: f32) -> Vec<Pos2> {
    let mut points = Vec::with_capacity(CORNER_SEGMENTS + 2);
    points.push(corner);
    for index in 0..=CORNER_SEGMENTS {
        let angle = start + (end - start) * index as f32 / CORNER_SEGMENTS as f32;
        points.push(center + Vec2::angled(angle) * radius);
    }
    points
}

/// The callout card: the step's title in `type.headline`, its body, "Step n of
/// m", and Back, Next (primary) and Skip tour. Returns the button clicked.
fn card(ui: &mut Ui, tour: &Tour, index: usize, anchors: &[(String, Rect)]) -> Option<Action> {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let step = tour.steps[index];
    ui.set_width(CARD_WIDTH);
    let mut action = None;
    glass_frame(ui.ctx(), radius::GROUP).inner_margin(Margin::same(CARD_PADDING as i8)).show(ui, |ui| {
        ui.set_width(CARD_WIDTH - 2.0 * CARD_PADDING);
        ui.label(RichText::new(step.title).font(fonts::semibold(typography::HEADLINE_SIZE)).color(neutrals.text_primary));
        ui.add_space(CARD_LINE_GAP);
        ui.label(RichText::new(step.body).font(fonts::medium(typography::BODY_SIZE)).color(neutrals.text_secondary));
        ui.add_space(CARD_LINE_GAP);
        let progress = format!("Step {} of {}", index + 1, tour.steps.len());
        ui.label(RichText::new(progress).font(fonts::medium(typography::CAPTION_SIZE)).color(neutrals.text_tertiary));
        ui.add_space(CARD_LINE_GAP);
        ui.horizontal(|ui| {
            let has_back = previous_step(tour, anchors, index).is_some();
            if ui.add_enabled(has_back, egui::Button::new("Back")).clicked() {
                action = Some(Action::Back);
            }
            if crate::primary_button(ui, "Next").clicked() {
                action = Some(Action::Next);
            }
            if ui.button("Skip tour").clicked() {
                action = Some(Action::Skip);
            }
        });
    });
    action
}

/// The tour's "seen" flag: `help.json` in the app's Cutmark config folder
/// (spec: S1 Help "First-run tour"). A missing or corrupt file means the tour
/// has not been seen, so a fresh profile shows it once.
pub mod help_store {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    /// The file name inside the app's help folder.
    const FILE_NAME: &str = "help.json";

    /// The on-disk JSON: the seen version of every tour, by tour id.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct File {
        seen: BTreeMap<String, u32>,
    }

    /// The app's help folder: `CUTMARK_HELP_DIR` if set (tests use it), else
    /// the Cutmark folder in the OS config directory, then the app name.
    fn folder(app: &str) -> Option<PathBuf> {
        if let Ok(from_env) = std::env::var("CUTMARK_HELP_DIR")
            && !from_env.is_empty()
        {
            return Some(PathBuf::from(from_env).join(app));
        }
        config_folder().map(|base| base.join(app))
    }

    /// The Cutmark config folder for this OS (the same rules as the
    /// appearance file).
    fn config_folder() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("Cutmark"))
        }
        #[cfg(target_os = "macos")]
        {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Application Support").join("Cutmark"))
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
                let base = PathBuf::from(base);
                if base.is_absolute() {
                    return Some(base.join("cutmark"));
                }
            }
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("cutmark"))
        }
    }

    /// The app's help file, or None when no folder is available.
    fn path(app: &str) -> Option<PathBuf> {
        folder(app).map(|folder| folder.join(FILE_NAME))
    }

    /// Reads the seen versions; any problem (missing or corrupt file) gives an
    /// empty map, so the tour shows.
    fn load(app: &str) -> BTreeMap<String, u32> {
        let Some(path) = path(app) else { return BTreeMap::new() };
        let Ok(text) = fs::read_to_string(path) else { return BTreeMap::new() };
        serde_json::from_str::<File>(&text).map(|file| file.seen).unwrap_or_default()
    }

    /// True when `tour_id` has been seen at `version` or later.
    pub fn is_seen(app: &str, tour_id: &str, version: u32) -> bool {
        load(app).get(tour_id).is_some_and(|seen| *seen >= version)
    }

    /// Records `tour_id` as seen at `version`, keeping any higher version
    /// already stored. Writes a temp file beside the target and renames it over
    /// it; a write problem is returned as its message.
    pub fn mark_seen(app: &str, tour_id: &str, version: u32) -> Result<(), String> {
        let path = path(app).ok_or_else(|| "no help file path: CUTMARK_HELP_DIR and the OS config folder are both unset".to_string())?;
        let folder = path.parent().ok_or_else(|| format!("{} has no parent folder", path.display()))?;
        fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
        let mut seen = load(app);
        seen.entry(tour_id.to_string()).and_modify(|stored| *stored = (*stored).max(version)).or_insert(version);
        let json = serde_json::to_string(&File { seen }).map_err(|error| format!("cannot encode {}: {error}", path.display()))?;
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, json).map_err(|error| format!("cannot write {}: {error}", temp.display()))?;
        fs::rename(&temp, &path).map_err(|error| format!("cannot move {} over {}: {error}", temp.display(), path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// One lock for the tests that touch the process-wide help dir env var.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// The demo tour's steps, over three named targets.
    const STEPS: [TourStep; 3] = [
        TourStep { target: "toolbar", title: "Toolbar", body: "One." },
        TourStep { target: "panel", title: "Panel", body: "Two." },
        TourStep { target: "transport", title: "Transport", body: "Three." },
    ];

    /// The demo tour.
    const TOUR: Tour = Tour { app: "test", id: "demo", version: 1, steps: &STEPS };

    /// A context with the ui-kit fonts installed (one warm-up pass, since a
    /// font change takes effect at the start of the next pass).
    fn test_ctx() -> Context {
        let ctx = Context::default();
        crate::apply(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx
    }

    /// Runs one frame: registers `targets` at one rect, then draws the tour
    /// with an optional key pressed this frame.
    fn frame(ctx: &Context, state: &mut TourState, targets: &[&str], key: Option<Key>) -> Option<TourEvent> {
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        if let Some(key) = key {
            input.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        }
        let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(200.0, 60.0));
        let mut event = None;
        let mut output = ctx.run_ui(input, |ui| {
            for target in targets {
                anchor(ui, target, rect);
            }
            event = show_tour(ui.ctx(), state, &TOUR);
        });
        output.textures_delta.clear();
        event
    }

    /// A step whose target is not registered this frame is skipped.
    #[test]
    fn tour_skips_a_missing_target() {
        let ctx = test_ctx();
        let mut state = TourState::default();
        state.start();
        let event = frame(&ctx, &mut state, &["panel", "transport"], None);
        assert_eq!(state.step, 1, "the missing first target is skipped");
        assert_eq!(event, None, "skipping a target is not an event");
    }

    /// Right (or Enter) is Next and Left is Back.
    #[test]
    fn back_and_next_move_between_steps() {
        let ctx = test_ctx();
        let mut state = TourState::default();
        state.start();
        let all = ["toolbar", "panel", "transport"];
        frame(&ctx, &mut state, &all, None);
        assert_eq!(state.step, 0);
        frame(&ctx, &mut state, &all, Some(Key::ArrowRight));
        assert_eq!(state.step, 1, "Right moves forward");
        frame(&ctx, &mut state, &all, Some(Key::ArrowRight));
        assert_eq!(state.step, 2, "Right moves forward again");
        frame(&ctx, &mut state, &all, Some(Key::ArrowLeft));
        assert_eq!(state.step, 1, "Left moves back");
    }

    /// Esc skips the tour and reports Skipped.
    #[test]
    fn esc_returns_skipped() {
        let ctx = test_ctx();
        let mut state = TourState::default();
        state.start();
        let event = frame(&ctx, &mut state, &["toolbar"], Some(Key::Escape));
        assert_eq!(event, Some(TourEvent::Skipped));
        assert!(!state.open, "Esc closes the tour");
    }

    /// Next on the last step finishes the tour and reports Finished.
    #[test]
    fn finishing_returns_finished() {
        let ctx = test_ctx();
        let mut state = TourState::default();
        state.start();
        let two = ["toolbar", "panel"];
        frame(&ctx, &mut state, &two, None);
        frame(&ctx, &mut state, &two, Some(Key::ArrowRight));
        assert_eq!(state.step, 1);
        let event = frame(&ctx, &mut state, &two, Some(Key::ArrowRight));
        assert_eq!(event, Some(TourEvent::Finished));
        assert!(!state.open, "finishing closes the tour");
    }

    /// The seen flag round-trips, and a corrupt file reads as not seen.
    #[test]
    fn seen_flag_round_trips_and_corrupt_reads_as_not_seen() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = std::env::temp_dir().join(format!("cutmark-ui-kit-help-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        // SAFETY: the lock keeps every other test out of the process-wide var.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("CUTMARK_HELP_DIR", &dir)
        };

        assert!(!help_store::is_seen("app", "demo", 1), "a missing file means not seen");
        help_store::mark_seen("app", "demo", 1).expect("mark seen");
        assert!(help_store::is_seen("app", "demo", 1), "a seen tour stays seen");
        assert!(help_store::is_seen("app", "demo", 0), "a lower version counts as seen");
        assert!(!help_store::is_seen("app", "demo", 2), "a newer version shows again");
        assert!(!help_store::is_seen("app", "other", 1), "another tour is its own flag");

        let file = dir.join("app").join("help.json");
        std::fs::write(&file, "{ not json").expect("write corrupt help file");
        assert!(!help_store::is_seen("app", "demo", 1), "a corrupt file means not seen");

        help_store::mark_seen("app", "demo", 3).expect("mark seen again");
        assert!(help_store::is_seen("app", "demo", 3), "marking seen repairs the file");

        #[allow(unsafe_code)]
        unsafe {
            std::env::remove_var("CUTMARK_HELP_DIR")
        };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
