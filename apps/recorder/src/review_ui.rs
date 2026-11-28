//! The Review mode panels (spec "Review mode"): the takes list on the left,
//! the take's playback in the center and the take info on the right.
//!
//! The center is a stage that plays the selected take through the app's
//! [`Playback`]: the current frame letterboxed in a 16:9 box (uploaded to one
//! reused texture) and a scrub bar under the box that seeks by click or drag.
//! The stage's bottom bar holds the timecode, Add mark, previous and next
//! mark, Stop and the total length, and Play sits in its notch. Clicking a
//! mark in the right panel's Marks list seeks to it, and while the take plays
//! the mark under the playhead is selected. The Add mark action reports
//! through [`CenterAction`]; the app does the sidecar work.
//! The Actions group and the delete confirmation dialog report through
//! [`TakeAction`] and the app does the file work.
//!
//! A take with an input track has a cursor icon in its row, a Show cursor switch
//! and the track's counts in the File group (input track spec, "Review"), and Open
//! in Studio in Actions (Studio spec "Mode"). Open in Studio replaced Bake with
//! cursor; the bake code stays until Studio's export lands in T6.

use std::path::PathBuf;
use std::time::Duration;

use egui::{
    Align, Button, Color32, ColorImage, CornerRadius, Layout, Pos2, Rect, Response, RichText, Sense, TextureHandle, TextureOptions, Ui, WidgetInfo, WidgetType,
    pos2, vec2,
};
use marks::MarkType;
use media_io::player::video::RgbaFrame;
use ui_kit::tokens::{color, radius, size, space, typography};
use ui_kit::{
    MarkerKind, PlayKind, PrimaryButton, STAGE_BAR_BUTTON_DIAMETER, StageResponse, Theme, inspector_group, inspector_row, marker_color, paint_marker,
    paint_playhead,
};

use crate::bar_ui::{self, BarButton, Companion};
use crate::config::CompositeMode;
use crate::data_text;
use crate::layout_ui;
use crate::playback::Playback;
use crate::review::bake::{self, BakeJob, BakeRequest, Baked, Capturing};
use crate::review::{self, Day, ReviewView, Take, TrackSummary};

/// The Takes panel's two views, in the order the segmented control shows them.
const VIEWS: [(ReviewView, &str); 2] = [(ReviewView::Takes, "Takes"), (ReviewView::Replays, "Replays")];

/// Left padding of a panel header's title, the same inset the app's other
/// headers use.
const HEADER_PAD: f32 = 6.0;

/// The 1px hairline stroke width.
const HAIRLINE: f32 = 1.0;

/// Height of the scrub bar's interaction rect: the playhead head is 16 x 20
/// (the ui_kit playhead style), so the bar is 20 tall and the track inside it
/// is 12 (no size token for the review scrub track).
const SCRUB_HEIGHT: f32 = 20.0;

/// Height of the scrub bar's track itself.
const SCRUB_TRACK_HEIGHT: f32 = 12.0;

/// Gap between the 16:9 box and the scrub bar under it.
const SCRUB_GAP: f32 = space::SCALE_8;

/// How many frames the center keeps repainting after a seek while the take is
/// paused, so the decoded picture arrives; the decoder delivers within a few
/// hundred ms.
const SETTLE_FRAMES: u8 = 8;

/// How long each settle frame waits.
const SETTLE_DELAY: Duration = Duration::from_millis(50);

/// What the takes panel's header buttons did this frame.
#[derive(Debug, PartialEq, Eq)]
pub enum TakesAction {
    /// Nothing was clicked.
    None,
    /// Open the folder of the current view.
    OpenFolder,
    /// Rescan the takes.
    Refresh,
    /// The other view was picked.
    ViewChanged,
}

/// What the take panel's Actions group or Marks list did this frame
/// (spec "Review mode").
#[derive(Debug, PartialEq)]
pub enum TakeAction {
    /// Nothing was clicked.
    None,
    /// Open Explorer with the take's main file selected.
    ShowInFolder,
    /// Confirm the rename to this name; the app validates it and renames every file.
    Rename(String),
    /// Seek the playback to this position, in milliseconds from a mark row click.
    Seek(u64),
    /// The Show cursor switch was set to this.
    ShowCursor(bool),
    /// Bake the selected take with its cursor.
    // No button asks for this any more. T6's export supersedes the bake: delete it there.
    #[allow(dead_code)]
    Bake,
    /// Open the selected take in Studio.
    OpenInStudio,
}

/// What the panel knows about the selected take's input track, read once per file.
struct TrackInfo {
    path: PathBuf,
    /// The file's size at the scan, so a file that changed since is read again.
    size: u64,
    /// Whether the take's video shows the screen, where the cursor belongs.
    screen: bool,
    /// None when the file could not be read.
    summary: Option<TrackSummary>,
}

/// What to tell the user when a bake has ended.
#[derive(Debug, PartialEq, Eq)]
pub struct BakeReport {
    /// The toast's text.
    pub text: String,
    /// The baked file, for the toast's Show in folder button; None when the bake failed.
    pub file: Option<PathBuf>,
}

impl BakeReport {
    /// The report of a bake of the take named `take`.
    fn of(take: &str, outcome: Result<Baked, String>) -> Self {
        match outcome {
            Ok(baked) => {
                let note = baked.note.map(|note| format!(". {note}")).unwrap_or_default();
                Self { text: format!("Baked {}{note}", crate::profile_ui::take_label(take)), file: Some(baked.file) }
            }
            Err(reason) => Self { text: format!("Bake failed: {reason}"), file: None },
        }
    }
}

/// The take panel's state: the open rename field, the delete dialog, the Show cursor switch and the running bake.
pub struct TakePanelState {
    /// The rename buffer while the take's name is being edited.
    pub renaming: Option<String>,
    /// Whether the rename field already took focus, so focus is only requested once.
    rename_focused: bool,
    /// The reason the last rename was refused, shown under the field.
    pub rename_error: Option<String>,
    /// Whether the delete confirmation dialog is open.
    pub confirm_delete: bool,
    /// Whether playback draws the cursor; on until the switch is turned off, and the same for every take.
    pub show_cursor: bool,
    /// The selected take's input track.
    track: Option<TrackInfo>,
    /// The running bake. It goes on while another take or mode is on screen.
    bake: Option<BakeJob>,
    /// What the app was capturing when last told, so a recording that starts is noticed once.
    capturing: Capturing,
}

impl Default for TakePanelState {
    fn default() -> Self {
        Self {
            renaming: None,
            rename_focused: false,
            rename_error: None,
            confirm_delete: false,
            show_cursor: true,
            track: None,
            bake: None,
            capturing: Capturing::default(),
        }
    }
}

impl TakePanelState {
    /// Starts baking the take, unless a bake is running already or the take has no input track. `wake` asks the
    /// window to draw again. The report is an error when the bake could not even start.
    pub fn start_bake(&mut self, take: &Take, wake: impl Fn() + Send + 'static) -> Option<BakeReport> {
        let events = take.events.clone()?;
        if self.bake.is_some() {
            return None;
        }
        // The current style: v1 has no control for it yet, so it is the default.
        let request = BakeRequest { video: take.video.clone(), events, style: cursor_render::CursorStyle::default() };
        match BakeJob::start(take.name.clone(), request, wake) {
            Ok(job) => {
                self.bake = Some(job);
                None
            }
            Err(err) => Some(BakeReport::of(&take.name, Err(err.to_string()))),
        }
    }

    /// Takes in what the app captures now and cancels a running bake when a recording has just started or the replay
    /// buffer was just turned on. Returns whether it cancelled one.
    pub fn watch_capture(&mut self, now: Capturing) -> bool {
        let before = std::mem::replace(&mut self.capturing, now);
        self.bake.as_ref().is_some_and(|job| job.give_way(before, now))
    }

    /// Takes in the end of the running bake: the report to show, if it ended with one. A cancelled bake has none.
    pub fn poll_bake(&mut self) -> Option<BakeReport> {
        let outcome = self.bake.as_ref()?.poll()?;
        let job = self.bake.take()?;
        outcome.map(|outcome| BakeReport::of(job.take(), outcome))
    }

    /// Reads the take's input track unless it is the one already read.
    fn read_track(&mut self, take: &Take) {
        let Some(path) = &take.events else {
            self.track = None;
            return;
        };
        let size = take.files.iter().find(|file| &file.path == path).map_or(0, |file| file.size);
        if self.track.as_ref().is_some_and(|track| &track.path == path && track.size == size) {
            return;
        }
        let summary = review::load_track(path).map(|track| TrackSummary::of(&track));
        self.track = Some(TrackInfo { path: path.clone(), size, screen: bake::shows_screen(&take.video), summary });
    }

    /// Closes the rename field and clears its reason, after a rename or when the take goes away.
    pub fn close_rename(&mut self) {
        self.renaming = None;
        self.rename_focused = false;
        self.rename_error = None;
    }
}

/// The id of the Review stage.
const STAGE_ID: &str = "recorder-review-stage";

/// What a click on the Review bar asks the app to do beyond the playback.
#[derive(Debug, PartialEq, Eq)]
pub enum CenterAction {
    /// Nothing was clicked.
    None,
    /// Add a Note mark at this position (in milliseconds) to the take's sidecar.
    AddMark(u64),
}

/// The scrub bar's drag state: whether the pointer is down on the bar and
/// whether the take was playing before the drag paused it, so it resumes on
/// release.
#[derive(Debug, Default)]
pub struct ScrubState {
    /// Whether the pointer is pressed on the scrub bar.
    pub dragging: bool,
    /// Whether the take was playing when the drag started.
    pub was_playing: bool,
    /// Whether the drag has actually moved far enough to pause the take;
    /// a plain click never pauses it.
    pub paused_for_drag: bool,
}

/// The review center's frame-to-frame state, held by the app: the reused
/// texture the frames upload into and the scrub bar's drag state.
#[derive(Default)]
pub struct CenterState {
    /// The picture texture, reused for every frame of every take.
    texture: Option<TextureHandle>,
    /// The scrub bar's drag state.
    pub scrub: ScrubState,
    /// How many settle frames are left after a seek while paused.
    settle_frames: u8,
}

impl CenterState {
    /// Keeps the center repainting for a few frames so a paused picture
    /// settles after a seek (a scrub bar drag, a mark jump or a key).
    pub fn settle(&mut self) {
        self.settle_frames = SETTLE_FRAMES;
    }
}

/// The left Takes panel (spec "Review mode"): the header with the Takes |
/// Replays choice and the folder and refresh buttons, then the days with their
/// takes, newest first. `days` is the scan of `view`'s folder. Returns what the
/// header did.
pub fn takes_panel(ui: &mut Ui, view: &mut ReviewView, days: &[Day], selected: &mut Option<String>) -> TakesAction {
    let action = takes_header(ui, view);
    ui.add_space(space::SCALE_8);
    egui::ScrollArea::vertical().id_salt("review-takes").auto_shrink(false).show(ui, |ui| {
        ui.set_max_width(ui.available_width());
        let mut picked = None;
        for day in days {
            group_caption(ui, &day.date.format("%Y-%m-%d").to_string());
            for take in &day.takes {
                let is_selected = selected.as_deref() == Some(take.name.as_str());
                if take_row(ui, is_selected, take).clicked() {
                    picked = Some(take.name.clone());
                }
            }
            ui.add_space(space::SCALE_8);
        }
        if let Some(name) = picked {
            *selected = Some(name);
        }
    });
    action
}

/// The takes panel's header row (44 tall): the segmented Takes | Replays and
/// the open-folder and refresh icon buttons on the right, with the hairline
/// along the bottom of the row.
fn takes_header(ui: &mut Ui, view: &mut ReviewView) -> TakesAction {
    let mut action = TakesAction::None;
    let row = vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(HEADER_PAD);
        if layout_ui::choice(ui, "review-view", view, &VIEWS) {
            action = TakesAction::ViewChanged;
        }
        let folder_tooltip = match view {
            ReviewView::Takes => "Open project folder",
            ReviewView::Replays => "Open replays folder",
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Right to left, so the pair still reads "open folder, refresh" left to right.
            if ui_kit::icon_button(ui, ui_kit::icons::regular::ARROWS_CLOCKWISE, "Refresh takes").clicked() {
                action = TakesAction::Refresh;
            }
            if ui_kit::icon_button(ui, ui_kit::icons::regular::FOLDER_OPEN, folder_tooltip).clicked() {
                action = TakesAction::OpenFolder;
            }
        });
    });
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
    action
}

/// One take row, two lines in the list row: the name on top, cut short with an ellipsis when it
/// does not fit, and under it the take's details at full width against the right edge. The two do
/// not fit side by side at the narrowest panel width. A cut name shows in full on hover.
fn take_row(ui: &mut Ui, selected: bool, take: &Take) -> Response {
    // The ui_kit list row gives the fill, the click and the focus ring; the two lines are painted over it.
    let response = ui_kit::list_row(ui, selected, None, "", None);
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &take.name));
    if !ui.is_rect_visible(response.rect) {
        return response;
    }
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let painter = ui.painter();
    let inner = response.rect.shrink2(vec2(space::SCALE_4, 0.0));
    let name_color = if selected { neutrals.text_primary } else { neutrals.text_secondary };
    let body = ui_kit::fonts::medium(typography::BODY_SIZE);
    let width_of = |text: &str| painter.layout_no_wrap(text.to_owned(), body.clone(), name_color).size().x;
    let shown = fit_name(&take.name, inner.width(), width_of);
    let cut = shown != take.name;
    let name = painter.layout_no_wrap(shown, body.clone(), name_color);
    // The word after the mark count is not data; the time, the count and the length are.
    let count = take.marks.len().to_string();
    // A take with an input track has the cursor icon ahead of its details.
    let cursor = take.events.is_some().then(|| {
        let font = ui_kit::fonts::icon_regular(typography::BODY_SIZE);
        painter.layout_no_wrap(ui_kit::icons::regular::CURSOR.to_owned(), font, neutrals.text_tertiary)
    });
    let texts = take_row_details(take);
    let words = texts.iter().map(|text| {
        let data = if text.ends_with(" marks") { count.as_str() } else { text.as_str() };
        painter.layout_job(data_text::with_data(text, &[data], neutrals.text_tertiary))
    });
    let details: Vec<_> = cursor.into_iter().chain(words).collect();
    let details_height = details.iter().map(|galley| galley.size().y).fold(0.0, f32::max);
    let top = response.rect.center().y - (name.size().y + details_height) / 2.0;
    let details_top = top + name.size().y;
    painter.galley(pos2(inner.left(), top), name, name_color);
    let mut right = inner.right();
    for galley in details.into_iter().rev() {
        let left = right - galley.size().x;
        painter.galley(pos2(left, details_top), galley, neutrals.text_tertiary);
        right = left - space::SCALE_8;
    }
    if cut { response.on_hover_text(&take.name) } else { response }
}

/// The details of a take row: the start time, the mark count and the duration when the sidecar
/// knows it.
fn take_row_details(take: &Take) -> Vec<String> {
    let mut details = vec![take.start.format("%H:%M").to_string(), format!("{} marks", take.marks.len())];
    if let Some(duration) = take.duration_ms {
        details.push(review::format_duration(duration));
    }
    details
}

/// `name` cut to at most `max` points as `width_of` measures it. A name that ends in a take number
/// loses its middle, so the number stays readable; any other name loses its end. The end is also
/// what goes when there is no room for the take number.
fn fit_name(name: &str, max: f32, width_of: impl Fn(&str) -> f32) -> String {
    if width_of(name) <= max {
        return name.to_owned();
    }
    let tail = take_number_tail(name).unwrap_or("");
    let head = &name[..name.len() - tail.len()];
    // The longest start of `text` that fits with the ellipsis and `end` after it.
    let longest =
        |text: &str, end: &str| text.char_indices().rev().map(|(at, _)| format!("{}\u{2026}{end}", &text[..at])).find(|candidate| width_of(candidate) <= max);
    let middle = if tail.is_empty() { None } else { longest(head, tail) };
    middle.or_else(|| longest(name, "")).unwrap_or_else(|| "\u{2026}".to_owned())
}

/// The take number `name` ends in, "take" and its digits with the separator before them
/// ("_take01", " Take 3"); None when the name does not end that way.
fn take_number_tail(name: &str) -> Option<&str> {
    const SEPARATORS: [char; 3] = ['_', '-', ' '];
    let stem = name.trim_end_matches(|ch: char| ch.is_ascii_digit());
    if stem.len() == name.len() {
        return None;
    }
    let stem = stem.strip_suffix(SEPARATORS).unwrap_or(stem);
    let word_at = stem.len().checked_sub("take".len())?;
    if !stem.get(word_at..)?.eq_ignore_ascii_case("take") {
        return None;
    }
    let before = &stem[..word_at];
    let start = before.strip_suffix(SEPARATORS).map_or(word_at, str::len);
    Some(&name[start..])
}

/// A group caption ("2026-11-21", "File", "Marks") in the caption style.
fn group_caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).selectable(false));
}

/// The Review center's content (spec "Review mode"): the selected take
/// playing back. The frame is letterboxed in a 16:9 box on the stage's inner
/// color, with the scrub bar under it. `playback` is the app's open playback of
/// the selected take; with `None` the box shows the take's name until the
/// playback opens, or the reason `open_error` gives. While the take plays the
/// center asks for a repaint every frame, and for a few settle frames after
/// each seek so a paused picture arrives. The transport is [`stage`]'s.
pub fn center(ui: &mut Ui, days: &[Day], selected: Option<&str>, playback: Option<&mut Playback>, open_error: Option<&str>, state: &mut CenterState) {
    let available = ui.available_rect_before_wrap();
    // The scrub bar sits under the box, so the box fits the area above it.
    let scrub_top = available.bottom() - SCRUB_HEIGHT - SCRUB_GAP;
    let picture_area = Rect::from_min_max(available.min, pos2(available.max.x, scrub_top.max(available.top())));
    let stage_inner = Theme::get(ui.ctx()).neutrals.bg_stage_inner;
    ui.painter().rect_filled(picture_area, radius::GROUP, stage_inner);
    let Some(take) = review::find(days, selected) else {
        placeholder_caption(ui, picture_area.center(), "No take selected");
        return;
    };
    let box_rect = fit_16_9(picture_area);
    let scrub_rect =
        Rect::from_min_max(pos2(available.left(), box_rect.bottom() + SCRUB_GAP), pos2(available.right(), box_rect.bottom() + SCRUB_GAP + SCRUB_HEIGHT));
    let Some(playback) = playback else {
        let why = match open_error {
            Some(reason) => reason.to_owned(),
            None => "The take's picture opens here".to_owned(),
        };
        box_caption(ui, box_rect, &take.name, Some(&why));
        return;
    };
    let frame = playback.current_frame();
    draw_frame(ui, box_rect, frame, state);
    let duration = playback.duration_ms();
    let position = playback.position_ms();
    scrub_bar(ui, scrub_rect, playback, &take.marks, state, position, duration);
    // Repaint every frame only while playing; while paused a few settle
    // frames follow each seek so the decoder's picture shows up.
    if playback.is_playing() {
        ui.ctx().request_repaint();
    } else if state.settle_frames > 0 {
        state.settle_frames -= 1;
        ui.ctx().request_repaint_after(SETTLE_DELAY);
    }
}

/// What the Review bar shows of the open playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewTransport {
    /// The playback position in milliseconds.
    pub position: u64,
    /// The take's length in milliseconds.
    pub duration: u64,
    /// Whether the take is playing.
    pub playing: bool,
}

impl ReviewTransport {
    /// Reads the open playback of the selected take.
    pub fn of(playback: &Playback) -> Self {
        Self { position: playback.position_ms(), duration: playback.duration_ms(), playing: playback.is_playing() }
    }
}

/// What a click on the Review bar or its notch asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarAction {
    /// Play, or pause while playing.
    TogglePlay,
    /// Seek to the mark before the position.
    PreviousMark,
    /// Seek to the mark after the position.
    NextMark,
    /// Pause and go back to the start.
    Stop,
    /// Add a mark at the position.
    AddMark,
}

/// The Review stage: `add_content` above the bottom bar, and Play in the
/// notch, which shows Pause while the take plays. The bar holds the timecode
/// and Add mark on the left, and previous mark, next mark, Stop and the take's
/// total length on the right. `transport` is None when there is no playback
/// (no take selected, or it could not open); every control is disabled then.
/// Returns what was clicked, and the stage.
pub fn stage(ui: &mut Ui, transport: Option<ReviewTransport>, add_content: impl FnOnce(&mut Ui)) -> (Option<BarAction>, StageResponse) {
    let ready = transport.is_some();
    let playing = transport.is_some_and(|transport| transport.playing);
    let mut left = None;
    let mut right = None;
    let stage = ui_kit::stage_panel(
        ui,
        egui::Id::new(STAGE_ID),
        PrimaryButton { kind: PlayKind::Play, active: playing, enabled: ready },
        add_content,
        |ui| {
            let (main, fraction) = position_timecode(transport.map_or(0, |transport| transport.position));
            // Add mark stays after the timecode has gone. The Capture menu and the mark hotkey add one too.
            let companion = Companion { width: STAGE_BAR_BUTTON_DIAMETER, only_route: true };
            let add = bar_ui::left_side(ui, &main, &fraction, companion, |ui| bar_ui::bar_button(ui, ui_kit::icons::fill::FLAG, "Add mark", ready).clicked());
            left = (add == Some(true)).then_some(BarAction::AddMark);
        },
        |ui| {
            let button = |icon, tooltip: &str, action| BarButton { icon, tooltip: tooltip.to_owned(), enabled: ready, action };
            let buttons = [
                button(ui_kit::icons::fill::CARET_LEFT, "Previous mark ([)", BarAction::PreviousMark),
                button(ui_kit::icons::fill::CARET_RIGHT, "Next mark (])", BarAction::NextMark),
                button(ui_kit::icons::fill::STOP, "Stop", BarAction::Stop),
            ];
            let length = transport.map(|transport| length_readout(ui, transport.duration));
            right = bar_ui::right_side(ui, &buttons, length);
        },
    );
    let _ = stage.primary.clone().on_hover_text(if playing { "Pause (Space)" } else { "Play (Space)" });
    let play = stage.clicked().then_some(BarAction::TogglePlay);
    (play.or(left).or(right), stage)
}

/// The take's total length as the bar draws it.
pub(crate) fn length_readout(ui: &Ui, duration: u64) -> egui::text::LayoutJob {
    let tint = Theme::get(ui.ctx()).neutrals.text_tertiary;
    ui_kit::fonts::tracked(review::format_duration(duration), ui_kit::fonts::mono(typography::VALUE_SIZE), tint, ui_kit::fonts::MONO_LETTER_SPACING_EM)
}

/// Carries out a bar click on the open playback of a take with `marks`. Adding
/// a mark is reported back, and the app does the sidecar work.
pub fn run_bar_action(action: BarAction, playback: &mut Playback, marks: &[marks::Mark], state: &mut CenterState) -> CenterAction {
    let positions: Vec<u64> = marks.iter().map(|mark| mark.t_ms).collect();
    let position = playback.position_ms();
    let seek = match action {
        BarAction::TogglePlay => {
            playback.toggle();
            None
        }
        BarAction::PreviousMark => review::previous_mark(&positions, position),
        BarAction::NextMark => review::next_mark(&positions, position),
        BarAction::Stop => {
            playback.pause();
            Some(0)
        }
        BarAction::AddMark => return CenterAction::AddMark(position),
    };
    if let Some(ms) = seek {
        playback.seek(ms);
        state.settle();
    }
    CenterAction::None
}

/// Uploads the frame into the center's one reused texture and draws it
/// letterboxed into the box. A frame that has not
/// arrived (or one with a broken shape) leaves the last picture in place.
fn draw_frame(ui: &mut Ui, box_rect: Rect, frame: Option<RgbaFrame>, state: &mut CenterState) {
    let Some(frame) = frame else { return };
    let expected = frame.width as usize * frame.height as usize * 4;
    if frame.width == 0 || frame.height == 0 || frame.data.len() != expected {
        return;
    }
    let image = ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.data);
    let handle = state.texture.get_or_insert_with(|| ui.ctx().load_texture("review-frame", image.clone(), TextureOptions::LINEAR));
    handle.set(image, TextureOptions::LINEAR);
    let dest = fit_aspect(box_rect, frame.width, frame.height);
    ui.painter().image(handle.id(), dest, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
}

/// The scrub bar under the preview, the full width of the panel minus its
/// padding: the track in `bg.selected`, the progress up to the playhead in
/// the accent, the playhead in the ui_kit playhead style and one dot per mark
/// in its category color. Clicking or dragging seeks; a drag pauses the take
/// (when it was playing) and it resumes on release.
fn scrub_bar(ui: &mut Ui, scrub_rect: Rect, playback: &mut Playback, marks: &[marks::Mark], state: &mut CenterState, position: u64, duration: u64) {
    let track = Rect::from_center_size(scrub_rect.center(), vec2(scrub_rect.width(), SCRUB_TRACK_HEIGHT));
    let response = ui.allocate_rect(scrub_rect, Sense::click_and_drag());
    if response.drag_started() {
        state.scrub.dragging = true;
        state.scrub.was_playing = playback.is_playing();
        state.scrub.paused_for_drag = false;
    }
    if response.dragged() && state.scrub.dragging && !state.scrub.paused_for_drag {
        // The first real movement pauses for the scrub; a plain click never does.
        state.scrub.paused_for_drag = true;
        if state.scrub.was_playing {
            playback.pause();
        }
    }
    if ui.is_rect_visible(scrub_rect) {
        let theme = Theme::get(ui.ctx());
        let painter = ui.painter();
        painter.rect_filled(track, CornerRadius::from(SCRUB_TRACK_HEIGHT / 2.0), color::BG_SELECTED);
        let x = scrub_x(track, position, duration);
        if duration > 0 {
            let progress = Rect::from_min_max(pos2(track.left(), track.top()), pos2(x, track.bottom()));
            painter.rect_filled(progress, CornerRadius::from(SCRUB_TRACK_HEIGHT / 2.0), theme.accent.accent);
        }
        for mark in marks {
            paint_marker(painter, pos2(scrub_x(track, mark.t_ms, duration), track.center().y), mark_kind(mark.mark_type));
        }
        paint_playhead(painter, track, track, x, &theme);
    }
    // A click or a drag seeks to the pointer; the release ends the drag and
    // resumes the take when the drag had paused it.
    let mut target = None;
    if response.clicked() || response.dragged() {
        target = response.interact_pointer_pos().map(|pointer| review::scrub_ms(pointer.x, track.left(), track.width(), duration));
    }
    if response.drag_stopped() {
        state.scrub.dragging = false;
        if state.scrub.paused_for_drag && state.scrub.was_playing && !playback.is_playing() {
            playback.play();
        }
        state.scrub.paused_for_drag = false;
        target = response.interact_pointer_pos().map(|pointer| review::scrub_ms(pointer.x, track.left(), track.width(), duration));
    }
    if let Some(ms) = target {
        playback.seek(ms);
        state.settle();
    }
}

/// A centered caption pair inside the 16:9 box: the take's name, and the
/// reason the playback is not showing under it.
pub(crate) fn box_caption(ui: &mut Ui, box_rect: Rect, name: &str, why: Option<&str>) {
    let painter = ui.painter();
    let name_galley =
        painter.layout(name.to_owned(), ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_TERTIARY, box_rect.width() - 2.0 * space::SCALE_16);
    let offset = why.map_or(0.0, |_| name_galley.size().y / 2.0 + space::SCALE_4);
    painter.galley(box_rect.center() - name_galley.size() / 2.0 - vec2(0.0, offset), name_galley, color::TEXT_TERTIARY);
    if let Some(why) = why {
        let why_galley =
            painter.layout(why.to_owned(), ui_kit::fonts::medium(typography::CAPTION_SIZE), color::TEXT_TERTIARY, box_rect.width() - 2.0 * space::SCALE_16);
        painter.galley(box_rect.center() - why_galley.size() / 2.0 + vec2(0.0, offset), why_galley, color::TEXT_TERTIARY);
    }
}

/// A centered tertiary caption where the picture would be.
pub(crate) fn placeholder_caption(ui: &Ui, at: egui::Pos2, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), ui_kit::fonts::medium(typography::CAPTION_SIZE), color::TEXT_TERTIARY);
    ui.painter().galley(at - galley.size() / 2.0, galley, color::TEXT_TERTIARY);
}

/// The largest 16:9 box that fits in `available`, centered in it.
fn fit_16_9(available: Rect) -> Rect {
    fit_aspect(available, 16, 9)
}

/// The largest box of `width` : `height` aspect that fits in `available`,
/// centered in it.
pub(crate) fn fit_aspect(available: Rect, width: u32, height: u32) -> Rect {
    if width == 0 || height == 0 {
        return Rect::NOTHING;
    }
    let aspect = width as f32 / height as f32;
    let side = available.width().min(available.height() * aspect).max(0.0);
    Rect::from_center_size(available.center(), vec2(side, side / aspect))
}

/// The x of a timeline position on the scrub track: clamped to the track,
/// its left edge without a duration.
fn scrub_x(track: Rect, t_ms: u64, duration: u64) -> f32 {
    if duration == 0 {
        return track.left();
    }
    let fraction = (t_ms as f32 / duration as f32).min(1.0);
    track.left() + fraction * track.width()
}

/// The bar timecode of `ms`: `HH:MM:SS` and the `.mmm` fraction.
pub(crate) fn position_timecode(ms: u64) -> (String, String) {
    let seconds = ms / 1000;
    (format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60), format!(".{:03}", ms % 1000))
}

/// The right take info panel (spec "Review mode"): the header with the take's
/// name and date, then the File group, the sidecar's marks and the Actions
/// group at the bottom. Returns what the Actions group or the rename field
/// did; `recording` disables Rename and Delete while a take runs.
/// `playback_info` is the running playback's `(position_ms, is_playing)`;
/// clicking a mark seeks to it (a `TakeAction::Seek`) and, while playing, the
/// mark under the playhead is selected.
pub fn take_panel(
    ui: &mut Ui,
    days: &[Day],
    selected: Option<&str>,
    state: &mut TakePanelState,
    recording: bool,
    playback_info: Option<(u64, bool)>,
) -> TakeAction {
    let take = review::find(days, selected);
    let mut action = take_header(ui, take, state).map_or(TakeAction::None, TakeAction::Rename);
    ui.add_space(space::SCALE_8);
    let Some(take) = take else { return action };
    state.read_track(take);
    egui::ScrollArea::vertical().id_salt("review-info").auto_shrink(false).show(ui, |ui| {
        ui.set_max_width(ui.available_width());
        group_caption(ui, "File");
        inspector_group(ui, |ui| {
            let duration = take.duration_ms.map(review::format_duration).unwrap_or_else(|| "\u{2014}".to_owned());
            row(ui, "Duration", &duration);
            // The hover names every file of the take with its own size.
            row(ui, "Size", &review::format_size(take.total_size)).on_hover_ui(|ui| file_list(ui, take));
            row(ui, "Video format", take.video_format.as_deref().unwrap_or("\u{2014}"));
            if take.iso.is_some() {
                row(ui, "Camera ISO format", take.iso_format.as_deref().unwrap_or("\u{2014}"));
            }
            let composite = match take.composite {
                CompositeMode::Baked => "Baked",
                CompositeMode::Clean => "Clean",
            };
            row(ui, "Composite", composite);
            if let Some(track) = &state.track {
                track_rows(ui, track, &mut state.show_cursor, &mut action);
            }
        });
        ui.add_space(space::SCALE_8);
        group_caption(ui, "Marks");
        if take.marks.is_empty() {
            placeholder_caption(ui, ui.cursor().left_top() + egui::vec2(ui.available_width() / 2.0, 10.0), "No marks");
            ui.add_space(20.0);
        }
        // Clicking a mark seeks to it, and while playing the mark under the
        // playhead is the selected row (spec "Review mode").
        let positions: Vec<u64> = take.marks.iter().map(|mark| mark.t_ms).collect();
        let under_playhead = playback_info.and_then(|(position, playing)| if playing { review::mark_under(&positions, position) } else { None });
        for (index, mark) in take.marks.iter().enumerate() {
            let label = mark.label.clone().unwrap_or_else(|| type_name(mark.mark_type, mark.original_type.as_deref()));
            let trailing = marks::format_tc(mark.t_ms);
            let response = ui_kit::list_row(ui, under_playhead == Some(index), Some(mark_dot(mark.mark_type)), &label, Some(&trailing));
            if response.clicked() {
                action = TakeAction::Seek(mark.t_ms);
            }
        }
        ui.add_space(space::SCALE_8);
        group_caption(ui, "Actions");
        actions_group(ui, take, state, recording, &mut action);
        ui.add_space(space::SCALE_8);
    });
    action
}

/// The File group's rows about the input track: the Show cursor switch and the counts, with a warning when the
/// track is incomplete.
fn track_rows(ui: &mut Ui, track: &TrackInfo, show_cursor: &mut bool, action: &mut TakeAction) {
    let Some(summary) = track.summary else {
        row(ui, "Input track", "Could not be read");
        return;
    };
    if track.screen {
        inspector_row(
            ui,
            "Show cursor",
            |ui| {
                if ui_kit::switch(ui, show_cursor).changed() {
                    *action = TakeAction::ShowCursor(*show_cursor);
                }
            },
            None,
        );
    }
    for (label, count) in [("Clicks", summary.clicks), ("Shortcuts", summary.shortcuts), ("App switches", summary.app_switches)] {
        row(ui, label, &count.map_or_else(|| "Not recorded".to_owned(), |count| count.to_string()));
    }
    if !summary.complete {
        ui.colored_label(ui.visuals().warn_fg_color, "Input track incomplete");
    }
}

/// The Actions group at the bottom of the take panel (spec "Review mode"):
/// Show in folder, Rename and Delete take as text buttons; Rename and Delete
/// are disabled while a take runs, and for the take being baked. Under them
/// Open in Studio, for a take with an input track, and the running bake's
/// progress with Cancel.
fn actions_group(ui: &mut Ui, take: &Take, state: &mut TakePanelState, recording: bool, action: &mut TakeAction) {
    let baking_this = state.bake.as_ref().is_some_and(|job| job.take() == take.name);
    inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui.horizontal(|ui| {
            if ui.button("Show in folder").clicked() {
                *action = TakeAction::ShowInFolder;
            }
            // A bake reads the take's files, so they stay where they are until it ends.
            ui.add_enabled_ui(!recording && !baking_this, |ui| {
                if ui.button("Rename").clicked() {
                    state.renaming = Some(take.name.clone());
                    state.rename_focused = false;
                    state.rename_error = None;
                }
                let delete = Button::new(RichText::new("Delete take").color(color::SEM_RECORD));
                if ui.add(delete).clicked() {
                    state.confirm_delete = true;
                }
            });
        });
        if take.events.is_some() && !baking_this {
            let button = ui.add_enabled(!recording, Button::new("Open in Studio"));
            if button.on_disabled_hover_text("Studio opens after the recording stops").clicked() {
                *action = TakeAction::OpenInStudio;
            }
        }
        if let Some(job) = &state.bake {
            if !baking_this {
                ui.add(egui::Label::new(format!("Baking {}", job.take())).truncate());
            }
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        job.cancel();
                    }
                    ui.add(egui::ProgressBar::new(job.progress()).show_percentage());
                });
            });
        }
        ui.add_space(space::SCALE_4);
    });
}

/// The take info header row: the take's name over its start date, with the
/// hairline along the bottom of the row. While the name is being edited the
/// name is an inline text field instead: Enter confirms (the new name is
/// returned), Esc cancels, and a refused name shows its reason under the field.
fn take_header(ui: &mut Ui, take: Option<&Take>, state: &mut TakePanelState) -> Option<String> {
    let mut submitted = None;
    let mut cancelled = false;
    let renaming = state.renaming.is_some() && take.is_some();
    // The field grows past the 44px row, so the fixed height is only reserved
    // when the plain name is shown.
    let height = if renaming { 0.0 } else { size::PANEL_TOOLBAR_HEIGHT };
    let row = vec2(ui.available_width(), height);
    let inner = ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(HEADER_PAD);
        ui.vertical(|ui| {
            match take {
                Some(take) => match &mut state.renaming {
                    Some(buffer) => {
                        let edit = ui.add(egui::TextEdit::singleline(buffer).desired_width(ui.available_width()));
                        if !state.rename_focused {
                            edit.request_focus();
                            state.rename_focused = true;
                        }
                        if edit.lost_focus() {
                            let (enter, escape) = ui.input(|input| (input.key_pressed(egui::Key::Enter), input.key_pressed(egui::Key::Escape)));
                            if escape {
                                cancelled = true;
                            } else if enter {
                                submitted = Some(buffer.clone());
                            }
                        }
                    }
                    None => {
                        ui.add(
                            egui::Label::new(RichText::new(take.name.clone()).font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY))
                                .selectable(false)
                                .truncate(),
                        );
                    }
                },
                None => {
                    ui.add(
                        egui::Label::new(RichText::new("No take selected").font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY))
                            .selectable(false),
                    );
                }
            }
            if !renaming && let Some(take) = take {
                ui.add(
                    egui::Label::new(
                        RichText::new(take.start.format("%b %d, %Y").to_string())
                            .font(ui_kit::fonts::medium(typography::CAPTION_SIZE))
                            .color(color::TEXT_TERTIARY),
                    )
                    .selectable(false),
                );
            }
            if let Some(reason) = &state.rename_error {
                ui.colored_label(ui.visuals().error_fg_color, reason);
            }
        });
    });
    if cancelled {
        state.close_rename();
    }
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
    submitted
}

/// Every file of `take` on its own line with its size.
fn file_list(ui: &mut Ui, take: &Take) {
    let tint = Theme::get(ui.ctx()).neutrals.text_primary;
    for file in &take.files {
        let name = file.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let size = review::format_size(file.size);
        ui.add(egui::Label::new(data_text::with_data(&format!("{name} ({size})"), &[&size], tint)).wrap());
    }
}

/// One read-only inspector row with its value.
fn row(ui: &mut Ui, label: &str, value: &str) -> egui::Response {
    inspector_row(ui, label, |_ui| {}, Some(value))
}

/// The dot color of a sidecar mark: the marker palette has no Highlight or
/// Chapter tokens, so they take the Retake and Note colors (the same mapping
/// the live marks list uses).
fn mark_dot(mark_type: MarkType) -> Color32 {
    marker_color(mark_kind(mark_type))
}

/// The marker kind of a sidecar mark: the marker palette has no Highlight or
/// Chapter tokens, so they take the Retake and Note colors (the same mapping
/// the live marks list uses).
fn mark_kind(mark_type: MarkType) -> MarkerKind {
    match mark_type {
        MarkType::Cut => MarkerKind::Cut,
        MarkType::Retake | MarkType::Highlight => MarkerKind::Retake,
        MarkType::Note | MarkType::Chapter => MarkerKind::Note,
        MarkType::BRoll => MarkerKind::BRoll,
        MarkType::System => MarkerKind::System,
    }
}

/// The display name of a mark type; an unknown type keeps the original string
/// the reader saved.
fn type_name(mark_type: MarkType, original: Option<&str>) -> String {
    match original {
        Some(original) => original.to_owned(),
        None => match mark_type {
            MarkType::Highlight => "Highlight".to_owned(),
            MarkType::Cut => "Cut".to_owned(),
            MarkType::Retake => "Retake".to_owned(),
            MarkType::Chapter => "Chapter".to_owned(),
            MarkType::Note => "Note".to_owned(),
            MarkType::BRoll => "B-roll".to_owned(),
            MarkType::System => "System".to_owned(),
        },
    }
}

/// The delete confirmation dialog (spec "Review mode"): every file of the take
/// with its size, Cancel and "Move to Recycle Bin". Returns whether the move
/// was confirmed this frame; the app does the file work.
pub fn delete_dialog(ctx: &egui::Context, take: Option<&Take>, open: &mut bool) -> bool {
    let mut confirmed = false;
    if !*open {
        return false;
    }
    let Some(take) = take else {
        *open = false;
        return false;
    };
    // The buttons report through cells; the dialog itself closes `open` on Esc
    // and a backdrop click, so the closures must not capture it.
    let closed = std::cell::Cell::new(false);
    ui_kit::dialog(
        ctx,
        egui::Id::new("review-delete-take"),
        "Delete take",
        &mut *open,
        |ui| {
            ui.label(format!("Move the files of \u{201c}{}\u{201d} to the Recycle Bin?", take.name));
            ui.add_space(space::SCALE_8);
            egui::ScrollArea::vertical().id_salt("review-delete-files").max_height(220.0).auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                file_list(ui, take);
            });
        },
        |ui| {
            if ui.button("Cancel").clicked() {
                closed.set(true);
            }
            if ui_kit::primary_button(ui, "Move to Recycle Bin").clicked() {
                closed.set(true);
                confirmed = true;
            }
        },
    );
    if closed.get() {
        *open = false;
    }
    confirmed
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use chrono::Local;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;
    use crate::bar_ui::bar_checks::{self, assert_bar_is_clean, stage_harness, stage_width};
    use crate::config::{self, Mode};
    use crate::tools_ui::row_checks::{assert_no_overlap, painted, panel_harness};

    /// A take with the given name, number of marks and length.
    fn row_take(name: &str, marks: usize, duration_ms: Option<u64>) -> Take {
        let mark: marks::Mark = serde_json::from_value(serde_json::json!({
            "id": "0196f3a2-9d44-7e02-8c15-6a0b3e7d2f90", "type": "note", "media": 0, "t_ms": 0,
            "source": "hotkey", "created_at": "2026-11-20T12:00:00.000Z"
        }))
        .unwrap();
        Take {
            name: name.to_owned(),
            start: Local::now(),
            duration_ms,
            total_size: 0,
            files: Vec::new(),
            video: PathBuf::new(),
            iso: None,
            events: None,
            video_format: None,
            iso_format: None,
            marks: vec![mark; marks],
            composite: CompositeMode::Baked,
        }
    }

    /// At the narrowest left panel the takes rows paint no text over another, show the time, the
    /// mark count and the length whole, and cut only a name that does not fit.
    #[test]
    fn take_rows_do_not_overlap_at_the_narrowest_panel() {
        let days = vec![Day {
            date: Local::now().date_naive(),
            takes: vec![
                Take { events: Some(PathBuf::from("take01.events")), ..row_take("2026-11-20_10-00-00_take01", 12, Some(3_698_784)) },
                row_take("Interview with the whole product team, second try take 12", 123, Some(359_999_000)),
                row_take("A renamed take with a long name and no number at the end", 0, None),
            ],
        }];
        let harness = panel_harness(config::LEFT_PANEL_MIN, move |ui| {
            takes_panel(ui, &mut ReviewView::Takes, &days, &mut None);
        });
        let texts = painted(&harness);
        let all: Vec<_> = texts.iter().collect();
        assert_no_overlap(&all, &[], config::LEFT_PANEL_MIN);
        let shown = |wanted: &str| texts.iter().any(|painted| painted.text == wanted);
        for whole in ["2026-11-20_10-00-00_take01", "12 marks", "123 marks", "0 marks", "01:01:38", "99:59:59"] {
            assert!(shown(whole), "{whole} is shown whole");
        }
        let icons = texts.iter().filter(|painted| painted.text == ui_kit::icons::regular::CURSOR).count();
        assert_eq!(icons, 1, "only the take with an input track has the cursor icon");
        assert!(texts.iter().any(|painted| painted.text.starts_with("Interview") && painted.text.ends_with("\u{2026} take 12")));
        assert!(texts.iter().any(|painted| painted.text.starts_with("A renamed") && painted.text.ends_with('\u{2026}')));
    }

    /// An events file with two clicks and no keyboard stream; without its tail when it is to be incomplete.
    fn events_file(path: &std::path::Path, complete: bool) -> Result<(), events::EventsError> {
        use events::{CaptureSource, EventsWriter, Header, InputEvent, MouseAction, MouseButton, MouseEvent, ScreenRect, Streams};

        let header = Header {
            qpc_frequency: 1_000_000,
            first_frame_qpc: 0,
            source: CaptureSource::Monitor(ScreenRect::default()),
            displays: Vec::new(),
            cursor_size: 32,
            streams: Streams { cursor: true, clicks: true, keys: false, apps: true },
        };
        let mut writer = EventsWriter::create(path, &header)?;
        for qpc in [10, 20] {
            writer.push(InputEvent::Mouse(MouseEvent { qpc, x: 0, y: 0, action: MouseAction::Down(MouseButton::Left) }))?;
        }
        if complete {
            writer.finish(1_000)?;
        } else {
            writer.flush_second()?;
        }
        Ok(())
    }

    /// The take panel of a take with an input track has the switch, the counts and Open in Studio, which recording
    /// disables; a track without its tail says it is incomplete, and a take without a track has none of it.
    #[test]
    fn the_take_panel_shows_the_input_track_and_offers_studio() -> Result<(), Box<dyn std::error::Error>> {
        let folder = std::env::temp_dir().join(format!("cutmark-recorder-test-{}-panel-track", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder)?;
        for (name, complete, recording) in [("whole", true, false), ("cut", false, true)] {
            let events = folder.join(format!("{name}.events"));
            events_file(&events, complete)?;
            let take = Take { events: Some(events), video: folder.join(format!("{name}.mkv")), ..row_take(name, 0, None) };
            let days = vec![Day { date: Local::now().date_naive(), takes: vec![take] }];
            let asked = Rc::new(RefCell::new(Vec::new()));
            let record = Rc::clone(&asked);
            let mut state = TakePanelState::default();
            assert!(state.show_cursor, "the cursor shows until the switch is turned off");
            let mut harness = panel_harness(300.0, move |ui| {
                let action = take_panel(ui, &days, Some(name), &mut state, recording, None);
                if action != TakeAction::None {
                    record.borrow_mut().push(action);
                }
            });
            let texts: Vec<String> = painted(&harness).into_iter().map(|painted| painted.text).collect();
            for wanted in ["Show cursor", "Clicks", "2", "Shortcuts", "Not recorded", "App switches", "0"] {
                assert!(texts.iter().any(|text| text == wanted), "{name}: {wanted} is shown in {texts:?}");
            }
            assert_eq!(texts.iter().any(|text| text == "Input track incomplete"), !complete, "{name}");
            assert_eq!(harness.get_by_label("Open in Studio").accesskit_node().is_disabled(), recording, "{name}");
            assert!(harness.query_by_label("Bake with cursor").is_none(), "{name}");
            harness.get_by_label("Switch").click();
            harness.run();
            harness.get_by_label("Open in Studio").click();
            harness.run();
            let expected: &[TakeAction] = if recording { &[TakeAction::ShowCursor(false)] } else { &[TakeAction::ShowCursor(false), TakeAction::OpenInStudio] };
            assert_eq!(*asked.borrow(), expected, "{name}");
        }

        let days = vec![Day { date: Local::now().date_naive(), takes: vec![row_take("plain", 0, None)] }];
        let mut state = TakePanelState::default();
        let harness = panel_harness(300.0, move |ui| {
            take_panel(ui, &days, Some("plain"), &mut state, false, None);
        });
        assert!(harness.query_by_label("Open in Studio").is_none() && harness.query_by_label("Switch").is_none());
        std::fs::remove_dir_all(&folder)?;
        Ok(())
    }

    /// A long name loses its middle when it ends in a take number and its end otherwise. Widths
    /// here are one point per character.
    #[test]
    fn long_take_names_are_cut_with_an_ellipsis() {
        let fit = |name: &str, max: f32| fit_name(name, max, |text| text.chars().count() as f32);
        assert_eq!(fit("short_take01", 20.0), "short_take01");
        assert_eq!(fit("2026-11-20_10-00-00_take01", 16.0), "2026-11-\u{2026}_take01");
        assert_eq!(fit("Interview Take 3", 12.0), "Inte\u{2026} Take 3");
        assert_eq!(fit("intro_take02b_final", 10.0), "intro_tak\u{2026}");
        assert_eq!(fit("chapter 2026", 8.0), "chapter\u{2026}");
        assert_eq!(fit("interview_take01", 6.0), "inter\u{2026}", "no room for the take number");
        assert_eq!(fit("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}", 3.0), "\u{e9}\u{e9}\u{2026}");
        assert_eq!(take_number_tail("take7"), Some("take7"));
        assert_eq!(take_number_tail("mistake-take_7"), Some("-take_7"));
        assert_eq!(take_number_tail("take"), None);
        assert_eq!(take_number_tail("12"), None);
    }

    /// The bar timecode splits the position into `HH:MM:SS` and the `.mmm` fraction.
    #[test]
    fn the_bar_timecode_splits_time_and_fraction() {
        assert_eq!(position_timecode(0), ("00:00:00".to_owned(), ".000".to_owned()));
        assert_eq!(position_timecode(3_698_784), ("01:01:38".to_owned(), ".784".to_owned()));
    }

    /// A Review stage `width` wide that records its bar clicks.
    fn review_harness(width: f32, transport: Option<ReviewTransport>) -> (egui_kittest::Harness<'static>, bar_checks::Drawn, Rc<RefCell<Vec<BarAction>>>) {
        let clicked = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&clicked);
        let (harness, drawn) = stage_harness(width, move |ui| {
            let (action, stage) = stage(ui, transport, |_| {});
            record.borrow_mut().extend(action);
            stage
        });
        (harness, drawn, clicked)
    }

    /// With no take there is no playback, and Play and the whole bar are disabled.
    #[test]
    fn review_play_is_disabled_with_no_take() {
        let (mut harness, _, clicked) = review_harness(700.0, None);
        for label in ["Play", "Add mark", "Previous mark ([)", "Next mark (])", "Stop"] {
            assert!(harness.get_by_label(label).accesskit_node().is_disabled(), "{label} is disabled");
            harness.get_by_label(label).click();
            harness.run();
        }
        assert!(clicked.borrow().is_empty(), "disabled controls report nothing: {:?}", clicked.borrow());
    }

    /// With a take, the notch and the bar report what was clicked, and the notch reads Pause
    /// while the take plays.
    #[test]
    fn the_review_controls_report_their_clicks() {
        let transport = ReviewTransport { position: 1_500, duration: 61_000, playing: false };
        let (mut harness, _, clicked) = review_harness(700.0, Some(transport));
        for label in ["Play", "Add mark", "Previous mark ([)", "Next mark (])", "Stop"] {
            harness.get_by_label(label).click();
            harness.run();
        }
        assert_eq!(*clicked.borrow(), [BarAction::TogglePlay, BarAction::AddMark, BarAction::PreviousMark, BarAction::NextMark, BarAction::Stop]);
        let (harness, _, _) = review_harness(700.0, Some(ReviewTransport { playing: true, ..transport }));
        assert!(harness.query_by_label("Pause").is_some() && harness.query_by_label("Play").is_none());
    }

    /// Play in the notch toggles a real playback, and Stop pauses it at the start. Skipped
    /// where the test clip cannot be encoded.
    #[test]
    fn review_play_toggles_the_playback() -> Result<(), Box<dyn std::error::Error>> {
        let take = match crate::playback::tests::test_take() {
            Ok(take) => take,
            Err(reason) => {
                eprintln!("skipped: {reason}");
                return Ok(());
            }
        };
        let playback = Rc::new(RefCell::new(Playback::open(take)?));
        let shown = Rc::clone(&playback);
        let (mut harness, _) = stage_harness(700.0, move |ui| {
            let transport = ReviewTransport::of(&shown.borrow());
            let (action, stage) = stage(ui, Some(transport), |_| {});
            if let Some(action) = action {
                run_bar_action(action, &mut shown.borrow_mut(), &[], &mut CenterState::default());
            }
            stage
        });
        harness.get_by_label("Play").click();
        harness.run_steps(3);
        assert!(playback.borrow().is_playing(), "Play starts the take");
        harness.get_by_label("Pause").click();
        harness.run_steps(3);
        assert!(!playback.borrow().is_playing(), "Pause stops the clock");
        playback.borrow_mut().seek(200);
        harness.run_steps(3);
        harness.get_by_label("Stop").click();
        harness.run_steps(3);
        assert!(!playback.borrow().is_playing());
        assert_eq!(playback.borrow().position_ms(), 0, "Stop goes back to the start");
        Ok(())
    }

    /// The default window shows the whole Review bar, and the narrowest one still shows
    /// every action, with the panels as wide as they go too.
    #[test]
    fn the_review_bar_keeps_every_action_in_the_narrowest_window() {
        let transport = Some(ReviewTransport { position: 0, duration: 61_000, playing: false });
        let actions = ["Play", "Add mark", "Previous mark ([)", "Next mark (])", "Stop"];
        let (harness, _, _) = review_harness(stage_width(Mode::Review, 1280.0), transport);
        for label in actions.into_iter().chain(["00:00:00.000", "00:01:01"]) {
            assert!(harness.query_by_label(label).is_some(), "{label} is shown");
        }
        for width in bar_checks::narrowest_stage_widths(Mode::Review) {
            let (harness, drawn, _) = review_harness(width, transport);
            for label in actions {
                assert!(harness.query_by_label(label).is_some(), "{label} is shown at {width}");
            }
            let stage = drawn.borrow().clone().expect("the stage was drawn");
            assert_bar_is_clean(&harness, &stage, width);
        }
    }

    /// From the narrowest window up, the Review bar's center column stays empty and nothing in
    /// the bar is cut off.
    #[test]
    fn the_review_bar_center_column_stays_free_at_minimum_width() {
        let transport = ReviewTransport { position: 3_698_784, duration: 359_999_000, playing: false };
        let mut widths = vec![stage_width(Mode::Review, config::MIN_WINDOW_WIDTH)];
        widths.extend((100..=900).step_by(20).map(|width| width as f32));
        for width in widths {
            for transport in [None, Some(transport)] {
                let (harness, drawn, _) = review_harness(width, transport);
                let stage = drawn.borrow().clone().expect("the stage was drawn");
                assert_bar_is_clean(&harness, &stage, width);
            }
        }
    }

    /// The center draws headlessly without a take, and with a take whose
    /// playback has not opened (the reason shows), without panicking.
    #[test]
    fn the_center_draws_without_a_take_and_without_a_playback() {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        let days = vec![Day {
            date: Local::now().date_naive(),
            takes: vec![Take {
                name: "take01".to_owned(),
                start: Local::now(),
                duration_ms: Some(60_000),
                total_size: 0,
                files: Vec::new(),
                video: PathBuf::new(),
                iso: None,
                events: None,
                video_format: None,
                iso_format: None,
                marks: Vec::new(),
                composite: CompositeMode::Baked,
            }],
        }];
        let mut state = CenterState::default();
        for selected in [None, Some("take01")] {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0))), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                center(ui, &days, selected, None, Some("the file could not be probed"), &mut state);
            });
            output.textures_delta.clear();
        }
    }
}
