//! help: the app's embedded help topics, the F1 context tags, the first-run
//! tour and the About box's content (spec: S1 Help "Topic files", "Help
//! window", "First-run tour", "About box"). The topic Markdown is embedded at
//! build time, so the app ships no files to read at runtime. This module holds
//! data only; the help window is drawn by `ui_kit::help`.

use ui_kit::AboutInfo;
use ui_kit::help::{HelpBook, HelpError};
use ui_kit::tour::{Tour, TourStep};

/// The topic F1 opens when no area is tagged (the same id ui-kit falls back to).
pub const FALLBACK_TOPIC_ID: &str = "getting-started";

/// The copyright line the About box shows (spec: S1 Help "About box"). JD
/// replaces the holder before release.
const COPYRIGHT: &str = "Copyright \u{a9} 2026 [COPYRIGHT HOLDER]";

/// The generated "Keyboard shortcuts" page: its front matter only. The page
/// body is drawn by `ui_kit::shortcuts::shortcut_sheet` through the help
/// window's `extra` callback, keyed by `ui_kit::help::KEYBOARD_PAGE_ID`
/// (spec: S1 Help "Shortcut sheet").
const KEYBOARD_PAGE: &str =
    "---\nid: keyboard-shortcuts\ntitle: Keyboard shortcuts\nsection: Help\norder: 800\nkeywords: keys, bindings, hotkeys, shortcuts\n---\n";

/// The embedded topic sources: one Markdown file per topic in
/// `apps/audio-editor/help`, plus the generated keyboard page.
pub const SOURCES: &[(&str, &str)] = &[
    ("getting-started.md", include_str!("../help/getting-started.md")),
    ("projects.md", include_str!("../help/projects.md")),
    ("the-window.md", include_str!("../help/the-window.md")),
    ("recording-a-take.md", include_str!("../help/recording-a-take.md")),
    ("choosing-devices.md", include_str!("../help/choosing-devices.md")),
    ("importing-audio.md", include_str!("../help/importing-audio.md")),
    ("working-with-clips.md", include_str!("../help/working-with-clips.md")),
    ("ripple-and-snap.md", include_str!("../help/ripple-and-snap.md")),
    ("markers.md", include_str!("../help/markers.md")),
    ("undo-and-redo.md", include_str!("../help/undo-and-redo.md")),
    ("tracks.md", include_str!("../help/tracks.md")),
    ("playback-and-transport.md", include_str!("../help/playback-and-transport.md")),
    ("zoom-and-navigation.md", include_str!("../help/zoom-and-navigation.md")),
    ("master-and-levels.md", include_str!("../help/master-and-levels.md")),
    ("exporting.md", include_str!("../help/exporting.md")),
    ("appearance.md", include_str!("../help/appearance.md")),
    ("troubleshooting.md", include_str!("../help/troubleshooting.md")),
    ("keyboard-shortcuts.md", KEYBOARD_PAGE),
];

/// Parses and checks the topics. A duplicate id or a link to a missing topic
/// comes back as errors; a build test asserts there are none, so the app never
/// has to handle a broken book.
pub fn try_book() -> Result<HelpBook, Vec<HelpError>> {
    HelpBook::from_sources(SOURCES)
}

/// The book the app draws. A broken book (which a test forbids) yields an empty
/// book rather than a panic.
pub fn book() -> HelpBook {
    try_book().unwrap_or_default()
}
/// The F1 context tag for the timeline panel (its lanes and ruler).
pub const CONTEXT_TIMELINE: &str = "timeline";
/// The F1 context tag for the timeline's track header column.
pub const CONTEXT_TRACKS: &str = "timeline.tracks";
/// The F1 context tag for the transport: the bottom bar and the Play button.
pub const CONTEXT_TRANSPORT: &str = "timeline.transport";
/// The F1 context tag for the media panel.
pub const CONTEXT_MEDIA: &str = "media";
/// The F1 context tag for the master panel.
pub const CONTEXT_MASTER: &str = "master";
/// The F1 context tag for the start screen.
pub const CONTEXT_START: &str = "start";
/// The F1 context tag for the New Project dialog.
pub const CONTEXT_DIALOG_NEW_PROJECT: &str = "dialog.new_project";
/// The F1 context tag for the Save As dialog.
pub const CONTEXT_DIALOG_SAVE_AS: &str = "dialog.save_as";
/// The F1 context tag for the Project Settings dialog.
pub const CONTEXT_DIALOG_PROJECT_SETTINGS: &str = "dialog.project_settings";
/// The F1 context tag for the Audio Settings dialog.
pub const CONTEXT_DIALOG_AUDIO_SETTINGS: &str = "dialog.audio_settings";
/// The F1 context tag for the Export dialog.
pub const CONTEXT_DIALOG_EXPORT: &str = "dialog.export";
/// The F1 context tag for the Gain dialog.
pub const CONTEXT_DIALOG_GAIN: &str = "dialog.gain";
/// The F1 context tag for the Normalize dialog.
pub const CONTEXT_DIALOG_NORMALIZE: &str = "dialog.normalize";

/// Every F1 context tag the app registers, gathered from the same constants the
/// code uses. Only the topic-coverage test reads it (spec: S1 Help "Topic
/// files").
#[cfg(test)]
pub const CONTEXT_TAGS: &[&str] = &[
    CONTEXT_TIMELINE,
    CONTEXT_TRACKS,
    CONTEXT_TRANSPORT,
    CONTEXT_MEDIA,
    CONTEXT_MASTER,
    CONTEXT_START,
    CONTEXT_DIALOG_NEW_PROJECT,
    CONTEXT_DIALOG_SAVE_AS,
    CONTEXT_DIALOG_PROJECT_SETTINGS,
    CONTEXT_DIALOG_AUDIO_SETTINGS,
    CONTEXT_DIALOG_EXPORT,
    CONTEXT_DIALOG_GAIN,
    CONTEXT_DIALOG_NORMALIZE,
];

/// The tour anchor id for the media panel.
pub const ANCHOR_MEDIA: &str = "media-panel";
/// The tour anchor id for the track header column.
pub const ANCHOR_TRACKS: &str = "track-headers";
/// The tour anchor id for the bottom bar and the Play button.
pub const ANCHOR_TRANSPORT: &str = "transport";
/// The tour anchor id for the timeline panel toolbar.
pub const ANCHOR_TIMELINE_TOOLBAR: &str = "timeline-toolbar";
/// The tour anchor id for the title bar's menu bar.
pub const ANCHOR_MENU_BAR: &str = "menu-bar";

/// The 5 tour steps (spec: S1 Help "First-run tour").
const TOUR_STEPS: [TourStep; 5] = [
    TourStep { target: ANCHOR_MEDIA, title: "Media panel", body: "Import audio files here, then drag them onto a track." },
    TourStep { target: ANCHOR_TRACKS, title: "Tracks", body: "Each track has Mute, Solo and Record arm. Arm a track, then press Record to record onto it." },
    TourStep { target: ANCHOR_TRANSPORT, title: "Transport", body: "Play with the round button. Stop and Record are in the bar under the timeline." },
    TourStep {
        target: ANCHOR_TIMELINE_TOOLBAR,
        title: "Timeline toolbar",
        body: "Undo, Redo, Ripple and Snap on the left; waveform height and zoom on the right.",
    },
    TourStep { target: ANCHOR_MENU_BAR, title: "Menu bar", body: "Everything else lives in the menus. Press F1 any time for help on what you are looking at." },
];

/// The app's first-run tour: shown once per profile, version 1 (spec: S1 Help
/// "First-run tour").
pub const TOUR: Tour = Tour { app: "audio-editor", id: "intro", version: 1, steps: &TOUR_STEPS };

/// The third-party notices, then the Instrument Sans and Martian Mono font licenses the
/// app ships (spec: S1 Help "About box"). The two font OFL texts are the ones
/// ui-kit embeds, since the app draws with those faces.
const NOTICES: &str = concat!(
    include_str!("../THIRD-PARTY-NOTICES.txt"),
    "\n\nInstrument Sans (SIL Open Font License 1.1)\n\n",
    include_str!("../../../crates/ui-kit/assets/fonts/OFL-InstrumentSans.txt"),
    "\n\nMartian Mono (SIL Open Font License 1.1)\n\n",
    include_str!("../../../crates/ui-kit/assets/fonts/OFL-MartianMono.txt"),
);

/// The About box's content (spec: S1 Help "About box").
pub const ABOUT: AboutInfo =
    AboutInfo { app_name: "CutMark Audio", brand: Some(ui_kit::brand::App::Audio), version: env!("CARGO_PKG_VERSION"), copyright: COPYRIGHT, notices: NOTICES };

#[cfg(test)]
mod tests {
    use super::*;
    use ui_kit::help::KEYBOARD_PAGE_ID;

    /// The topics build into a valid book: one topic per source, no duplicate
    /// ids and no link to a missing topic.
    #[test]
    fn the_topics_form_a_valid_book() {
        let book = try_book().expect("the topics are valid");
        assert_eq!(book.len(), SOURCES.len(), "one topic per source");
        for id in ["getting-started", "projects", "tracks", "exporting", "appearance", KEYBOARD_PAGE_ID] {
            assert!(book.get(id).is_some(), "the {id} topic must exist");
        }
    }

    /// Every F1 context tag the app registers has a topic (spec: S1 Help
    /// "Topic files").
    #[test]
    fn every_context_tag_has_a_topic() {
        let book = try_book().expect("the topics are valid");
        for tag in CONTEXT_TAGS {
            assert!(book.topic_for_context(tag).is_some(), "the {tag} tag has no topic");
        }
    }

    /// The tour has the spec's five steps, each with a target, title and body.
    #[test]
    fn the_tour_has_five_named_steps() {
        assert_eq!(TOUR.steps.len(), 5);
        for step in TOUR.steps {
            assert!(!step.target.is_empty() && !step.title.is_empty() && !step.body.is_empty(), "a step is incomplete");
        }
        assert_eq!(TOUR.app, "audio-editor");
        assert_eq!(TOUR.id, "intro");
        assert_eq!(TOUR.version, 1);
    }

    /// The About box names the app and carries the version, copyright and the
    /// font license texts.
    #[test]
    fn the_about_box_names_the_app_and_lists_the_licenses() {
        assert_eq!(ABOUT.app_name, "CutMark Audio");
        assert_eq!(ABOUT.brand, Some(ui_kit::brand::App::Audio));
        assert_eq!(ABOUT.version, env!("CARGO_PKG_VERSION"));
        assert!(ABOUT.copyright.contains("2026"), "the copyright line names the year");
        assert!(ABOUT.notices.contains("Open Font License"), "the font licenses are embedded");
        assert!(ABOUT.notices.contains("LAME"), "the third-party notices are embedded");
    }

    /// Searching "mp3" puts Exporting first: its keyword outranks the topics
    /// that only mention MP3 in their body (spec: S1 Help acceptance 2).
    #[test]
    fn searching_mp3_ranks_exporting_first() {
        let book = try_book().expect("the topics are valid");
        let hits = ui_kit::help::search(&book, "mp3");
        let first = hits.first().expect("\"mp3\" must match a topic");
        assert_eq!(book.topics[first.topic].id, "exporting", "\"mp3\" must open Exporting first");
        assert!(hits.iter().any(|hit| book.topics[hit.topic].id == "importing-audio"), "Importing audio still mentions MP3 in its body");
    }
}
