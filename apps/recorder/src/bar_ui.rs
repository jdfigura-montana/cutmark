//! The stage's bottom bar and notch button: the narrow-bar rule every mode shares, and the
//! Setup and Record stage with its take controls.
//!
//! See docs/suite/Recorder-E3.md, sections 2 and 3.

use egui::text::LayoutJob;
use egui::{Align, Layout, Response, Ui, vec2};
use ui_kit::tokens::{space, typography};
use ui_kit::{PlayKind, PrimaryButton, STAGE_BAR_BUTTON_DIAMETER, StageResponse, Theme, TimecodeSize};

use crate::app::{Chords, send};
use crate::engine::bus::{BusSender, CommandKind, MarkKind};
use crate::engine::controller::Phase;

/// The id of the Setup and Record stage.
const TAKE_STAGE_ID: &str = "recorder-take-stage";
/// Space between the buttons of the bar's right side, and before its readout. Between the
/// buttons it shrinks, down to nothing, before a button is left out.
const BUTTON_GAP: f32 = space::SCALE_8;
/// Space between the timecode and the control beside it.
const TIMECODE_GAP: f32 = space::SCALE_12;
/// The timecode while no take runs.
const IDLE_TIMECODE: &str = "00:00:00";
/// The idle timecode's frame part.
const IDLE_TIMECODE_FRACTION: &str = ":00";
/// The label over the next take's name.
const NEXT_TAKE_LABEL: &str = "Next take";

/// How the bar's left side draws its timecode, from the widest to nothing. Nothing in the bar
/// is ever cut off: what does not fit whole is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimecodeFit {
    /// Large, with the fraction.
    Large,
    /// Small, with the fraction.
    Value,
    /// Small, without the fraction.
    NoFraction,
    /// Not drawn.
    Hidden,
}

/// The timecode's widths in its three forms.
#[derive(Debug, Clone, Copy)]
struct TimecodeWidths {
    large: f32,
    value: f32,
    no_fraction: f32,
}

/// The control beside the timecode.
#[derive(Debug, Clone, Copy)]
pub struct Companion {
    /// How wide it draws.
    pub width: f32,
    /// True for a control with no other route, which outlasts the timecode. Any other control
    /// goes before the timecode does.
    pub only_route: bool,
}

/// What fits in the left side, `column` wide: the form of the timecode, and whether the
/// control beside it is drawn. The timecode shrinks before the control goes.
fn left_fit(column: f32, timecode: TimecodeWidths, companion: Companion) -> (TimecodeFit, bool) {
    let forms = [(TimecodeFit::Large, timecode.large), (TimecodeFit::Value, timecode.value), (TimecodeFit::NoFraction, timecode.no_fraction)];
    let beside = TIMECODE_GAP + companion.width;
    if let Some((form, _)) = forms.iter().find(|(_, width)| width + beside <= column) {
        (*form, true)
    } else if companion.only_route && companion.width <= column {
        (TimecodeFit::Hidden, true)
    } else if let Some((form, _)) = forms.iter().skip(1).find(|(_, width)| *width <= column) {
        (*form, false)
    } else {
        (TimecodeFit::Hidden, false)
    }
}

/// The width of `count` bar buttons in a row, `gap` apart.
fn buttons_width(count: usize, gap: f32) -> f32 {
    (0..count).fold(0.0, |width, index| width + STAGE_BAR_BUTTON_DIAMETER + if index == 0 { 0.0 } else { gap })
}

/// What the bar's right side draws.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RightFit {
    /// How many buttons, from the first.
    shown: usize,
    /// The space between them.
    gap: f32,
    /// Whether the readout is drawn after them.
    readout: bool,
}

/// What fits in the right side, `column` wide, of its `count` buttons and the readout,
/// `readout` wide. The readout goes first, then the gaps shrink, then buttons go from the end.
fn right_fit(column: f32, count: usize, readout: f32) -> RightFit {
    let shown = (0..=count).rev().find(|shown| buttons_width(*shown, 0.0) <= column).unwrap_or(0);
    let gaps = shown.saturating_sub(1).max(1) as f32;
    let gap = ((column - buttons_width(shown, 0.0)) / gaps).clamp(0.0, BUTTON_GAP);
    let readout = shown == count && buttons_width(count, BUTTON_GAP) + BUTTON_GAP + readout <= column;
    RightFit { shown, gap, readout }
}

/// The bar's left side: the timecode (`main` and its `fraction`), then the control
/// `add_companion` draws. Returns what the control returned, when there was room for it.
pub fn left_side<R>(ui: &mut Ui, main: &str, fraction: &str, companion: Companion, add_companion: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let widths = TimecodeWidths {
        large: ui_kit::timecode_width(ui, main, fraction, TimecodeSize::Large),
        value: ui_kit::timecode_width(ui, main, fraction, TimecodeSize::Value),
        no_fraction: ui_kit::timecode_width(ui, main, "", TimecodeSize::Value),
    };
    let (timecode, companion_fits) = left_fit(ui.available_width(), widths, companion);
    ui.spacing_mut().item_spacing.x = TIMECODE_GAP;
    match timecode {
        TimecodeFit::Large => drop(ui_kit::timecode_sized(ui, main, fraction, TimecodeSize::Large)),
        TimecodeFit::Value => drop(ui_kit::timecode_sized(ui, main, fraction, TimecodeSize::Value)),
        TimecodeFit::NoFraction => drop(ui_kit::timecode_sized(ui, main, "", TimecodeSize::Value)),
        TimecodeFit::Hidden => {}
    }
    companion_fits.then(|| add_companion(ui))
}

/// One button of the bar's right side.
pub struct BarButton<A> {
    /// The phosphor fill glyph.
    pub icon: &'static str,
    /// The tooltip, which is also the accessible label.
    pub tooltip: String,
    /// A disabled button draws faded and reports no clicks.
    pub enabled: bool,
    /// What a click asks for.
    pub action: A,
}

/// The bar's right side: the buttons, the most important first, then the small readout. A
/// narrow side leaves out the readout first, then closes up the buttons, then leaves out
/// buttons from the end. Returns the clicked button's action.
pub fn right_side<A: Clone>(ui: &mut Ui, buttons: &[BarButton<A>], readout: Option<LayoutJob>) -> Option<A> {
    let readout_width = readout.as_ref().map_or(0.0, |job| ui.painter().layout_job(job.clone()).size().x);
    let fit = right_fit(ui.available_width(), buttons.len(), readout_width);
    ui.spacing_mut().item_spacing.x = fit.gap;
    let mut clicked = None;
    for button in buttons.iter().take(fit.shown) {
        if bar_button(ui, button.icon, &button.tooltip, button.enabled).clicked() {
            clicked = Some(button.action.clone());
        }
    }
    if fit.readout
        && let Some(job) = readout
    {
        ui.add(egui::Label::new(job).extend().selectable(false));
    }
    clicked
}

/// A ui-kit bar button that can be disabled.
pub fn bar_button(ui: &mut Ui, icon: &str, tooltip: &str, enabled: bool) -> Response {
    ui.add_enabled_ui(enabled, |ui| ui_kit::bar_button(ui, icon, tooltip)).inner
}

/// An action tooltip with its hotkey chord, when one is registered.
pub fn tip(action: &str, chord: &str) -> String {
    if chord.is_empty() { action.to_owned() } else { format!("{action} ({chord})") }
}

/// What the Setup and Record stage needs from the app for one frame.
pub struct TakeBar<'a> {
    /// The controller phase.
    pub phase: Phase,
    /// The frame rate of the running take, for the timecode.
    pub fps: u32,
    /// The frames captured so far, while a take runs.
    pub frames: Option<u64>,
    /// The countdown setting in seconds; 0 is off.
    pub countdown_secs: u8,
    /// The hotkey chords the tooltips show.
    pub chords: &'a Chords,
    /// The next take's short name, shown while no take runs; None when it is unknown.
    pub next_take: Option<&'a str>,
    /// The next take's full video file path, or why it is unknown, as the name's tooltip.
    pub next_take_path: &'a str,
}

/// The notch button of Setup and Record: Record, or the stop square while a take runs. It is
/// never disabled: a press during a countdown or a pre-flight cancels it, and the controller
/// refuses a start it cannot carry out.
fn take_primary(phase: Phase) -> PrimaryButton {
    PrimaryButton { kind: PlayKind::Record, active: phase.has_take(), enabled: true }
}

/// The Setup and Record stage: `add_content` above the bottom bar, and Record in the notch.
/// Every control sends the command the menu and the hotkeys send, and every tooltip names the
/// hotkey.
pub fn take_stage(ui: &mut Ui, bar: &TakeBar<'_>, bus: &BusSender, add_content: impl FnOnce(&mut Ui)) -> StageResponse {
    let running = bar.phase.has_take();
    let stage = ui_kit::stage_panel(
        ui,
        egui::Id::new(TAKE_STAGE_ID),
        take_primary(bar.phase),
        add_content,
        |ui| take_left(ui, bar, bus),
        |ui| {
            if let Some(command) = right_side(ui, &take_buttons(bar), countdown_readout(ui, bar)) {
                send(bus, command);
            }
        },
    );
    let action = if running { "Stop recording" } else { "Start recording" };
    let _ = stage.primary.clone().on_hover_text(tip(action, &bar.chords.start_stop));
    if stage.clicked() {
        send(bus, CommandKind::StartStop);
    }
    stage
}

/// The right side's buttons, each with the command it sends: Pause or Resume and Mute mic
/// while a take runs; Add mark (which the left side holds during a take) and the disabled
/// Pause while idle.
fn take_buttons(bar: &TakeBar<'_>) -> [BarButton<CommandKind>; 2] {
    let running = bar.phase.has_take();
    let (icon, action) = if bar.phase == Phase::Paused { (ui_kit::icons::fill::PLAY, "Resume") } else { (ui_kit::icons::fill::PAUSE, "Pause") };
    let button = |icon, action: &str, chord: &str, enabled, command| BarButton { icon, tooltip: tip(action, chord), enabled, action: command };
    let pause = button(icon, action, &bar.chords.pause_resume, running, CommandKind::PauseResume);
    if running {
        // The mic is assumed to sit on track 0, like the hotkey does.
        let mute = button(ui_kit::icons::fill::MICROPHONE_SLASH, "Mute mic", &bar.chords.mute_mic, true, CommandKind::Mute { track: 0 });
        [pause, mute]
    } else {
        let mark = button(ui_kit::icons::fill::FLAG, "Add mark", &bar.chords.mark, true, CommandKind::Mark(MarkKind::Silent));
        [mark, pause]
    }
}

/// The left side: the timecode, then Add mark while a take runs, or the next take's name.
fn take_left(ui: &mut Ui, bar: &TakeBar<'_>, bus: &BusSender) {
    let (main, fraction) = match (bar.phase.has_take(), bar.frames) {
        (true, Some(frames)) => take_timecode(frames, bar.fps),
        _ => (IDLE_TIMECODE.to_owned(), IDLE_TIMECODE_FRACTION.to_owned()),
    };
    if bar.phase.has_take() {
        let companion = Companion { width: STAGE_BAR_BUTTON_DIAMETER, only_route: false };
        left_side(ui, &main, &fraction, companion, |ui| {
            if bar_button(ui, ui_kit::icons::fill::FLAG, &tip("Add mark", &bar.chords.mark), true).clicked() {
                send(bus, CommandKind::Mark(MarkKind::Silent));
            }
        });
    } else {
        let width = next_take_width(ui, bar.next_take);
        left_side(ui, &main, &fraction, Companion { width, only_route: false }, |ui| next_take(ui, bar, width));
    }
}

/// The next take's two lines: the label, and the name in Martian Mono.
fn next_take_jobs(ui: &Ui, name: Option<&str>) -> (LayoutJob, Option<LayoutJob>) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let font = ui_kit::fonts::medium(typography::CAPTION_SIZE);
    let label = LayoutJob::simple_singleline(NEXT_TAKE_LABEL.to_owned(), font, neutrals.text_tertiary);
    let name = name.map(|name| {
        let font = ui_kit::fonts::mono(typography::VALUE_SIZE);
        ui_kit::fonts::tracked(name, font, neutrals.text_secondary, ui_kit::fonts::MONO_LETTER_SPACING_EM)
    });
    (label, name)
}

/// How wide the next take's block draws.
fn next_take_width(ui: &Ui, name: Option<&str>) -> f32 {
    let (label, name) = next_take_jobs(ui, name);
    let width = |job: LayoutJob| ui.painter().layout_job(job).size().x;
    name.map_or(0.0, width).max(width(label)).ceil()
}

/// The next take's block, the label over the name, centered on the bar's line. Hovering it
/// shows the full path.
fn next_take(ui: &mut Ui, bar: &TakeBar<'_>, width: f32) {
    let (label, name) = next_take_jobs(ui, bar.next_take);
    let line = |job: LayoutJob| ui.painter().layout_job(job);
    let (label, name) = (line(label), name.map(line));
    let height = label.size().y + name.as_ref().map_or(0.0, |name| name.size().y);
    ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut response = ui.add(egui::Label::new(label).selectable(false));
        if let Some(name) = name {
            response = response.union(ui.add(egui::Label::new(name).selectable(false)));
        }
        if !bar.next_take_path.is_empty() {
            response.on_hover_text(bar.next_take_path);
        }
    });
}

/// The countdown readout of the right side, when there is one to show.
fn countdown_readout(ui: &Ui, bar: &TakeBar<'_>) -> Option<LayoutJob> {
    let caption = countdown_caption(bar.phase, bar.countdown_secs);
    (!caption.is_empty())
        .then(|| LayoutJob::simple_singleline(caption, ui_kit::fonts::medium(typography::CAPTION_SIZE), Theme::get(ui.ctx()).neutrals.text_tertiary))
}

/// The running take's timecode: hours, minutes, seconds and the frame within
/// the second, from the frames the take has captured (paused time removed).
fn take_timecode(frames: u64, fps: u32) -> (String, String) {
    let fps = u64::from(fps.max(1));
    let seconds = frames / fps;
    (format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60), format!(":{:02}", frames % fps))
}

/// The countdown readout's text: the countdown setting while idle, "Starting in N"
/// while the countdown runs, nothing while a take records.
fn countdown_caption(phase: Phase, countdown_secs: u8) -> String {
    match phase {
        Phase::Countdown(left) => format!("Starting in {left}"),
        Phase::Idle => match countdown_secs {
            0 => String::new(),
            secs => format!("{secs} s countdown"),
        },
        Phase::Recording | Phase::Paused => String::new(),
    }
}

/// What the kittest checks of both stages' bars share.
#[cfg(test)]
pub(crate) mod bar_checks {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use egui::accesskit::Role;
    use egui::{Rect, Shape, Ui, UiBuilder, pos2, vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use ui_kit::StageResponse;
    use ui_kit::tokens::space;

    use crate::app::{three_columns, two_columns};
    use crate::config::{self, Mode, PanelWidths};

    /// The height of the test stage, with room under it for the notch button.
    const STAGE_HEIGHT: f32 = 300.0;

    /// Where a harness leaves the stage it drew last.
    pub(crate) type Drawn = Rc<RefCell<Option<StageResponse>>>;

    /// A harness that draws one stage `width` wide, with the suite's fonts and style.
    pub(crate) fn stage_harness(width: f32, mut draw: impl FnMut(&mut Ui) -> StageResponse + 'static) -> (Harness<'static>, Drawn) {
        let drawn = Drawn::default();
        let last = Rc::clone(&drawn);
        let first_pass = Cell::new(true);
        let mut harness = Harness::builder().with_size(vec2(width + 200.0, STAGE_HEIGHT + 100.0)).build_ui(move |ui| {
            // The fonts a style asks for are only there from the next pass on.
            if first_pass.replace(false) {
                ui_kit::apply(ui.ctx());
                return;
            }
            let rect = Rect::from_min_size(pos2(100.0, 0.0), vec2(width, STAGE_HEIGHT));
            let mut column = ui.new_child(UiBuilder::new().max_rect(rect));
            column.set_clip_rect(rect);
            *last.borrow_mut() = Some(draw(&mut column));
        });
        harness.run();
        (harness, drawn)
    }

    /// The width of `mode`'s stage in a window that wide, at the default panel widths.
    pub(crate) fn stage_width(mode: Mode, window_width: f32) -> f32 {
        let body = Rect::from_min_size(pos2(0.0, 0.0), vec2(window_width, 400.0)).shrink(space::WINDOW_GAP);
        match mode {
            Mode::Record => two_columns(body, PanelWidths::default())[0].width(),
            Mode::Setup | Mode::Review => three_columns(body, PanelWidths::default())[1].width(),
        }
    }

    /// `mode`'s stage widths in the narrowest window the Recorder allows: with the default
    /// panel widths, and with the widest saved ones.
    pub(crate) fn narrowest_stage_widths(mode: Mode) -> [f32; 2] {
        let body = Rect::from_min_size(pos2(0.0, 0.0), vec2(config::MIN_WINDOW_WIDTH, 400.0)).shrink(space::WINDOW_GAP);
        let widest = PanelWidths { left: config::LEFT_PANEL_MAX, right: config::RIGHT_PANEL_MAX };
        let dragged = match mode {
            Mode::Record => two_columns(body, widest)[0].width(),
            Mode::Setup | Mode::Review => three_columns(body, widest)[1].width(),
        };
        [stage_width(mode, config::MIN_WINDOW_WIDTH), dragged]
    }

    /// Fails when anything but the notch button reaches into the bar's center column, or when
    /// a control or a text of the bar is cut off.
    pub(crate) fn assert_bar_is_clean(harness: &Harness<'_>, stage: &StageResponse, width: f32) {
        // The panel's own span: the center column of a stage narrower than it reaches past it.
        let center = stage.bar_center_rect;
        let mut sides = vec![
            Rect::from_x_y_ranges(stage.panel_rect.left()..=center.left().max(stage.panel_rect.left()), stage.bar_rect.y_range()),
            Rect::from_x_y_ranges(center.right().min(stage.panel_rect.right())..=stage.panel_rect.right(), stage.bar_rect.y_range()),
        ];
        sides.retain(|side| side.width() > 0.0);
        let whole = |rect: Rect| sides.iter().any(|side| side.expand(0.5).contains_rect(rect));
        for button in harness.query_all_by_role(Role::Button) {
            let rect = button.rect();
            if !stage.bar_rect.contains(rect.center()) || stage.primary.rect.expand(0.5).contains_rect(rect) {
                continue;
            }
            assert!(whole(rect), "at {width} a bar button {rect:?} is cut off or in the center column {center:?}");
        }
        for clipped in &harness.output().shapes {
            let Shape::Text(text) = &clipped.shape else { continue };
            let rect = Rect::from_min_size(text.pos, text.galley.size());
            if text.galley.text().is_empty() || !stage.bar_rect.contains(rect.center()) || stage.primary.rect.contains_rect(rect) {
                continue;
            }
            // A button's glyph may be taller than its line box; what counts is the width.
            assert!(whole(rect), "at {width} the bar text {:?} is cut off or in the center column: {rect:?}", text.galley.text());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use egui_kittest::kittest::Queryable;

    use super::bar_checks::{self, assert_bar_is_clean, stage_harness, stage_width};
    use super::*;
    use crate::config::{self, Mode};
    use crate::engine::bus::Bus;

    fn chords() -> Chords {
        Chords {
            start_stop: "F10".to_owned(),
            pause_resume: "F11".to_owned(),
            mark: "F9".to_owned(),
            mark_label: "Shift+F9".to_owned(),
            mute_mic: "F8".to_owned(),
        }
    }

    /// One take stage over a command bus with no controller behind it, so nothing is captured.
    fn take_harness(width: f32, phase: Rc<Cell<Phase>>, countdown_secs: u8) -> (egui_kittest::Harness<'static>, bar_checks::Drawn, Bus) {
        let bus = Bus::default();
        let sender = bus.sender();
        let chords = chords();
        let (harness, drawn) = stage_harness(width, move |ui| {
            let bar = TakeBar {
                phase: phase.get(),
                fps: 60,
                frames: phase.get().has_take().then_some(60 * 3601 + 1),
                countdown_secs,
                chords: &chords,
                next_take: Some("take07"),
                next_take_path: "D:\\Takes\\take07.mkv",
            };
            take_stage(ui, &bar, &sender, |_| {})
        });
        (harness, drawn, bus)
    }

    /// Everything sent since the last call.
    fn sent(bus: &Bus) -> Vec<CommandKind> {
        std::iter::from_fn(|| bus.recv_timeout(Duration::ZERO).map(|command| command.kind)).collect()
    }

    fn click(harness: &mut egui_kittest::Harness<'_>, label: &str) {
        harness.get_by_label(label).click();
        harness.run();
    }

    /// The notch button sends start or stop: it reads Record while idle, starts the take, and
    /// reads Stop recording while the take records or is paused, which stops it.
    #[test]
    fn the_notch_button_starts_and_stops_a_take() {
        let phase = Rc::new(Cell::new(Phase::Idle));
        let (mut harness, _, bus) = take_harness(700.0, Rc::clone(&phase), 3);
        assert!(sent(&bus).is_empty(), "drawing sends nothing");
        click(&mut harness, "Record");
        assert_eq!(sent(&bus), [CommandKind::StartStop]);
        for running in [Phase::Recording, Phase::Paused] {
            phase.set(running);
            harness.run();
            assert!(harness.query_by_label("Record").is_none(), "{running:?} shows the stop square");
            click(&mut harness, "Stop recording");
            assert_eq!(sent(&bus), [CommandKind::StartStop]);
        }
        // A press during the countdown cancels it, as the old Record capsule did.
        phase.set(Phase::Countdown(2));
        harness.run();
        click(&mut harness, "Record");
        assert_eq!(sent(&bus), [CommandKind::StartStop]);
    }

    /// Every other control of the old deck is on the bar and sends the same command.
    #[test]
    fn the_bar_buttons_send_the_old_deck_commands() {
        let phase = Rc::new(Cell::new(Phase::Idle));
        let (mut harness, _, bus) = take_harness(700.0, Rc::clone(&phase), 3);
        click(&mut harness, "Add mark (F9)");
        assert_eq!(sent(&bus), [CommandKind::Mark(MarkKind::Silent)]);
        click(&mut harness, "Pause (F11)");
        assert!(sent(&bus).is_empty(), "Pause is disabled while idle");
        assert!(harness.query_by_label("Mute mic (F8)").is_none(), "Mute mic shows only during a take");

        phase.set(Phase::Recording);
        harness.run();
        click(&mut harness, "Add mark (F9)");
        assert_eq!(sent(&bus), [CommandKind::Mark(MarkKind::Silent)]);
        click(&mut harness, "Pause (F11)");
        assert_eq!(sent(&bus), [CommandKind::PauseResume]);
        click(&mut harness, "Mute mic (F8)");
        assert_eq!(sent(&bus), [CommandKind::Mute { track: 0 }]);

        phase.set(Phase::Paused);
        harness.run();
        click(&mut harness, "Resume (F11)");
        assert_eq!(sent(&bus), [CommandKind::PauseResume]);
    }

    /// The idle bar names the next take and the countdown; a running take shows neither.
    #[test]
    fn the_idle_bar_shows_the_next_take_and_the_countdown() {
        let phase = Rc::new(Cell::new(Phase::Idle));
        let (mut harness, _, _bus) = take_harness(900.0, Rc::clone(&phase), 3);
        for text in ["Next take", "take07", "3 s countdown"] {
            assert!(harness.query_by_label(text).is_some(), "the idle bar shows {text}");
        }
        phase.set(Phase::Recording);
        harness.run();
        for text in ["Next take", "take07", "3 s countdown"] {
            assert!(harness.query_by_label(text).is_none(), "a take hides {text}");
        }
    }

    /// From the narrowest window up, in every phase, the bar's center column stays empty and
    /// nothing in the bar is cut off.
    #[test]
    fn the_take_bar_center_column_stays_free_at_minimum_width() {
        let minimum = config::MIN_WINDOW_WIDTH;
        let mut widths = vec![stage_width(Mode::Setup, minimum), stage_width(Mode::Record, minimum)];
        widths.extend((100..=900).step_by(20).map(|width| width as f32));
        for width in widths {
            for running in [Phase::Idle, Phase::Countdown(2), Phase::Recording, Phase::Paused] {
                let (harness, drawn, _bus) = take_harness(width, Rc::new(Cell::new(running)), 3);
                let stage = drawn.borrow().clone().expect("the stage was drawn");
                assert_bar_is_clean(&harness, &stage, width);
            }
        }
    }

    /// In the narrowest window every action of the Setup and Record bars is shown, at the
    /// default panel widths and with the panels as wide as they go.
    #[test]
    fn the_narrowest_window_shows_every_take_bar_action() {
        for mode in [Mode::Setup, Mode::Record] {
            for width in bar_checks::narrowest_stage_widths(mode) {
                let phases: &[(Phase, &[&str])] = &[
                    (Phase::Idle, &["Record", "Add mark (F9)", "Pause (F11)"]),
                    (Phase::Recording, &["Stop recording", "Add mark (F9)", "Pause (F11)", "Mute mic (F8)"]),
                    (Phase::Paused, &["Stop recording", "Add mark (F9)", "Resume (F11)", "Mute mic (F8)"]),
                ];
                for (phase, labels) in phases {
                    // A take is only ever on screen in Record mode.
                    if mode == Mode::Setup && phase.has_take() {
                        continue;
                    }
                    let (harness, drawn, _bus) = take_harness(width, Rc::new(Cell::new(*phase)), 3);
                    for label in *labels {
                        assert!(harness.query_by_label(label).is_some(), "{label} is shown at {width} in {mode:?}");
                    }
                    let stage = drawn.borrow().clone().expect("the stage was drawn");
                    assert_bar_is_clean(&harness, &stage, width);
                }
            }
        }
    }

    /// The default window has room for the whole idle bar of Setup, the narrower of the two
    /// stages that show it.
    #[test]
    fn the_default_window_shows_the_whole_setup_bar() {
        let width = stage_width(Mode::Setup, 1280.0);
        let (harness, _, _bus) = take_harness(width, Rc::new(Cell::new(Phase::Idle)), 3);
        for label in ["Record", "Add mark (F9)", "Pause (F11)", "00:00:00:00", "Next take", "take07", "3 s countdown"] {
            assert!(harness.query_by_label(label).is_some(), "{label} is shown");
        }
    }

    /// The default window has room for every control of a running take.
    #[test]
    fn the_default_window_shows_the_whole_record_bar() {
        let width = stage_width(Mode::Record, 1280.0);
        let phase = Rc::new(Cell::new(Phase::Recording));
        let (harness, _, _bus) = take_harness(width, phase, 3);
        for label in ["Stop recording", "Add mark (F9)", "Pause (F11)", "Mute mic (F8)", "01:00:01:01"] {
            assert!(harness.query_by_label(label).is_some(), "{label} is shown");
        }
    }

    /// The right side drops the readout before any button, closes up the buttons, and only
    /// then drops buttons from the end.
    #[test]
    fn a_narrow_right_side_drops_the_readout_first() {
        let two = buttons_width(2, BUTTON_GAP);
        assert_eq!(two, 2.0 * STAGE_BAR_BUTTON_DIAMETER + BUTTON_GAP);
        let fit = |column, count| right_fit(column, count, 60.0);
        assert_eq!(fit(two + BUTTON_GAP + 60.0, 2), RightFit { shown: 2, gap: BUTTON_GAP, readout: true });
        assert_eq!(fit(two + BUTTON_GAP + 59.0, 2), RightFit { shown: 2, gap: BUTTON_GAP, readout: false });
        assert_eq!(fit(two - 1.0, 2), RightFit { shown: 2, gap: BUTTON_GAP - 1.0, readout: false });
        assert_eq!(fit(3.0 * STAGE_BAR_BUTTON_DIAMETER, 3), RightFit { shown: 3, gap: 0.0, readout: false });
        assert_eq!(fit(2.0 * STAGE_BAR_BUTTON_DIAMETER - 1.0, 2).shown, 1);
        assert_eq!(fit(0.0, 2).shown, 0);
    }

    /// The left side shrinks the timecode before it drops the control beside it.
    #[test]
    fn a_narrow_left_side_shrinks_the_timecode_first() {
        let widths = TimecodeWidths { large: 160.0, value: 90.0, no_fraction: 66.0 };
        let companion = Companion { width: 40.0, only_route: false };
        let beside = TIMECODE_GAP + companion.width;
        assert_eq!(left_fit(160.0 + beside, widths, companion), (TimecodeFit::Large, true));
        assert_eq!(left_fit(159.0 + beside, widths, companion), (TimecodeFit::Value, true));
        assert_eq!(left_fit(89.0 + beside, widths, companion), (TimecodeFit::NoFraction, true));
        assert_eq!(left_fit(65.0 + beside, widths, companion), (TimecodeFit::Value, false));
        assert_eq!(left_fit(89.0, widths, companion), (TimecodeFit::NoFraction, false));
        assert_eq!(left_fit(65.0, widths, companion), (TimecodeFit::Hidden, false));
    }

    /// A control with no other route stays after the timecode has gone.
    #[test]
    fn a_control_with_no_other_route_outlasts_the_timecode() {
        let widths = TimecodeWidths { large: 160.0, value: 90.0, no_fraction: 66.0 };
        let companion = Companion { width: 40.0, only_route: true };
        assert_eq!(left_fit(65.0 + TIMECODE_GAP + 40.0, widths, companion), (TimecodeFit::Hidden, true));
        assert_eq!(left_fit(40.0, widths, companion), (TimecodeFit::Hidden, true));
        assert_eq!(left_fit(39.0, widths, companion), (TimecodeFit::Hidden, false));
    }

    /// The countdown readout follows the setting and the running countdown.
    #[test]
    fn the_countdown_caption_follows_the_setting_and_the_countdown() {
        assert_eq!(countdown_caption(Phase::Idle, 0), "");
        assert_eq!(countdown_caption(Phase::Idle, 3), "3 s countdown");
        assert_eq!(countdown_caption(Phase::Countdown(2), 5), "Starting in 2");
        assert_eq!(countdown_caption(Phase::Recording, 3), "");
    }

    /// The running timecode splits the captured frames into time and frame.
    #[test]
    fn the_running_timecode_counts_time_and_frames() {
        assert_eq!(take_timecode(0, 60), ("00:00:00".to_owned(), ":00".to_owned()));
        // One hour, one second and one frame at 60 fps.
        assert_eq!(take_timecode(60 * 3601 + 1, 60), ("01:00:01".to_owned(), ":01".to_owned()));
    }
}
