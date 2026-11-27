//! The input track of a capture: one input logger feeds the running take's `{stem}.events` file and
//! the replay buffer's history.
//!
//! See the input track spec: Recording, items 1, 6, 8 and 9. Event times in a file follow the
//! video's timeline: paused time is taken out, with the same frame borders the take gate cuts the
//! video at. A take's file is created once its first video frame has arrived, and until then its
//! events are held. Wherever a file starts or goes on after a gap, the cursor, window and app of
//! that moment are stated again.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use events::{
    AppSwitch, CaptureSource, CursorSample, CursorShape, DisplayInfo, EventsError, EventsWriter, GeometryChange, GeometryEvent, Header, InputEvent, ScreenRect,
    Streams,
};

use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::config::{CursorMode, Profile};

/// How often the thread takes in the logger's events.
const POLL: Duration = Duration::from_millis(20);

/// How often a take's events file is flushed.
const FLUSH_EVERY: Duration = Duration::from_secs(1);

/// The history kept beyond what the replay buffer needs, and all of it while the buffer is off: a take is attached a
/// moment after its first frame.
const HISTORY_MARGIN: Duration = Duration::from_secs(5);

/// The toast when the logger cannot start.
pub(crate) const START_FAILED: &str = "Input tracking could not start, so the cursor is recorded in the video.";

/// The toast when the logger ends on its own.
const DIED: &str = "Input tracking stopped. Recording goes on, but the input track is incomplete.";

/// The toast when a take begins on a capture whose logger has already ended.
const ALREADY_DEAD: &str = "Input tracking has stopped, so this take has no cursor. Stop all recording and turn the replay buffer off to restart it.";

/// The events file of a take.
pub(crate) fn events_path(take_folder: &Path, base_name: &str) -> PathBuf {
    take_folder.join(format!("{base_name}.events"))
}

/// A running input logger.
pub(crate) trait InputSource: Send {
    /// The next event that has arrived, if any.
    fn try_next(&mut self) -> Option<InputEvent>;
    /// Whether the logger can still deliver events.
    fn is_running(&self) -> bool;
}

/// What the logger read when it started.
pub(crate) struct SourceInfo {
    /// QPC ticks per second.
    pub(crate) qpc_frequency: i64,
    /// The Windows cursor size setting.
    pub(crate) cursor_size: u32,
    /// The display layout at the logger's start.
    pub(crate) displays: Vec<DisplayInfo>,
    /// What the take captures.
    pub(crate) capture: CaptureSource,
    /// The app in front at the logger's start, which the logger itself never reports. None while app switches are off.
    pub(crate) app: Option<AppSwitch>,
}

/// The streams a Track profile writes.
pub(crate) fn streams(profile: &Profile) -> Streams {
    Streams { cursor: true, clicks: profile.record_clicks, keys: profile.record_keys, apps: profile.record_apps }
}

/// How a capture handles the cursor.
pub(crate) struct CursorPlan {
    /// Whether Windows Graphics Capture draws the cursor into the video.
    pub(crate) burn_in: bool,
    /// The running input track, with Track.
    pub(crate) track: Option<InputTrack>,
    /// Why the cursor is burned in although the profile asked for Track.
    pub(crate) failure: Option<String>,
}

/// Decides the cursor of a new capture. `start` is only called for Track; when it fails, the cursor is burned in.
pub(crate) fn plan_cursor(mode: CursorMode, start: impl FnOnce() -> Result<InputTrack, String>) -> CursorPlan {
    match mode {
        CursorMode::Show => CursorPlan { burn_in: true, track: None, failure: None },
        CursorMode::Hide => CursorPlan { burn_in: false, track: None, failure: None },
        CursorMode::Track => match start() {
            Ok(track) => CursorPlan { burn_in: false, track: Some(track), failure: None },
            Err(reason) => {
                log::warn!("the input logger could not start: {reason}");
                CursorPlan { burn_in: true, track: None, failure: Some(START_FAILED.to_owned()) }
            }
        },
    }
}

/// The QPC stamp of an event; None for a shape, which has no time.
fn stamp(event: &InputEvent) -> Option<i64> {
    match event {
        InputEvent::Sample(sample) => Some(sample.qpc),
        InputEvent::Mouse(mouse) => Some(mouse.qpc),
        InputEvent::Geometry(geometry) => Some(geometry.qpc),
        InputEvent::Chord(chord) => Some(chord.qpc),
        InputEvent::App(app) => Some(app.qpc),
        InputEvent::Shape(_) => None,
    }
}

/// The event with another QPC stamp.
fn restamped(mut event: InputEvent, qpc: i64) -> InputEvent {
    match &mut event {
        InputEvent::Sample(sample) => sample.qpc = qpc,
        InputEvent::Mouse(mouse) => mouse.qpc = qpc,
        InputEvent::Geometry(geometry) => geometry.qpc = qpc,
        InputEvent::Chord(chord) => chord.qpc = qpc,
        InputEvent::App(app) => app.qpc = qpc,
        InputEvent::Shape(_) => {}
    }
    event
}

/// What holds at one instant, which a file that begins there relies on.
#[derive(Clone, Default)]
struct State {
    sample: Option<CursorSample>,
    window: Option<ScreenRect>,
    displays: Vec<DisplayInfo>,
    app: Option<AppSwitch>,
}

impl State {
    fn apply(&mut self, event: &InputEvent) {
        match event {
            InputEvent::Sample(sample) => self.sample = Some(*sample),
            InputEvent::Geometry(geometry) => match &geometry.change {
                GeometryChange::WindowRect(rect) => self.window = Some(*rect),
                GeometryChange::Displays(displays) => self.displays.clone_from(displays),
            },
            InputEvent::App(app) => self.app = Some(app.clone()),
            InputEvent::Shape(_) | InputEvent::Mouse(_) | InputEvent::Chord(_) => {}
        }
    }

    /// The events that bring a file holding `written` up to this state at `qpc`. The cursor is always stated.
    fn restate(&self, written: &State, qpc: i64) -> Vec<InputEvent> {
        let mut out = Vec::new();
        if self.displays != written.displays {
            out.push(InputEvent::Geometry(GeometryEvent { qpc, change: GeometryChange::Displays(self.displays.clone()) }));
        }
        if let Some(rect) = self.window.filter(|rect| written.window != Some(*rect)) {
            out.push(InputEvent::Geometry(GeometryEvent { qpc, change: GeometryChange::WindowRect(rect) }));
        }
        if let Some(sample) = self.sample {
            out.push(InputEvent::Sample(CursorSample { qpc, ..sample }));
        }
        let known = written.app.as_ref().map(|app| (&app.exe, &app.display_name));
        if let Some(app) = self.app.as_ref().filter(|app| known != Some((&app.exe, &app.display_name))) {
            out.push(InputEvent::App(AppSwitch { qpc, ..app.clone() }));
        }
        out
    }
}

/// The logger's recent events, with the state from before the oldest.
#[derive(Default)]
struct History {
    base: State,
    events: VecDeque<InputEvent>,
}

impl History {
    /// What held just before `qpc`.
    fn state_before(&self, qpc: i64) -> State {
        let mut state = self.base.clone();
        for event in self.events.iter().filter(|event| stamp(event).is_some_and(|at| at < qpc)) {
            state.apply(event);
        }
        state
    }

    /// Folds the events older than `oldest` into the base state.
    fn trim(&mut self, oldest: i64) {
        while self.events.front().and_then(stamp).is_some_and(|at| at < oldest) {
            if let Some(event) = self.events.pop_front() {
                self.base.apply(&event);
            }
        }
    }
}

/// A take's pauses on the QPC clock.
#[derive(Default)]
struct Pauses {
    /// The finished pauses, as QPC ranges.
    cuts: Vec<(i64, i64)>,
    paused_from: Option<i64>,
}

impl Pauses {
    fn pause(&mut self, from_qpc: i64) {
        self.paused_from.get_or_insert(from_qpc);
    }

    /// Ends the running pause at `to_qpc`; false when none is running.
    fn resume(&mut self, to_qpc: i64) -> bool {
        let Some(from) = self.paused_from.take() else { return false };
        match self.cuts.last_mut() {
            // A pause that began before the last one ended (it held no frame) lengthens that one.
            Some(last) if from < last.1 => last.1 = last.1.max(to_qpc),
            _ => self.cuts.push((from, to_qpc.max(from))),
        }
        true
    }
}

/// A take whose first video frame has not arrived, so it has no file yet.
struct WaitingTake {
    path: PathBuf,
    /// The QPC time the take began at: its first frame is not before it, and the history keeps everything from here on.
    since_qpc: i64,
    pauses: Pauses,
    /// True when the logger had ended before the take began, which leaves it no events.
    dead_at_start: bool,
}

/// The events file of the running take.
struct TakeTrack {
    /// None once writing has failed or the logger has ended; the file then stays without its tail.
    writer: Option<EventsWriter<File>>,
    first_qpc: i64,
    pauses: Pauses,
    /// The QPC time at which the state must be stated again before any later event.
    catch_up: Option<i64>,
    /// What a reader of the file knows so far.
    written: State,
    flushed: Instant,
}

impl TakeTrack {
    /// The time removed by the pauses that ended at or before `qpc`.
    fn removed(&self, qpc: i64) -> i64 {
        self.pauses.cuts.iter().filter(|(_, to)| qpc >= *to).map(|(from, to)| to - from).sum()
    }

    /// The video time of a QPC stamp; None before the first frame and inside a pause.
    fn map(&self, qpc: i64) -> Option<i64> {
        let paused = self.pauses.paused_from.is_some_and(|from| qpc >= from) || self.pauses.cuts.iter().any(|(from, to)| qpc >= *from && qpc < *to);
        (qpc >= self.first_qpc && !paused).then(|| qpc - self.removed(qpc))
    }

    /// Runs a writer call; the first error ends the file and is returned as a notice.
    fn write(&mut self, call: impl FnOnce(&mut EventsWriter<File>) -> Result<(), EventsError>) -> Option<String> {
        let err = call(self.writer.as_mut()?).err()?;
        self.writer = None;
        log::warn!("the events file could not be written: {err}");
        Some(format!("The input track could not be written: {err}"))
    }

    fn push(&mut self, event: InputEvent) -> Option<String> {
        self.written.apply(&event);
        self.write(|writer| writer.push(event))
    }

    fn state_again(&mut self, state: &State, at: i64) -> Option<String> {
        self.catch_up = None;
        let mapped = self.map(at)?;
        let events = state.restate(&self.written, mapped);
        events.into_iter().fold(None, |notice, event| self.push(event).or(notice))
    }

    /// Takes one timed event. `history` must hold everything before it.
    fn feed(&mut self, event: &InputEvent, history: &History) -> Option<String> {
        let at = stamp(event)?;
        let mut notice = None;
        if let Some(from) = self.catch_up.filter(|from| at >= *from) {
            notice = self.state_again(&history.state_before(from), from);
        }
        match self.map(at) {
            Some(mapped) => self.push(restamped(event.clone(), mapped)).or(notice),
            None => notice,
        }
    }

    /// Goes on from `from`, taking in what the history already has from there on. The state is stated again at `from`
    /// and wherever a later pause ended.
    fn go_on_from(&mut self, from: i64, history: &History) -> Option<String> {
        let ends = self.pauses.cuts.iter().map(|(_, to)| *to).filter(|to| *to > from);
        let mut points: VecDeque<i64> = std::iter::once(from).chain(ends).collect();
        let mut notice = None;
        for event in &history.events {
            let Some(at) = stamp(event).filter(|at| *at >= from) else { continue };
            while let Some(point) = points.front().copied().filter(|point| at >= *point) {
                self.catch_up = Some(point);
                points.pop_front();
            }
            notice = self.feed(event, history).or(notice);
        }
        // A point no event has reached yet waits for the next one, or for the take's end.
        if let Some(point) = points.back() {
            self.catch_up = Some(*point);
        }
        notice
    }

    /// Writes what is pending and leaves the file without its tail, which marks it incomplete.
    fn abandon(&mut self) {
        let _ = self.write(EventsWriter::flush_second);
        self.writer = None;
    }
}

/// A saved replay's part of the input track, ready to be written.
pub(crate) struct EventsClip {
    header: Header,
    events: Vec<InputEvent>,
    end_qpc: i64,
    /// False when the logger had ended, so the file gets no tail.
    complete: bool,
}

impl EventsClip {
    /// Writes the clip as a new events file.
    pub(crate) fn write(self, path: &Path) -> Result<(), EventsError> {
        let mut writer = EventsWriter::create(path, &self.header)?;
        for event in self.events {
            writer.push(event)?;
        }
        if self.complete {
            writer.finish(self.end_qpc)?;
        } else {
            writer.flush_second()?;
        }
        Ok(())
    }
}

/// The routing of the logger's events, behind the input track's lock.
pub(crate) struct Core {
    source: Box<dyn InputSource>,
    info: SourceInfo,
    streams: Streams,
    /// Every shape so far; the logger sends each one only once.
    shapes: BTreeMap<u32, CursorShape>,
    history: History,
    newest_qpc: i64,
    /// The exe of the app last known to be in front.
    front_exe: Option<String>,
    /// How long the history reaches back.
    keep: Duration,
    /// A take before its first video frame; never set together with `take`.
    waiting: Option<WaitingTake>,
    take: Option<TakeTrack>,
    dead: bool,
    notices: Vec<String>,
}

impl Core {
    pub(crate) fn new(source: Box<dyn InputSource>, info: SourceInfo, streams: Streams) -> Self {
        let history = History { base: State { displays: info.displays.clone(), app: info.app.clone(), ..State::default() }, events: VecDeque::new() };
        let front_exe = info.app.as_ref().map(|app| app.exe.clone());
        let (shapes, notices) = (BTreeMap::new(), Vec::new());
        Self { source, info, streams, shapes, history, newest_qpc: 0, front_exe, keep: HISTORY_MARGIN, waiting: None, take: None, dead: false, notices }
    }

    /// Takes in everything the logger has sent, and notices when it has ended.
    pub(crate) fn pump(&mut self) {
        if self.dead {
            return;
        }
        // Read before draining, so nothing a logger sent before it ended is left behind.
        let running = self.source.is_running();
        while let Some(event) = self.source.try_next() {
            self.accept(event);
        }
        if !running {
            self.dead = true;
            log::warn!("the input logger ended on its own");
            if let Some(take) = &mut self.take {
                take.abandon();
            }
            self.notices.push(DIED.to_owned());
        }
    }

    fn accept(&mut self, event: InputEvent) {
        // The logger does not know what was in front when it started, so its first report may name that app again.
        if let InputEvent::App(app) = &event
            && self.front_exe.replace(app.exe.clone()).as_ref() == Some(&app.exe)
        {
            return;
        }
        let notice = match (&event, &mut self.take) {
            // A shape has no time: it goes in even during a pause, as a later sample may use it.
            (InputEvent::Shape(_), Some(take)) => take.write(|writer| writer.push(event.clone())),
            (_, Some(take)) => take.feed(&event, &self.history),
            (_, None) => None,
        };
        self.notices.extend(notice);
        if let InputEvent::Shape(shape) = event {
            self.shapes.insert(shape.id, shape);
        } else if let Some(at) = stamp(&event) {
            self.newest_qpc = self.newest_qpc.max(at);
            self.history.events.push_back(event);
            let keep = i128::from(self.info.qpc_frequency) * self.keep.as_millis() as i128 / 1000;
            let oldest = self.newest_qpc.saturating_sub(i64::try_from(keep).unwrap_or(i64::MAX));
            // A take that waits for its first frame has its events held here, however long that takes.
            self.history.trim(self.waiting.as_ref().map_or(oldest, |waiting| oldest.min(waiting.since_qpc)));
        }
    }

    /// Sets how far back a replay save can reach; None while the buffer is off.
    pub(crate) fn set_replay(&mut self, length: Option<Duration>) {
        self.keep = length.unwrap_or_default() + HISTORY_MARGIN;
    }

    fn header(&self, first_qpc: i64, displays: &[DisplayInfo]) -> Header {
        Header {
            qpc_frequency: self.info.qpc_frequency,
            first_frame_qpc: first_qpc,
            source: self.info.capture,
            displays: displays.to_vec(),
            cursor_size: self.info.cursor_size,
            streams: self.streams,
        }
    }

    /// Takes on a take that began at `since_qpc`. Its events are held until [`Core::first_frame`] creates its file.
    pub(crate) fn begin_take(&mut self, path: &Path, since_qpc: i64) {
        self.pump();
        if self.dead {
            self.notices.push(ALREADY_DEAD.to_owned());
        }
        self.waiting = Some(WaitingTake { path: path.to_owned(), since_qpc, pauses: Pauses::default(), dead_at_start: self.dead });
    }

    /// Creates the waiting take's events file, now that its first video frame, at `first_qpc`, has arrived, and writes
    /// the held events. Does nothing when no take waits.
    pub(crate) fn first_frame(&mut self, first_qpc: i64) {
        // Drained while the take still waits, so the history keeps all of it.
        self.pump();
        let Some(WaitingTake { path, mut pauses, dead_at_start, .. }) = self.waiting.take() else { return };
        // A pause that ended before the first frame took nothing out of the video.
        pauses.cuts.retain(|(_, to)| *to > first_qpc);
        let displays = self.history.state_before(first_qpc).displays;
        let writer = match EventsWriter::create(&path, &self.header(first_qpc, &displays)) {
            Ok(writer) => writer,
            Err(err) => {
                log::warn!("the events file {} could not be created: {err}", path.display());
                self.notices.push(format!("The input track could not be written: {err}"));
                return;
            }
        };
        let written = State { displays, ..State::default() };
        let mut take = TakeTrack { writer: Some(writer), first_qpc, pauses, catch_up: None, written, flushed: Instant::now() };
        let mut notice = None;
        for shape in self.shapes.values() {
            notice = take.write(|writer| writer.push(InputEvent::Shape(shape.clone()))).or(notice);
        }
        if !dead_at_start {
            notice = take.go_on_from(first_qpc, &self.history).or(notice);
        }
        if self.dead {
            take.abandon();
        }
        self.notices.extend(notice);
        self.take = Some(take);
    }

    /// Pauses the take's file from the QPC time the video is cut at.
    pub(crate) fn pause(&mut self, from_qpc: i64) {
        if let Some(waiting) = &mut self.waiting {
            waiting.pauses.pause(from_qpc);
        } else if let Some(take) = &mut self.take {
            take.pauses.pause(from_qpc);
        }
    }

    /// Goes on at the QPC time of the first frame after the pause.
    pub(crate) fn resume(&mut self, to_qpc: i64) {
        if let Some(waiting) = &mut self.waiting {
            waiting.pauses.resume(to_qpc);
        } else if let Some(take) = &mut self.take
            && take.pauses.resume(to_qpc)
        {
            let notice = take.go_on_from(to_qpc, &self.history);
            self.notices.extend(notice);
        }
    }

    /// Ends the take's file at `end_qpc`. A file whose logger ended gets no tail. A take whose first frame never
    /// arrived gets no file.
    pub(crate) fn finish_take(&mut self, end_qpc: i64) {
        self.pump();
        if let Some(waiting) = self.waiting.take() {
            log::warn!("the take had no video frame, so {} was not written", waiting.path.display());
        }
        let Some(mut take) = self.take.take() else { return };
        if let Some(from) = take.catch_up.filter(|from| *from <= end_qpc) {
            let notice = take.state_again(&self.history.state_before(from), from);
            self.notices.extend(notice);
        }
        let end = take.pauses.paused_from.map_or(end_qpc, |from| from.min(end_qpc));
        let end = end - take.removed(end);
        if let Some(writer) = take.writer.take()
            && let Err(err) = writer.finish(end)
        {
            log::warn!("the events file could not be finished: {err}");
            self.notices.push(format!("The input track could not be written: {err}"));
        }
    }

    /// Writes the take's pending events once a second.
    fn flush_if_due(&mut self) {
        let Some(take) = &mut self.take else { return };
        if take.flushed.elapsed() < FLUSH_EVERY {
            return;
        }
        take.flushed = Instant::now();
        let notice = take.write(EventsWriter::flush_second);
        self.notices.extend(notice);
    }

    /// The history from `from_qpc` up to `to_qpc` as a file of its own.
    pub(crate) fn clip(&mut self, from_qpc: i64, to_qpc: i64) -> EventsClip {
        self.pump();
        let state = self.history.state_before(from_qpc);
        let header = self.header(from_qpc, &state.displays);
        let mut timed = state.restate(&State { displays: state.displays.clone(), ..State::default() }, from_qpc);
        timed.extend(self.history.events.iter().filter(|event| stamp(event).is_some_and(|at| at >= from_qpc && at < to_qpc)).cloned());
        let used: BTreeSet<u32> = timed.iter().filter_map(|event| if let InputEvent::Sample(sample) = event { sample.shape } else { None }).collect();
        let mut events: Vec<InputEvent> = used.iter().filter_map(|id| self.shapes.get(id).cloned()).map(InputEvent::Shape).collect();
        events.append(&mut timed);
        EventsClip { header, events, end_qpc: to_qpc, complete: !self.dead }
    }

    /// The toasts raised since the last call.
    pub(crate) fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }
}

fn lock(core: &Mutex<Core>) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A capture's input logger and the thread that routes and flushes its events. Dropping it ends both.
pub(crate) struct InputTrack {
    core: Arc<Mutex<Core>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl InputTrack {
    /// Starts the "input-track" thread.
    pub(crate) fn start(core: Core) -> std::io::Result<Self> {
        let core = Arc::new(Mutex::new(core));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_core, thread_stop) = (Arc::clone(&core), Arc::clone(&stop));
        let thread = std::thread::Builder::new().name("input-track".to_owned()).spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                {
                    let mut core = lock(&thread_core);
                    core.pump();
                    core.flush_if_due();
                }
                std::thread::sleep(POLL);
            }
        })?;
        Ok(Self { core, stop, thread: Some(thread) })
    }

    /// Starts the Windows input logger for a monitor capture with the profile's streams.
    pub(crate) fn start_logger(monitor: usize, profile: &Profile) -> Result<Self, String> {
        let monitors = capture::list_monitors().map_err(|err| err.to_string())?;
        let monitor = monitors.get(monitor).ok_or_else(|| "the captured monitor was not found".to_owned())?;
        let rect = capture::ScreenRect { x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height };
        let streams = streams(profile);
        let options = capture::InputOptions { source: capture::CaptureSource::Monitor(rect), clicks: streams.clicks, keys: streams.keys, apps: streams.apps };
        let logger = capture::InputLogger::start(options).map_err(|err| err.to_string())?;
        let start = logger.start_info();
        let info = SourceInfo {
            qpc_frequency: start.qpc_frequency,
            cursor_size: start.cursor_size,
            displays: start.displays.iter().map(display).collect(),
            capture: CaptureSource::Monitor(screen_rect(rect)),
            // Read after the logger's start, so no switch can fall between the two.
            app: if streams.apps { foreground_app() } else { None },
        };
        Self::start(Core::new(Box::new(Logger(logger)), info, streams)).map_err(|err| err.to_string())
    }

    /// The routing, for the engine's calls.
    pub(crate) fn core(&self) -> MutexGuard<'_, Core> {
        lock(&self.core)
    }
}

impl Drop for InputTrack {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The app whose window is in front right now, named as the logger names apps; None when it cannot be told. Only the
/// exe name and the display name are read, never a window title.
fn foreground_app() -> Option<AppSwitch> {
    let mut pid = 0u32;
    // SAFETY: both calls accept any window, also none, and pid is a local out parameter.
    #[allow(unsafe_code)]
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid))
    };
    if pid == 0 {
        return None;
    }
    // The app list leaves out this process.
    if pid == std::process::id() {
        let exe = std::env::current_exe().ok()?.file_name()?.to_string_lossy().to_lowercase();
        return Some(AppSwitch { qpc: 0, exe, display_name: "CutMark Recorder".to_owned() });
    }
    let apps = capture::list_audio_apps(true).map_err(|err| log::warn!("the app in front could not be read: {err}")).ok()?;
    let app = apps.into_iter().find(|app| app.pids.contains(&pid))?;
    Some(AppSwitch { qpc: 0, exe: app.exe, display_name: app.display_name })
}

/// The capture crate's logger as a source of the events crate's records.
struct Logger(capture::InputLogger);

impl InputSource for Logger {
    fn try_next(&mut self) -> Option<InputEvent> {
        self.0.receiver().try_recv().ok().map(convert)
    }

    fn is_running(&self) -> bool {
        self.0.is_running()
    }
}

fn screen_rect(rect: capture::ScreenRect) -> ScreenRect {
    ScreenRect { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
}

fn display(info: &capture::DisplayInfo) -> DisplayInfo {
    DisplayInfo { name: info.name.clone(), rect: screen_rect(info.rect), dpi: info.dpi, primary: info.primary }
}

fn button(button: capture::MouseButton) -> events::MouseButton {
    match button {
        capture::MouseButton::Left => events::MouseButton::Left,
        capture::MouseButton::Right => events::MouseButton::Right,
        capture::MouseButton::Middle => events::MouseButton::Middle,
        capture::MouseButton::X1 => events::MouseButton::X1,
        capture::MouseButton::X2 => events::MouseButton::X2,
    }
}

/// The capture crate's record as the events crate's.
fn convert(event: capture::InputEvent) -> InputEvent {
    match event {
        capture::InputEvent::Sample(s) => InputEvent::Sample(CursorSample { qpc: s.qpc, x: s.x, y: s.y, shape: s.shape, visible: s.visible }),
        capture::InputEvent::Shape(s) => InputEvent::Shape(CursorShape {
            id: s.id,
            width: s.width,
            height: s.height,
            hotspot_x: s.hotspot_x,
            hotspot_y: s.hotspot_y,
            frames: s.frames.into_iter().map(|frame| events::ShapeFrame { duration_us: frame.duration_us, bgra: frame.bgra }).collect(),
        }),
        capture::InputEvent::Mouse(m) => InputEvent::Mouse(events::MouseEvent {
            qpc: m.qpc,
            x: m.x,
            y: m.y,
            action: match m.action {
                capture::MouseAction::Down(b) => events::MouseAction::Down(button(b)),
                capture::MouseAction::Up(b) => events::MouseAction::Up(button(b)),
                capture::MouseAction::Wheel(delta) => events::MouseAction::Wheel(delta),
                capture::MouseAction::HorizontalWheel(delta) => events::MouseAction::HorizontalWheel(delta),
            },
        }),
        capture::InputEvent::Geometry(g) => InputEvent::Geometry(GeometryEvent {
            qpc: g.qpc,
            change: match g.change {
                capture::GeometryChange::WindowRect(rect) => GeometryChange::WindowRect(screen_rect(rect)),
                capture::GeometryChange::Displays(displays) => GeometryChange::Displays(displays.iter().map(display).collect()),
            },
        }),
        capture::InputEvent::Chord { qpc, modifiers: m, key_name } => {
            InputEvent::Chord(events::Chord { qpc, modifiers: events::Modifiers { ctrl: m.ctrl, alt: m.alt, shift: m.shift, win: m.win }, key_name })
        }
        capture::InputEvent::App { qpc, exe, display_name } => InputEvent::App(AppSwitch { qpc, exe, display_name }),
    }
}

#[cfg(test)]
#[path = "input_track_tests.rs"]
mod tests;
