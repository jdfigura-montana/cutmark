//! The Layout tool's Screen group: what a recording does with the mouse cursor, and the Tutorial
//! capture preset (input track spec, "Settings").

use egui::{RichText, Ui};
use ui_kit::tokens::space;
use ui_kit::{inspector_group, inspector_row, switch};

use crate::config::{CursorMode, Profile, RateControl};
use crate::layout_ui::choice;
use crate::profile_ui::{AmfProbe, Control, Lock};
use crate::profiles::ProfileStore;

/// The choices of the Cursor control.
const CURSOR_MODES: [(CursorMode, &str); 3] = [(CursorMode::Show, "Show"), (CursorMode::Hide, "Hide"), (CursorMode::Track, "Track (draw it later)")];

/// The caption under the Cursor control.
const CURSOR_CAPTION: &str = "With Track, the video has no cursor. Review and Bake draw the cursor back.";

/// The frame rate the Tutorial preset asks for, and the one it falls back to.
const TUTORIAL_FPS: u32 = 60;
const TUTORIAL_FALLBACK_FPS: u32 = 30;

/// How much one quality step adds to a constant quality value.
const QUALITY_STEP: u8 = 10;

/// What a rate control without a bitrate ceiling is assumed to take, in megabits per second.
const UNCAPPED_MBPS: f32 = 40.0;

/// The bitrate one AAC stream takes, in megabits per second.
const AUDIO_MBPS: f32 = 0.32;

/// What the Screen group keeps between frames.
#[derive(Default)]
pub struct ScreenGroup {
    probe: AmfProbe,
    /// The profile the preset was pressed for and the native size it asked the encoder about.
    pending: Option<(String, (u32, u32))>,
    /// The profile and the note about the frame rate the preset fell back to.
    note: Option<(String, String)>,
}

/// Whether the clicks, keyboard shortcuts and app switches switches are shown.
fn switches_shown(mode: CursorMode) -> bool {
    mode == CursorMode::Track
}

/// The profile default's rate control with its quality one step up.
fn tutorial_rate(current: RateControl) -> RateControl {
    let max = match current {
        RateControl::ConstantQuality { max_mbps, .. } => Some(max_mbps),
        _ => None,
    };
    match Profile::tutorial_av1_1440p60().rate_control {
        RateControl::ConstantQuality { quality, max_mbps } => {
            RateControl::ConstantQuality { quality: quality.saturating_add(QUALITY_STEP).min(100), max_mbps: max.unwrap_or(max_mbps) }
        }
        other => other,
    }
}

/// Applies the Tutorial capture preset at `fps`.
fn apply_tutorial(profile: &mut Profile, fps: u32) {
    profile.width = 0;
    profile.height = 0;
    profile.fps = fps;
    profile.rate_control = tutorial_rate(profile.rate_control);
    profile.cursor = CursorMode::Track;
    profile.record_clicks = true;
}

/// The size an hour of recording is expected to reach, in GB. With a constant quality it is the ceiling.
fn gb_per_hour(profile: &Profile) -> f32 {
    let mbps = |rate: RateControl| match rate {
        RateControl::ConstantQuality { max_mbps, .. } => max_mbps,
        RateControl::ConstantQp { .. } => UNCAPPED_MBPS,
        RateControl::Cbr { mbps } => mbps,
        RateControl::VbrPeak { target_mbps, .. } => target_mbps,
    };
    let camera = profile.webcam.as_ref().filter(|webcam| webcam.iso).map_or(0.0, |webcam| mbps(webcam.iso_rc));
    (mbps(profile.rate_control) + camera + AUDIO_MBPS) * 3600.0 / 8.0 / 1000.0
}

fn size_text(profile: &Profile) -> String {
    let lead = if matches!(profile.rate_control, RateControl::Cbr { .. } | RateControl::VbrPeak { .. }) { "About" } else { "Up to about" };
    format!("{lead} {:.0} GB per hour", gb_per_hour(profile))
}

/// The captured monitor's size; the Recorder captures the first monitor.
fn native_size() -> Option<(u32, u32)> {
    let monitors = capture::list_monitors().map_err(|err| log::warn!("the monitor size could not be read: {err}")).ok()?;
    let monitor = monitors.first()?;
    Some((u32::try_from(monitor.width).ok()?, u32::try_from(monitor.height).ok()?))
}

impl ScreenGroup {
    /// Applies a pressed preset once the encoder probe has answered for the native size at 60 fps.
    fn poll(&mut self, ctx: &egui::Context, store: &mut ProfileStore) {
        let Some((name, (width, height))) = self.pending.clone() else { return };
        let key = (width, height, TUTORIAL_FPS);
        self.probe.update(ctx, key);
        let Some(codec) = store.get(&name).map(|profile| profile.codec) else {
            self.pending = None;
            return;
        };
        let Some(possible) = self.probe.works(key, codec.into()) else { return };
        let fps = if possible { TUTORIAL_FPS } else { TUTORIAL_FALLBACK_FPS };
        store.edit(&name, |profile| apply_tutorial(profile, fps));
        self.note = (!possible).then(|| {
            let note = format!("The encoder cannot record {width}x{height} at {TUTORIAL_FPS} fps, so the preset uses {TUTORIAL_FALLBACK_FPS} fps.");
            (name, note)
        });
        self.pending = None;
    }

    /// Draws the group for the active profile; `lock` makes it read-only.
    pub fn show(&mut self, ui: &mut Ui, store: &mut ProfileStore, profile_name: &str, lock: Lock) {
        if !lock.locks(Control::Profile) {
            self.poll(ui.ctx(), store);
        }
        let Some(profile) = store.get(profile_name) else { return };
        let before = (profile.cursor, profile.record_clicks, profile.record_keys, profile.record_apps);
        let size = size_text(profile);
        let (mut cursor, mut clicks, mut keys, mut apps) = before;
        let mut preset = false;
        let busy = self.pending.is_some();
        inspector_group(ui, |ui| {
            lock.scope(ui, Control::Profile, true, |ui| {
                ui.label("Cursor");
                choice(ui, "screen-cursor", &mut cursor, &CURSOR_MODES);
                ui.label(RichText::new(CURSOR_CAPTION).small().weak());
                if switches_shown(cursor) {
                    for (label, value) in [("Record clicks", &mut clicks), ("Record keyboard shortcuts", &mut keys), ("Record app switches", &mut apps)] {
                        inspector_row(
                            ui,
                            label,
                            |ui| {
                                switch(ui, value);
                            },
                            None,
                        );
                    }
                }
                ui.add_space(space::SCALE_4);
                preset = ui.add_enabled(!busy, egui::Button::new("Tutorial preset")).clicked();
                ui.label(RichText::new(size).small().weak());
                if busy {
                    ui.label(RichText::new("Checking the encoder...").small().weak());
                } else if let Some((_, note)) = self.note.as_ref().filter(|(name, _)| name == profile_name) {
                    ui.label(RichText::new(note).small().weak());
                }
            });
        });
        if (cursor, clicks, keys, apps) != before {
            store.edit(profile_name, |profile| {
                profile.cursor = cursor;
                profile.record_clicks = clicks;
                profile.record_keys = keys;
                profile.record_apps = apps;
            });
        }
        if preset {
            self.note = None;
            match native_size() {
                Some(size) => self.pending = Some((profile_name.to_owned(), size)),
                None => self.note = Some((profile_name.to_owned(), "The monitor size could not be read, so nothing was changed.".to_owned())),
            }
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_switches_show_only_with_track() {
        assert!(switches_shown(CursorMode::Track));
        assert!(!switches_shown(CursorMode::Show));
        assert!(!switches_shown(CursorMode::Hide));
    }

    #[test]
    fn the_screen_settings_lock_while_recording_and_while_the_replay_buffer_is_on() {
        for (recording, replay_on) in [(true, false), (false, true)] {
            let lock = Lock { recording, replay_on };
            assert!(lock.locks(Control::Profile));
        }
        assert!(!Lock { recording: false, replay_on: false }.locks(Control::Profile));
    }

    #[test]
    fn the_tutorial_preset_tracks_the_cursor_at_native_size_and_does_not_climb() {
        let mut profile = Profile { width: 1920, height: 1080, fps: 30, record_clicks: false, ..Profile::premiere_hevc_1080p60() };
        apply_tutorial(&mut profile, 60);
        assert_eq!((profile.width, profile.height, profile.fps), (0, 0, 60));
        assert_eq!((profile.cursor, profile.record_clicks, profile.record_keys), (CursorMode::Track, true, false));
        let once = profile.rate_control;
        apply_tutorial(&mut profile, 60);
        assert_eq!(profile.rate_control, once);
    }

    #[test]
    fn the_hourly_size_follows_the_bitrate() {
        let profile = Profile { rate_control: RateControl::Cbr { mbps: 8.0 }, webcam: None, ..Profile::tutorial_av1_1440p60() };
        assert_eq!(size_text(&profile), "About 4 GB per hour");
    }
}
