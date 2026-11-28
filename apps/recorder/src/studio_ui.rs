//! The Studio mode panels (Studio spec "Layout"): the takes that have an input track on the left
//! (Review's list), the live preview and the deck in the center and the tool strip on the right.
//!
//! The preview is drawn by studio-render on the window's own GPU device into a texture egui shows,
//! at the size the picture has on screen. A [`Playback`] supplies the source frames, without its
//! own cursor.
//!
//! The panels' controls are in `studio_panel_ui`. A change there shows in the preview on the next
//! frame and is saved to the take's Studio file half a second after the last one.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui_wgpu::{RenderState, wgpu};
use egui::{Color32, Pos2, Rect, Sense, Ui, pos2};
use media_io::player::probe::probe;
use media_io::player::video::RgbaFrame;
use studio_render::{Background, RenderError, RenderSettings, StudioFrameInput, StudioRenderer};
use ui_kit::tokens::{radius, space};
use ui_kit::{PlayKind, PrimaryButton, STAGE_BAR_BUTTON_DIAMETER, StageResponse, Theme};

use crate::bar_ui::{self, BarButton, Companion};
use crate::layout_ui;
use crate::playback::Playback;
use crate::review::{self, Day, ReviewView, Take, bake};
use crate::review_ui;
use crate::studio::background;
use crate::studio::settings::{BackgroundKind, FrameSettings};
use crate::studio::take::{StudioTake, VideoFacts, render_settings};
use crate::studio_panel_ui::{self, PanelFacts};

/// Frames are decoded at most this wide for the preview: enough to stay sharp when zoomed in.
const SOURCE_MAX_WIDTH: u32 = 1920;

/// How many frames the center keeps repainting after a take opens or a seek while paused, so the
/// decoded picture arrives.
const SETTLE_FRAMES: u8 = 40;

/// How long each settle frame waits.
const SETTLE_DELAY: Duration = Duration::from_millis(50);

/// The id of the Studio stage.
const STAGE_ID: &str = "recorder-studio-stage";

/// The format egui asks of a texture it did not make.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The inspector panels of the tool strip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StudioTool {
    #[default]
    Cursor,
    Zoom,
    Keys,
    Frame,
    Export,
}

/// The tool strip, in its order.
const TOOLS: [(StudioTool, &str); 5] =
    [(StudioTool::Cursor, "Cursor"), (StudioTool::Zoom, "Zoom"), (StudioTool::Keys, "Keys"), (StudioTool::Frame, "Frame"), (StudioTool::Export, "Export")];

impl StudioTool {
    /// What the panel says while its controls are yet to be built.
    fn placeholder(self) -> Option<&'static str> {
        match self {
            Self::Cursor | Self::Keys | Self::Frame => None,
            Self::Zoom => Some("Coming in T5"),
            Self::Export => Some("Coming in T6"),
        }
    }
}

/// The takes Studio lists: those with an input track, in the days that have one.
pub fn track_takes(mut days: Vec<Day>) -> Vec<Day> {
    for day in &mut days {
        day.takes.retain(|take| take.events.is_some());
    }
    days.retain(|day| !day.takes.is_empty());
    days
}

/// The texture the preview is drawn into and the renderer that draws it, for one take at one size.
struct Preview {
    gpu: RenderState,
    renderer: StudioRenderer,
    view: wgpu::TextureView,
    id: egui::TextureId,
    settings: RenderSettings,
    size: (u32, u32),
    /// The time of the frame in the texture; None while nothing has been drawn.
    drawn: Option<u64>,
}

impl Preview {
    fn new(gpu: &RenderState, size: (u32, u32), take: &StudioTake) -> Result<Self, RenderError> {
        let mut renderer = StudioRenderer::new(gpu.device.clone(), gpu.queue.clone(), size, TARGET_FORMAT)?;
        renderer.set_cursor_shapes(take.track().shapes())?;
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("studio preview"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let id = gpu.renderer.write().register_native_texture(&gpu.device, &view, wgpu::FilterMode::Linear);
        Ok(Self { gpu: gpu.clone(), renderer, view, id, settings: render_settings(&take.settings, None), size, drawn: None })
    }

    /// Renders the frame, unless the texture holds it already.
    fn draw(&mut self, take: &StudioTake, frame: &RgbaFrame) -> Result<(), RenderError> {
        if self.drawn == Some(frame.pts_ms) {
            return Ok(());
        }
        let source = self.renderer.upload_rgba(frame)?;
        let t_us = take.track_time_us(frame.pts_ms);
        let values = take.frame_values(t_us, (frame.width, frame.height));
        let input =
            StudioFrameInput { t_us, view_rect: values.view, cursor: values.cursor, clicks: &values.clicks, chord: values.chord, settings: &self.settings };
        self.renderer.render(&source, &input, &self.view)?;
        self.drawn = Some(frame.pts_ms);
        Ok(())
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.gpu.renderer.write().free_texture(&self.id);
    }
}

/// What the Frame panel says about a background picture that cannot be shown.
const NO_IMAGE_NOTE: &str = "No image is picked. The gradient is shown instead.";
const MISSING_IMAGE_NOTE: &str = "The image is missing. The gradient is shown instead.";
const BAD_IMAGE_NOTE: &str = "The image could not be opened. The gradient is shown instead.";

/// A background picture on its way to the renderer.
enum Picture {
    Decoded(RgbaFrame),
    /// On the window's GPU device, so it outlives the renderer.
    Uploaded(wgpu::Texture),
    /// The note that says why there is none.
    Failed(&'static str),
}

/// The background picture of the open take, read once for each path.
#[derive(Default)]
struct BackdropImage {
    loaded: Option<(PathBuf, Picture)>,
}

impl BackdropImage {
    /// Reads the picture the frame asks for, unless it is held already. A frame that shows no
    /// picture keeps the last one.
    fn sync(&mut self, frame: &FrameSettings) {
        let Some(path) = wanted_image(frame) else { return };
        if self.loaded.as_ref().is_some_and(|(held, _)| held == path) {
            return;
        }
        let picture = match background::load(path) {
            Ok(picture) => Picture::Decoded(picture),
            Err(err) => {
                log::warn!("the Studio background {} could not be loaded: {err}", path.display());
                Picture::Failed(if matches!(err, background::BackgroundError::Missing) { MISSING_IMAGE_NOTE } else { BAD_IMAGE_NOTE })
            }
        };
        self.loaded = Some((path.to_owned(), picture));
    }

    /// Forgets the picture, so that the next [`Self::sync`] reads the file again.
    fn forget(&mut self) {
        self.loaded = None;
    }

    /// What the Frame panel says under the Image row.
    fn note(&self, frame: &FrameSettings) -> Option<&'static str> {
        if frame.background != BackgroundKind::Image {
            return None;
        }
        let Some(path) = &frame.image else { return Some(NO_IMAGE_NOTE) };
        match &self.loaded {
            Some((held, Picture::Failed(note))) if held == path => Some(note),
            _ => None,
        }
    }

    /// The picture as the renderer takes it, put on the GPU the first time. None leaves the
    /// gradient in its place.
    fn texture(&mut self, frame: &FrameSettings, renderer: &StudioRenderer) -> Option<Background> {
        let path = wanted_image(frame)?;
        let (held, picture) = self.loaded.as_mut()?;
        if held != path {
            return None;
        }
        if let Picture::Decoded(decoded) = picture {
            *picture = match renderer.upload_rgba(decoded) {
                Ok(texture) => Picture::Uploaded(texture),
                Err(err) => {
                    log::warn!("the Studio background could not be put on the GPU: {err}");
                    Picture::Failed(BAD_IMAGE_NOTE)
                }
            };
        }
        match picture {
            Picture::Uploaded(texture) => Some(Background::Image(texture.clone())),
            _ => None,
        }
    }
}

/// The picture file the frame shows now.
fn wanted_image(frame: &FrameSettings) -> Option<&Path> {
    frame.image.as_deref().filter(|_| frame.enabled && frame.background == BackgroundKind::Image)
}

/// The take that is open: its playback, its Studio data and its preview.
struct Session {
    playback: Playback,
    take: StudioTake,
    preview: Option<Preview>,
    backdrop: BackdropImage,
    /// Whether the settings changed since the preview last took them.
    look_changed: bool,
    /// Why the preview cannot be drawn.
    render_error: Option<String>,
    settle_frames: u8,
}

impl Session {
    fn open(take: &Take) -> Result<Self, String> {
        let events = take.events.as_deref().ok_or("The take has no input track")?;
        let info = probe(&take.video).map_err(|err| err.to_string())?;
        let stream = info.video.as_ref().ok_or("The take has no video")?;
        let start_us = bake::video_start_us(&take.video).map_err(|err| err.to_string())?;
        let mut playback = Playback::open_scaled(&take.video, SOURCE_MAX_WIDTH).map_err(|err| err.to_string())?;
        // Studio draws the cursor itself.
        playback.set_show_cursor(false);
        let length_us = i64::try_from(playback.duration_ms()).unwrap_or(i64::MAX).saturating_mul(1000);
        let video = VideoFacts {
            size: (stream.width, stream.height),
            start_us,
            duration_us: length_us.saturating_sub(start_us).max(0),
            screen: bake::shows_screen(&take.video),
        };
        let take = StudioTake::open(&take.video, events, video).map_err(|err| err.to_string())?;
        Ok(Self { playback, take, preview: None, backdrop: BackdropImage::default(), look_changed: true, render_error: None, settle_frames: SETTLE_FRAMES })
    }

    /// Draws the current frame into a preview of `size` pixels and answers its texture. While the
    /// pointer is down the old size stays, so that dragging a divider does not rebuild the renderer
    /// on every step.
    fn render(&mut self, gpu: &RenderState, size: (u32, u32), pointer_down: bool) -> Option<egui::TextureId> {
        let frame = self.playback.current_frame();
        if self.render_error.is_some() {
            return None;
        }
        if self.preview.as_ref().is_none_or(|preview| preview.size != size && !pointer_down) {
            // The old texture and renderer go first.
            self.preview = None;
            match Preview::new(gpu, size, &self.take) {
                Ok(preview) => {
                    self.preview = Some(preview);
                    self.look_changed = true;
                }
                Err(err) => self.fail(&err),
            }
        }
        if let Some(preview) = &mut self.preview
            && std::mem::take(&mut self.look_changed)
        {
            self.backdrop.sync(&self.take.settings.frame);
            let image = self.backdrop.texture(&self.take.settings.frame, &preview.renderer);
            preview.settings = render_settings(&self.take.settings, image);
            preview.drawn = None;
        }
        if let (Some(preview), Some(frame)) = (&mut self.preview, frame)
            && let Err(err) = preview.draw(&self.take, &frame)
        {
            self.fail(&err);
        }
        self.preview.as_ref().filter(|preview| preview.drawn.is_some()).map(|preview| preview.id)
    }

    fn fail(&mut self, err: &RenderError) {
        log::warn!("the Studio preview could not be drawn: {err}");
        self.render_error = Some(err.to_string());
    }
}

/// What the app keeps for Studio mode.
#[derive(Default)]
pub struct StudioState {
    /// Which folder the list shows.
    pub view: ReviewView,
    /// That folder's takes with an input track, grouped by day.
    pub days: Vec<Day>,
    /// The selected take's name.
    pub selected: Option<String>,
    /// Whether the folder must be scanned again before the panels draw.
    pub dirty: bool,
    /// The tool strip's choice.
    pub tool: StudioTool,
    /// The window's GPU device, which the preview renders on; None without one, as in tests.
    gpu: Option<RenderState>,
    session: Option<Session>,
    /// The main file the session was opened for.
    opened_for: Option<PathBuf>,
    /// Why the selected take could not open.
    open_error: Option<String>,
}

impl StudioState {
    /// The state at launch. The list is scanned when Studio is first on screen.
    pub fn new(gpu: Option<RenderState>) -> Self {
        Self { dirty: true, gpu, ..Self::default() }
    }

    /// Takes in a new scan of the folder, keeping the selected take when it is still listed.
    pub fn set_days(&mut self, days: Vec<Day>) {
        self.days = track_takes(days);
        if review::find(&self.days, self.selected.as_deref()).is_none() {
            self.selected = self.days.iter().flat_map(|day| &day.takes).next().map(|take| take.name.clone());
        }
    }

    /// Opens the selected take while `active` (Studio on screen and no take running), and closes it
    /// otherwise. Another selection closes the open take first.
    pub fn sync(&mut self, active: bool) {
        let wanted = review::find(&self.days, self.selected.as_deref()).filter(|_| active);
        let path = wanted.map(|take| take.video.clone());
        if self.opened_for == path && (path.is_none() || self.session.is_some() || self.open_error.is_some()) {
            return;
        }
        if let Some(mut session) = self.session.take() {
            session.take.flush();
            session.playback.close();
        }
        self.open_error = None;
        self.opened_for = path;
        match review::find(&self.days, self.selected.as_deref()).filter(|_| active).map(Session::open) {
            Some(Ok(session)) => self.session = Some(session),
            Some(Err(reason)) => {
                log::warn!("a take could not open in Studio: {reason}");
                self.open_error = Some(reason);
            }
            None => {}
        }
    }

    /// Writes the open take's settings when their wait is over, and asks for the frame that will.
    pub fn save_when_due(&mut self, ctx: &egui::Context) {
        if let Some(wait) = self.session.as_mut().and_then(|session| session.take.save_when_due(Instant::now())) {
            ctx.request_repaint_after(wait);
        }
    }

    /// Writes the open take's unsaved settings now, as the app closes.
    pub fn flush(&mut self) {
        if let Some(session) = &mut self.session {
            session.take.flush();
        }
    }

    /// The right column: the tool strip and the chosen panel for the open take.
    pub fn tool_panel(&mut self, ui: &mut Ui) {
        layout_ui::choice(ui, "studio-tools", &mut self.tool, &TOOLS);
        ui.add_space(space::SCALE_8);
        if let Some(text) = self.tool.placeholder() {
            studio_panel_ui::caption(ui, text);
            return;
        }
        let Some(session) = &mut self.session else {
            studio_panel_ui::caption(ui, "Open a take to change its settings");
            return;
        };
        session.backdrop.sync(&session.take.settings.frame);
        let facts = PanelFacts { has_chords: session.take.has_chords(), image_note: session.backdrop.note(&session.take.settings.frame) };
        let tool = self.tool;
        let outcome = egui::ScrollArea::vertical()
            .id_salt("studio-panel")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let outcome = studio_panel_ui::show(ui, tool, &mut session.take.settings, facts);
                if let Some(why) = session.take.save_error() {
                    ui.add_space(space::SCALE_8);
                    studio_panel_ui::caption(ui, &format!("Not saved: {why}"));
                }
                outcome
            })
            .inner;
        if outcome.image_picked {
            // The same path can hold a new picture.
            session.backdrop.forget();
        }
        if outcome.changed {
            session.take.changed(Instant::now());
            session.look_changed = true;
            // The preview was drawn before the change.
            session.settle_frames = session.settle_frames.max(1);
            ui.ctx().request_repaint();
        }
    }

    /// What the deck shows of the open take.
    pub fn transport(&self) -> Option<Transport> {
        self.session.as_ref().map(|session| Transport {
            position: session.playback.position_ms(),
            duration: session.playback.duration_ms(),
            playing: session.playback.is_playing(),
            has_zoom: !session.take.settings.zoom.segments.is_empty(),
        })
    }

    /// Carries out a click on the deck.
    pub fn run(&mut self, action: DeckAction) {
        let Some(session) = &mut self.session else { return };
        let position = session.playback.position_ms();
        let target = match action {
            DeckAction::TogglePlay => {
                session.playback.toggle();
                None
            }
            DeckAction::PreviousZoom => review::previous_mark(&session.take.zoom_starts_ms(), position),
            DeckAction::NextZoom => review::next_mark(&session.take.zoom_starts_ms(), position),
        };
        if let Some(ms) = target {
            session.playback.seek(ms);
            session.settle_frames = SETTLE_FRAMES;
        }
    }

    /// The center's picture: the preview letterboxed in the stage, or why there is none.
    pub fn center(&mut self, ui: &mut Ui) {
        let area = ui.available_rect_before_wrap();
        ui.allocate_rect(area, Sense::hover());
        ui.painter().rect_filled(area, radius::GROUP, Theme::get(ui.ctx()).neutrals.bg_stage_inner);
        let Some(take) = review::find(&self.days, self.selected.as_deref()) else {
            review_ui::placeholder_caption(ui, area.center(), "No take with an input track");
            return;
        };
        let Some(session) = &mut self.session else {
            let why = self.open_error.as_deref().unwrap_or("The take's preview opens here");
            review_ui::box_caption(ui, area, &take.name, Some(why));
            return;
        };
        let Some(gpu) = &self.gpu else {
            review_ui::box_caption(ui, area, &take.name, Some("The preview needs the window's GPU"));
            return;
        };
        let (width, height) = session.take.video_size();
        let picture = review_ui::fit_aspect(area, width, height);
        let pixels = picture.size() * ui.ctx().pixels_per_point();
        let largest = gpu.device.limits().max_texture_dimension_2d as f32;
        let size = (pixels.x.round().clamp(1.0, largest) as u32, pixels.y.round().clamp(1.0, largest) as u32);
        let pointer_down = ui.input(|input| input.pointer.any_down());
        match session.render(gpu, size, pointer_down) {
            Some(id) => {
                ui.painter().image(id, picture, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                if let Some(why) = &session.render_error {
                    review_ui::box_caption(ui, area, &take.name, Some(why));
                }
            }
        }
        if session.playback.is_playing() {
            ui.ctx().request_repaint();
        } else if session.settle_frames > 0 {
            session.settle_frames -= 1;
            ui.ctx().request_repaint_after(SETTLE_DELAY);
        }
    }
}

/// What the deck shows of the open take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transport {
    /// The playback position in milliseconds.
    pub position: u64,
    /// The take's length in milliseconds.
    pub duration: u64,
    /// Whether the take is playing.
    pub playing: bool,
    /// Whether the lane has a segment to jump to.
    pub has_zoom: bool,
}

/// What a click on the deck asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckAction {
    /// Play, or pause while playing.
    TogglePlay,
    /// Seek to the start of the zoom segment before the position.
    PreviousZoom,
    /// Seek to the start of the next zoom segment.
    NextZoom,
}

/// The Studio stage: `add_content` above the deck, and Play in the notch. The deck holds the
/// timecode on the left, and previous zoom, next zoom and the total length on the right.
/// `transport` is None with no take open, and every control is disabled then.
pub fn stage(ui: &mut Ui, transport: Option<Transport>, add_content: impl FnOnce(&mut Ui)) -> (Option<DeckAction>, StageResponse) {
    let ready = transport.is_some();
    let playing = transport.is_some_and(|transport| transport.playing);
    let has_zoom = transport.is_some_and(|transport| transport.has_zoom);
    let mut right = None;
    let stage = ui_kit::stage_panel(
        ui,
        egui::Id::new(STAGE_ID),
        PrimaryButton { kind: PlayKind::Play, active: playing, enabled: ready },
        add_content,
        |ui| {
            let (main, fraction) = review_ui::position_timecode(transport.map_or(0, |transport| transport.position));
            let companion = Companion { width: STAGE_BAR_BUTTON_DIAMETER, only_route: false };
            let _ = bar_ui::left_side(ui, &main, &fraction, companion, |_ui| ());
        },
        |ui| {
            let button = |icon, tooltip: &str, action| BarButton { icon, tooltip: tooltip.to_owned(), enabled: has_zoom, action };
            let buttons = [
                button(ui_kit::icons::fill::CARET_LEFT, "Previous zoom", DeckAction::PreviousZoom),
                button(ui_kit::icons::fill::CARET_RIGHT, "Next zoom", DeckAction::NextZoom),
            ];
            let length = transport.map(|transport| review_ui::length_readout(ui, transport.duration));
            right = bar_ui::right_side(ui, &buttons, length);
        },
    );
    let _ = stage.primary.clone().on_hover_text(if playing { "Pause (Space)" } else { "Play (Space)" });
    let play = stage.clicked().then_some(DeckAction::TogglePlay);
    (play.or(right), stage)
}

#[cfg(test)]
mod tests {
    use chrono::Local;

    use super::*;
    use crate::bar_ui::bar_checks;
    use crate::config::{CompositeMode, Mode};

    /// The deck cuts nothing off and keeps the notch's column free, down to the narrowest window.
    #[test]
    fn the_deck_fits_the_narrowest_stage() {
        for width in bar_checks::narrowest_stage_widths(Mode::Studio) {
            let shown = Transport { position: 3_599_000, duration: 359_999_000, playing: false, has_zoom: true };
            let (harness, drawn) = bar_checks::stage_harness(width, move |ui| stage(ui, Some(shown), |_ui| {}).1);
            let drawn = drawn.borrow();
            assert!(drawn.as_ref().is_some_and(|stage| {
                bar_checks::assert_bar_is_clean(&harness, stage, width);
                true
            }));
        }
    }

    fn take(name: &str, events: bool) -> Take {
        Take {
            name: name.to_owned(),
            start: Local::now(),
            duration_ms: None,
            total_size: 0,
            files: Vec::new(),
            video: PathBuf::from(format!("{name}.mkv")),
            iso: None,
            events: events.then(|| PathBuf::from(format!("{name}.events"))),
            video_format: None,
            iso_format: None,
            marks: Vec::new(),
            composite: CompositeMode::Baked,
        }
    }

    #[test]
    fn a_missing_background_image_is_noted() {
        let mut backdrop = BackdropImage::default();
        let mut frame = FrameSettings { enabled: true, background: BackgroundKind::Image, ..FrameSettings::default() };
        backdrop.sync(&frame);
        assert_eq!(backdrop.note(&frame), Some(NO_IMAGE_NOTE));
        frame.image = Some(std::env::temp_dir().join("cutmark-studio-no-such-image.png"));
        backdrop.sync(&frame);
        assert_eq!(backdrop.note(&frame), Some(MISSING_IMAGE_NOTE));
        frame.background = BackgroundKind::Gradient;
        assert_eq!(backdrop.note(&frame), None);
    }

    #[test]
    fn only_takes_with_an_input_track_are_listed() {
        let day = |takes| Day { date: Local::now().date_naive(), takes };
        let mut state = StudioState { selected: Some("plain".to_owned()), ..StudioState::default() };
        state.set_days(vec![day(vec![take("plain", false)]), day(vec![take("tracked", true), take("other", false), take("second", true)])]);
        let listed: Vec<Vec<&str>> = state.days.iter().map(|day| day.takes.iter().map(|take| take.name.as_str()).collect()).collect();
        assert_eq!(listed, [["tracked", "second"]]);
        // The selection moves off a take that is not listed, and stays on one that is.
        assert_eq!(state.selected.as_deref(), Some("tracked"));
        state.selected = Some("second".to_owned());
        state.set_days(vec![day(vec![take("tracked", true), take("second", true)])]);
        assert_eq!(state.selected.as_deref(), Some("second"));
        state.set_days(vec![day(vec![take("plain", false)])]);
        assert!(state.days.is_empty() && state.selected.is_none());
    }
}
