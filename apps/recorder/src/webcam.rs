//! The webcam of the active profile: its settings, the open camera, its preview and its controls.
//!
//! One [`WebcamCapture`] is open for as long as the active profile has the webcam on, idle or recording.
//! It reopens when the camera or the format changes, and the saved controls are applied each time it opens.
//! A camera that is missing is a warning and nothing more: recording works without it. The recording
//! session reuses this capture later.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use capture::{CameraFormat, CameraInfo, CameraSubtype, ControlInfo, ControlProp, ControlState, Gpu, Preview, WebcamCapture};

use crate::config::{ControlSetting, WebcamControls, WebcamFormat, WebcamSettings};
use crate::preview_ui::PreviewView;
use crate::profiles::ProfileStore;

/// How often the values of controls that are in auto mode are read back from the camera.
const CONTROL_POLL: Duration = Duration::from_millis(500);

/// How long after a failed open or a lost camera the camera is tried again.
const RETRY_AFTER: Duration = Duration::from_secs(3);

/// The camera and mode a capture was opened with.
type OpenKey = (String, CameraFormat);

/// What a capture thread reports: the open camera or why it did not open.
type OpenResult = Result<WebcamCapture, String>;

/// What the settings and the list of cameras say should be open.
#[derive(Debug, Clone, PartialEq)]
enum Resolution {
    /// Open this camera in this mode.
    Open {
        /// The camera's symbolic link.
        camera_id: String,
        /// The camera's name.
        camera_name: String,
        /// The mode to open it in.
        format: CameraFormat,
    },
    /// There is nothing to open, for this reason.
    Problem(String),
}

/// The webcam of the active profile.
///
/// The webcam settings are part of the profile in the [`ProfileStore`]; the host reads and edits them there
/// and never keeps a copy, so the Profile section and the Webcam section cannot overwrite each other.
pub struct WebcamHost {
    ctx: egui::Context,
    /// The profile whose webcam is shown, or None before the first frame.
    profile: Option<String>,
    /// The settings of a webcam that was switched off, kept so that switching it on again restores them.
    parked: Option<WebcamSettings>,
    /// The cameras found at the last refresh.
    cameras: Vec<CameraInfo>,
    /// The running camera search, or None.
    listing: Option<Receiver<Result<Vec<CameraInfo>, String>>>,
    /// True once a camera search has finished, with or without an error.
    listed: bool,
    /// Why the last camera search failed.
    list_error: Option<String>,
    /// The graphics device of the capture and its preview.
    gpu: Option<Gpu>,
    /// The camera and mode that should be open, or None while nothing should be.
    want: Option<OpenKey>,
    /// The camera that is being opened on another thread.
    opening: Option<(OpenKey, Receiver<OpenResult>)>,
    /// Not before this time may the camera be opened again after a failure.
    retry_at: Option<Instant>,
    /// What the open capture was opened with.
    open_key: Option<OpenKey>,
    /// The live preview of the open capture; declared before the capture so it ends first.
    preview: Option<Preview>,
    /// The open capture.
    capture: Option<Arc<WebcamCapture>>,
    /// The supported controls with the values the window last saw.
    controls: Vec<ControlInfo>,
    /// When the controls in auto mode were last read back.
    last_poll: Instant,
    /// False while the window is minimized or hidden.
    shown: bool,
    /// Why the camera is not running, for the warning banner.
    camera_warning: Option<String>,
    /// Why the preview could not start.
    preview_error: Option<String>,
    /// Draws the preview picture.
    preview_view: PreviewView,
}

impl WebcamHost {
    /// A host with the webcam off, until the first [`sync`](Self::sync) loads the profile.
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            ctx: ctx.clone(),
            profile: None,
            parked: None,
            cameras: Vec::new(),
            listing: None,
            listed: false,
            list_error: None,
            gpu: None,
            want: None,
            opening: None,
            retry_at: None,
            open_key: None,
            preview: None,
            capture: None,
            controls: Vec::new(),
            last_poll: Instant::now(),
            shown: true,
            camera_warning: None,
            preview_error: None,
            preview_view: PreviewView::default(),
        }
    }

    /// Follows the active profile and keeps the capture and the preview in step with its webcam settings; call it once per frame.
    pub fn sync(&mut self, store: &mut ProfileStore, profile: &str, shown: bool) {
        self.shown = shown;
        if self.profile.as_deref() != Some(profile) {
            self.switch_profile(store, profile);
        }
        self.poll_listing();
        self.poll_open(store);
        if self.settings(store).is_none() {
            self.want = None;
            self.camera_warning = None;
            self.release();
            return;
        }
        if !self.listed && self.listing.is_none() {
            self.start_listing();
        }
        self.reconcile(store);
        self.poll_controls();
        if let Some(preview) = &self.preview {
            preview.set_active(shown);
        }
    }

    /// The webcam settings of the active profile in the store, or None while the webcam is off.
    pub fn settings<'a>(&self, store: &'a ProfileStore) -> Option<&'a WebcamSettings> {
        store.get(self.profile.as_deref()?)?.webcam.as_ref()
    }

    /// Changes the webcam settings of the active profile in the store, which schedules the save; does nothing while the webcam is off.
    fn edit(&self, store: &mut ProfileStore, change: impl FnOnce(&mut WebcamSettings)) {
        if let Some(name) = &self.profile {
            store.edit(name, |profile| {
                if let Some(settings) = &mut profile.webcam {
                    change(settings);
                }
            });
        }
    }

    /// The cameras found at the last refresh.
    pub fn cameras(&self) -> &[CameraInfo] {
        &self.cameras
    }

    /// The formats of the selected camera, largest first.
    pub fn formats(&self, store: &ProfileStore) -> &[CameraFormat] {
        let Some(settings) = self.settings(store) else { return &[] };
        self.cameras.iter().find(|camera| camera.id == settings.camera_id).map_or(&[], |camera| camera.formats.as_slice())
    }

    /// Whether the format is the one the profile has saved.
    pub fn is_current_format(&self, store: &ProfileStore, format: &CameraFormat) -> bool {
        self.settings(store).is_some_and(|settings| format_matches(&settings.format, format))
    }

    /// Whether a camera search is running.
    pub fn is_listing(&self) -> bool {
        self.listing.is_some()
    }

    /// The supported camera controls with the values the window last saw.
    pub fn controls(&self) -> &[ControlInfo] {
        &self.controls
    }

    /// The text of the warning banner, if there is something to warn about.
    pub fn warning(&self) -> Option<String> {
        self.camera_warning.clone().or_else(|| self.preview_error.clone()).or_else(|| self.preview.as_ref().and_then(Preview::error))
    }

    /// Switches the webcam on with the settings it had before, or the defaults.
    pub fn enable(&mut self, store: &mut ProfileStore) {
        let Some(name) = &self.profile else { return };
        let settings = self.parked.take().unwrap_or_default();
        store.edit(name, |profile| {
            if profile.webcam.is_none() {
                profile.webcam = Some(settings);
            }
        });
    }

    /// Switches the webcam off; the profile loses its webcam settings, and the window keeps them for switching it on again.
    pub fn disable(&mut self, store: &mut ProfileStore) {
        let Some(name) = &self.profile else { return };
        if let Some(settings) = store.edit(name, |profile| profile.webcam.take()).flatten() {
            self.parked = Some(settings);
        }
    }

    /// Looks for cameras again, and lets a camera that failed to open try again at once.
    pub fn refresh_cameras(&mut self) {
        self.retry_at = None;
        if self.listing.is_none() {
            self.start_listing();
        }
    }

    /// Picks a camera by its symbolic link; its default mode is chosen, and the saved controls are dropped because they belonged to the old camera.
    pub fn select_camera(&mut self, store: &mut ProfileStore, id: &str) {
        let Some(camera) = self.cameras.iter().find(|camera| camera.id == id) else { return };
        let (camera_id, camera_name) = (camera.id.clone(), camera.name.clone());
        let format = pick_default_format(&camera.formats).map(|format| saved_format(&format));
        self.edit(store, |settings| {
            settings.camera_id = camera_id;
            settings.camera_name = camera_name;
            if let Some(format) = format {
                settings.format = format;
            }
            settings.controls = WebcamControls::default();
        });
    }

    /// Picks a mode of the selected camera.
    pub fn select_format(&mut self, store: &mut ProfileStore, format: &CameraFormat) {
        let saved = saved_format(format);
        self.edit(store, |settings| settings.format = saved);
    }

    /// Changes one camera control live and saves it to the profile.
    pub fn set_control(&mut self, store: &mut ProfileStore, prop: ControlProp, setting: ControlSetting) {
        let Some(capture) = &self.capture else { return };
        let controls = capture.controls();
        let value = match setting {
            ControlSetting::Manual(value) => value,
            ControlSetting::Auto => self.controls.iter().find(|info| info.prop == prop).map_or(0, |info| info.current.value),
        };
        if let Err(error) = controls.set(prop, value, setting == ControlSetting::Auto) {
            log::warn!("webcam: could not set {}: {error}", prop.label());
            return;
        }
        let Some(state) = controls.current(prop) else { return };
        if let Some(info) = self.controls.iter_mut().find(|info| info.prop == prop) {
            info.current = state;
        }
        self.edit(store, |settings| settings.controls.set(prop, Some(setting_from_state(&state))));
    }

    /// Switches every control that is in auto mode to manual at its current value.
    pub fn lock_current(&mut self, store: &mut ProfileStore) {
        let Some(capture) = &self.capture else { return };
        let controls = capture.controls();
        controls.lock_current();
        self.controls = controls.supported();
        self.store_controls(store);
    }

    /// Returns every control to the camera's default.
    pub fn reset_defaults(&mut self, store: &mut ProfileStore) {
        let Some(capture) = &self.capture else { return };
        let controls = capture.controls();
        controls.reset_defaults();
        self.controls = controls.supported();
        self.store_controls(store);
    }

    /// Draws the live camera picture, or a message when there is none.
    pub fn show_preview(&mut self, ui: &mut egui::Ui) {
        match (&self.preview, &self.capture) {
            (Some(preview), _) => self.preview_view.show(ui, preview),
            (None, Some(_)) => PreviewView::placeholder(ui, "Camera preview unavailable"),
            (None, None) if self.opening.is_some() || !self.listed => PreviewView::placeholder(ui, "Opening the camera..."),
            (None, None) => PreviewView::placeholder(ui, "No camera"),
        }
    }

    /// Closes the old profile's camera and takes over the webcam of the new one.
    fn switch_profile(&mut self, store: &mut ProfileStore, name: &str) {
        self.release();
        self.controls.clear();
        self.want = None;
        self.retry_at = None;
        self.camera_warning = None;
        self.preview_error = None;
        self.parked = None;
        self.profile = Some(name.to_owned());
        // A hand-edited profile file may hold an offset outside the range the window allows.
        if self.settings(store).is_some_and(|settings| settings.offset_ms != settings.offset_clamped()) {
            self.edit(store, |settings| settings.offset_ms = settings.offset_clamped());
        }
    }

    /// Ends the preview and closes the camera.
    fn release(&mut self) {
        if self.preview.take().is_some() {
            self.preview_view.reset();
        }
        self.capture = None;
        self.open_key = None;
        self.controls.clear();
    }

    /// Starts a camera search on its own thread; opening each camera can take a moment.
    fn start_listing(&mut self) {
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new().name("webcam-list".to_owned()).spawn(move || {
            let result = capture::list_cameras().map_err(|error| error.to_string());
            // A receiver that is gone means the window closed; nothing is left to tell.
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.listing = Some(rx),
            Err(error) => {
                self.listed = true;
                self.list_error = Some(format!("could not start the camera search: {error}"));
            }
        }
    }

    /// Takes in the result of the camera search when it has arrived.
    fn poll_listing(&mut self) {
        let result = match &self.listing {
            Some(receiver) => receiver.try_recv(),
            None => return,
        };
        match result {
            Err(TryRecvError::Empty) => {}
            Ok(Ok(found)) => {
                self.listing = None;
                self.cameras = merge_listing(&self.cameras, found);
                self.listed = true;
                self.list_error = None;
            }
            Ok(Err(message)) => {
                self.listing = None;
                self.listed = true;
                self.list_error = Some(message);
            }
            Err(TryRecvError::Disconnected) => {
                self.listing = None;
                self.listed = true;
                self.list_error = Some("the camera search ended without a result".to_owned());
            }
        }
    }

    /// Opens, keeps or closes the capture so that it matches the settings and the cameras found.
    fn reconcile(&mut self, store: &mut ProfileStore) {
        if !self.listed {
            return;
        }
        if self.cameras.is_empty()
            && let Some(error) = &self.list_error
        {
            self.camera_warning = Some(format!("Could not look for cameras: {error}. Recording works without the camera."));
            self.want = None;
            self.release();
            return;
        }
        let resolution = match self.settings(store) {
            Some(settings) => resolve(&self.cameras, settings, self.open_key.as_ref()),
            None => return,
        };
        let (camera_id, camera_name, format) = match resolution {
            Resolution::Open { camera_id, camera_name, format } => (camera_id, camera_name, format),
            Resolution::Problem(problem) => {
                self.camera_warning = Some(format!("{problem}. Recording works without the camera."));
                self.want = None;
                self.release();
                return;
            }
        };
        self.adopt(store, &camera_id, &camera_name, &format);
        let key: OpenKey = (camera_id, format);
        self.want = Some(key.clone());
        if let Some(capture) = &self.capture
            && self.open_key.as_ref() == Some(&key)
        {
            if let Some(error) = capture.error() {
                self.camera_warning = Some(format!("The camera stopped: {error}. Recording works without it."));
                self.retry_at = Some(Instant::now() + RETRY_AFTER);
                self.release();
            }
            return;
        }
        if self.opening.is_some() {
            return;
        }
        if let Some(at) = self.retry_at {
            let now = Instant::now();
            if now < at {
                self.ctx.request_repaint_after(at - now);
                return;
            }
        }
        self.release();
        self.start_open(key);
    }

    /// Writes the camera and mode that were resolved back into the settings when they differ from what is saved.
    fn adopt(&self, store: &mut ProfileStore, camera_id: &str, camera_name: &str, format: &CameraFormat) {
        let differs = self
            .settings(store)
            .is_some_and(|settings| settings.camera_id != camera_id || settings.camera_name != camera_name || !format_matches(&settings.format, format));
        if differs {
            self.edit(store, |settings| {
                settings.camera_id = camera_id.to_owned();
                settings.camera_name = camera_name.to_owned();
                if !format_matches(&settings.format, format) {
                    settings.format = saved_format(format);
                }
            });
        }
    }

    /// Opens the camera on its own thread, because it can take a second or more.
    fn start_open(&mut self, key: OpenKey) {
        if self.gpu.is_none() {
            match Gpu::create(Some(0)) {
                Ok(gpu) => self.gpu = Some(gpu),
                Err(error) => {
                    self.camera_warning = Some(format!("Could not open the graphics adapter for the camera: {error}"));
                    self.retry_at = Some(Instant::now() + RETRY_AFTER);
                    return;
                }
            }
        }
        let Some(gpu) = self.gpu.clone() else { return };
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        let (id, format) = key.clone();
        let spawned = std::thread::Builder::new().name("webcam-open".to_owned()).spawn(move || {
            let result = WebcamCapture::start(&gpu, &id, &format).map_err(|error| error.to_string());
            // A receiver that is gone drops the capture here, which closes the camera.
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.opening = Some((key, rx)),
            Err(error) => {
                self.camera_warning = Some(format!("Could not start opening the camera: {error}"));
                self.retry_at = Some(Instant::now() + RETRY_AFTER);
            }
        }
    }

    /// Takes in the capture that was being opened, when it is ready.
    fn poll_open(&mut self, store: &ProfileStore) {
        let result = match &self.opening {
            Some((_, receiver)) => receiver.try_recv(),
            None => return,
        };
        if matches!(result, Err(TryRecvError::Empty)) {
            return;
        }
        let Some((key, _)) = self.opening.take() else { return };
        let wanted = self.settings(store).is_some() && self.want.as_ref() == Some(&key);
        match result {
            Ok(Ok(capture)) if wanted => self.install(store, key, capture),
            // The settings changed while the camera opened: the capture is dropped here and the next frame opens the right one.
            Ok(Ok(_)) => {}
            Ok(Err(message)) if wanted => {
                self.camera_warning = Some(format!("Could not open the camera: {message}. Recording works without it."));
                self.retry_at = Some(Instant::now() + RETRY_AFTER);
            }
            Err(TryRecvError::Disconnected) if wanted => {
                self.camera_warning = Some("The camera did not open. Recording works without it.".to_owned());
                self.retry_at = Some(Instant::now() + RETRY_AFTER);
            }
            _ => {}
        }
    }

    /// Makes a freshly opened capture the current one: applies the saved controls and starts its preview.
    fn install(&mut self, store: &ProfileStore, key: OpenKey, capture: WebcamCapture) {
        let capture = Arc::new(capture);
        let controls = capture.controls();
        if let Some(saved) = self.settings(store).map(|settings| settings.controls.clone()) {
            for info in controls.supported() {
                let applied = match saved.get(info.prop) {
                    Some(ControlSetting::Auto) => controls.set(info.prop, info.current.value, true),
                    Some(ControlSetting::Manual(value)) => controls.set(info.prop, value, false),
                    None => Ok(()),
                };
                if let Err(error) = applied {
                    log::warn!("webcam: could not apply the saved {}: {error}", info.prop.label());
                }
            }
        }
        self.controls = controls.supported();
        self.last_poll = Instant::now();
        self.open_key = Some(key);
        self.camera_warning = None;
        self.retry_at = None;
        self.start_preview(&capture);
        self.capture = Some(capture);
    }

    /// Starts the live preview of a capture on the capture's graphics device.
    fn start_preview(&mut self, capture: &Arc<WebcamCapture>) {
        self.preview_error = None;
        self.preview_view.reset();
        let Some(gpu) = &self.gpu else { return };
        match Preview::start_webcam(gpu, Arc::clone(capture)) {
            Ok(preview) => {
                let repaint_ctx = self.ctx.clone();
                preview.set_frame_callback(Box::new(move || repaint_ctx.request_repaint()));
                preview.set_active(self.shown);
                self.preview = Some(preview);
            }
            Err(error) => {
                log::warn!("the camera preview could not start: {error}");
                self.preview_error = Some(format!("Camera preview: {error}"));
            }
        }
    }

    /// Reads the values of the controls that are in auto mode back from the camera, a couple of times a second.
    fn poll_controls(&mut self) {
        if !self.shown || !self.controls.iter().any(|info| info.current.auto) {
            return;
        }
        let Some(capture) = &self.capture else { return };
        let elapsed = self.last_poll.elapsed();
        if elapsed < CONTROL_POLL {
            self.ctx.request_repaint_after(CONTROL_POLL - elapsed);
            return;
        }
        let controls = capture.controls();
        for info in &mut self.controls {
            if info.current.auto
                && let Some(state) = controls.current(info.prop)
            {
                info.current = state;
            }
        }
        self.last_poll = Instant::now();
        self.ctx.request_repaint_after(CONTROL_POLL);
    }

    /// Saves the state of every supported control into the settings.
    fn store_controls(&self, store: &mut ProfileStore) {
        self.edit(store, |settings| {
            for info in &self.controls {
                settings.controls.set(info.prop, Some(setting_from_state(&info.current)));
            }
        });
    }
}

/// The setting that holds a control as it is: auto, or the value it has.
fn setting_from_state(state: &ControlState) -> ControlSetting {
    if state.auto { ControlSetting::Auto } else { ControlSetting::Manual(state.value) }
}

/// The profile's way of writing a camera mode.
fn saved_format(format: &CameraFormat) -> WebcamFormat {
    WebcamFormat { subtype: format.subtype.to_string(), width: format.width, height: format.height, fps: format.fps() as f32 }
}

/// Whether a camera mode is the saved one: the same pixel format and size, and a frame rate within 0.01.
fn format_matches(saved: &WebcamFormat, format: &CameraFormat) -> bool {
    saved.subtype == format.subtype.to_string()
        && saved.width == format.width
        && saved.height == format.height
        && (f64::from(saved.fps) - format.fps()).abs() < 0.01
}

/// The offered mode that is the saved one, if there is one.
fn matching_format(offered: &[CameraFormat], saved: &WebcamFormat) -> Option<CameraFormat> {
    offered.iter().find(|format| format_matches(saved, format)).copied()
}

/// How much a pixel format is liked when two modes have the same size: MJPEG first, because raw modes need far more USB bandwidth.
fn subtype_rank(subtype: CameraSubtype) -> u8 {
    match subtype {
        CameraSubtype::Mjpg => 2,
        CameraSubtype::Nv12 => 1,
        CameraSubtype::Yuy2 | CameraSubtype::Other(_) => 0,
    }
}

/// The mode a camera starts with: the largest one up to 1920x1080 at about 30 fps, or else the first one offered.
fn pick_default_format(formats: &[CameraFormat]) -> Option<CameraFormat> {
    formats
        .iter()
        .filter(|format| format.width <= 1920 && format.height <= 1080 && (29.0..=31.0).contains(&format.fps()))
        .max_by_key(|format| (u64::from(format.width) * u64::from(format.height), subtype_rank(format.subtype)))
        .or_else(|| formats.first())
        .copied()
}

/// The new camera list, where a camera that now lists no formats keeps the formats it had.
///
/// A camera that is open cannot be opened a second time to read its formats, so a refresh reports it with none.
fn merge_listing(old: &[CameraInfo], found: Vec<CameraInfo>) -> Vec<CameraInfo> {
    found
        .into_iter()
        .map(|mut camera| {
            if camera.formats.is_empty()
                && let Some(known) = old.iter().find(|known| known.id == camera.id)
            {
                camera.formats.clone_from(&known.formats);
            }
            camera
        })
        .collect()
}

/// Decides which camera and mode to open for the saved settings and the cameras that were found.
fn resolve(cameras: &[CameraInfo], settings: &WebcamSettings, open: Option<&OpenKey>) -> Resolution {
    let camera = if settings.camera_id.is_empty() {
        cameras.first()
    } else {
        cameras
            .iter()
            .find(|camera| camera.id == settings.camera_id)
            .or_else(|| cameras.iter().find(|camera| !settings.camera_name.is_empty() && camera.name == settings.camera_name))
    };
    let Some(camera) = camera else {
        return Resolution::Problem(if settings.camera_id.is_empty() {
            "No camera was found".to_owned()
        } else if settings.camera_name.is_empty() {
            "The camera was not found".to_owned()
        } else {
            format!("The camera \"{}\" was not found", settings.camera_name)
        });
    };
    let format = if camera.formats.is_empty() {
        match open {
            Some((id, format)) if *id == camera.id => Some(*format),
            _ => None,
        }
    } else {
        matching_format(&camera.formats, &settings.format).or_else(|| pick_default_format(&camera.formats))
    };
    match format {
        Some(format) => Resolution::Open { camera_id: camera.id.clone(), camera_name: camera.name.clone(), format },
        None => Resolution::Problem(format!("The camera \"{}\" offers no formats, another program may be using it", camera.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(subtype: CameraSubtype, width: u32, height: u32, fps_num: u32, fps_den: u32) -> CameraFormat {
        CameraFormat { subtype, width, height, fps_num, fps_den }
    }

    fn camera(id: &str, name: &str, formats: Vec<CameraFormat>) -> CameraInfo {
        CameraInfo { id: id.to_owned(), name: name.to_owned(), formats }
    }

    fn mjpg(width: u32, height: u32, fps: u32) -> CameraFormat {
        format(CameraSubtype::Mjpg, width, height, fps, 1)
    }

    #[test]
    fn the_default_mode_is_the_largest_one_up_to_1080p_at_30_fps() {
        let formats = [
            format(CameraSubtype::Mjpg, 3840, 2160, 30, 1),
            mjpg(1920, 1080, 60),
            format(CameraSubtype::Yuy2, 1920, 1080, 30, 1),
            mjpg(1920, 1080, 30),
            mjpg(1280, 720, 30),
        ];
        assert_eq!(pick_default_format(&formats), Some(mjpg(1920, 1080, 30)));
    }

    #[test]
    fn the_default_mode_falls_back_to_the_first_one_offered() {
        let formats = [mjpg(1280, 720, 60), mjpg(640, 480, 60)];
        assert_eq!(pick_default_format(&formats), Some(mjpg(1280, 720, 60)));
        assert_eq!(pick_default_format(&[]), None);
    }

    #[test]
    fn an_ntsc_rate_counts_as_30_fps() {
        let formats = [mjpg(1280, 720, 60), format(CameraSubtype::Mjpg, 1280, 720, 30000, 1001)];
        assert_eq!(pick_default_format(&formats), Some(format(CameraSubtype::Mjpg, 1280, 720, 30000, 1001)));
    }

    #[test]
    fn a_saved_mode_matches_by_format_size_and_rate() {
        let offered = [mjpg(1920, 1080, 30), format(CameraSubtype::Nv12, 1920, 1080, 30, 1), format(CameraSubtype::Mjpg, 1280, 720, 30000, 1001)];
        let saved = saved_format(&offered[2]);
        assert_eq!(matching_format(&offered, &saved), Some(offered[2]));
        let nv12 = WebcamFormat { subtype: "NV12".to_owned(), width: 1920, height: 1080, fps: 30.0 };
        assert_eq!(matching_format(&offered, &nv12), Some(offered[1]));
        let gone = WebcamFormat { subtype: "MJPG".to_owned(), width: 800, height: 600, fps: 30.0 };
        assert_eq!(matching_format(&offered, &gone), None);
    }

    #[test]
    fn a_refresh_keeps_the_formats_of_a_camera_that_is_open() {
        let old = [camera("a", "Cam A", vec![mjpg(1280, 720, 30)]), camera("b", "Cam B", vec![mjpg(640, 480, 30)])];
        let found = vec![camera("a", "Cam A", Vec::new()), camera("b", "Cam B", vec![mjpg(1920, 1080, 30)]), camera("c", "Cam C", Vec::new())];
        let merged = merge_listing(&old, found);
        assert_eq!(merged[0].formats, vec![mjpg(1280, 720, 30)]);
        assert_eq!(merged[1].formats, vec![mjpg(1920, 1080, 30)]);
        assert!(merged[2].formats.is_empty());
    }

    #[test]
    fn a_new_webcam_takes_the_first_camera_and_its_default_mode() {
        let cameras = [camera("a", "Cam A", vec![mjpg(1920, 1080, 30)]), camera("b", "Cam B", vec![mjpg(640, 480, 30)])];
        let resolved = resolve(&cameras, &WebcamSettings::default(), None);
        assert_eq!(resolved, Resolution::Open { camera_id: "a".to_owned(), camera_name: "Cam A".to_owned(), format: mjpg(1920, 1080, 30) });
    }

    #[test]
    fn a_saved_camera_is_found_by_id_then_by_name() {
        let cameras = [camera("a", "Cam A", vec![mjpg(1280, 720, 30)]), camera("b2", "Cam B", vec![mjpg(1280, 720, 30)])];
        let by_id = WebcamSettings { camera_id: "a".to_owned(), camera_name: "Cam A".to_owned(), ..WebcamSettings::default() };
        assert!(matches!(resolve(&cameras, &by_id, None), Resolution::Open { camera_id, .. } if camera_id == "a"));
        // The same camera on another USB port has another link but the same name.
        let moved = WebcamSettings { camera_id: "b1".to_owned(), camera_name: "Cam B".to_owned(), ..WebcamSettings::default() };
        assert!(matches!(resolve(&cameras, &moved, None), Resolution::Open { camera_id, .. } if camera_id == "b2"));
    }

    #[test]
    fn a_missing_camera_is_a_problem_not_an_error() {
        let cameras = [camera("a", "Cam A", vec![mjpg(1280, 720, 30)])];
        let settings = WebcamSettings { camera_id: "z".to_owned(), camera_name: "Cam Z".to_owned(), ..WebcamSettings::default() };
        assert_eq!(resolve(&cameras, &settings, None), Resolution::Problem("The camera \"Cam Z\" was not found".to_owned()));
        assert_eq!(resolve(&[], &WebcamSettings::default(), None), Resolution::Problem("No camera was found".to_owned()));
    }

    #[test]
    fn a_saved_mode_the_camera_no_longer_offers_gives_way_to_the_default() {
        let cameras = [camera("a", "Cam A", vec![mjpg(1920, 1080, 30), mjpg(640, 480, 30)])];
        let settings = WebcamSettings {
            camera_id: "a".to_owned(),
            format: WebcamFormat { subtype: "MJPG".to_owned(), width: 3840, height: 2160, fps: 30.0 },
            ..WebcamSettings::default()
        };
        assert!(matches!(resolve(&cameras, &settings, None), Resolution::Open { format, .. } if format == mjpg(1920, 1080, 30)));
    }

    #[test]
    fn a_busy_camera_keeps_the_mode_it_is_open_in() {
        let cameras = [camera("a", "Cam A", Vec::new())];
        let settings = WebcamSettings { camera_id: "a".to_owned(), ..WebcamSettings::default() };
        let open: OpenKey = ("a".to_owned(), mjpg(1280, 720, 30));
        assert!(matches!(resolve(&cameras, &settings, Some(&open)), Resolution::Open { format, .. } if format == mjpg(1280, 720, 30)));
        assert!(matches!(resolve(&cameras, &settings, None), Resolution::Problem(_)));
    }

    #[test]
    fn a_control_is_saved_as_auto_or_as_its_value() {
        assert_eq!(setting_from_state(&ControlState { value: 7, auto: true }), ControlSetting::Auto);
        assert_eq!(setting_from_state(&ControlState { value: 7, auto: false }), ControlSetting::Manual(7));
    }
}
