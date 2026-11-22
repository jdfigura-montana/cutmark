//! The gallery's help topics: four sample Markdown files embedded at build
//! time, so the Help window has content to show (spec: S1 Help "Topic files").

use ui_kit::help::{HelpBook, HelpError};

/// The sample topics, in reading order. The files sit in `apps/gallery/help`
/// and are embedded, so the app ships no files to read at runtime.
const SOURCES: &[(&str, &str)] = &[
    ("getting-started.md", include_str!("../help/getting-started.md")),
    ("core-widgets.md", include_str!("../help/core-widgets.md")),
    ("media-widgets.md", include_str!("../help/media-widgets.md")),
    ("appearance.md", include_str!("../help/appearance.md")),
];

/// Parses and checks the sample topics. A broken id or link is a build check
/// failure, so the app only ever sees a valid book.
pub fn book() -> Result<HelpBook, Vec<HelpError>> {
    HelpBook::from_sources(SOURCES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_page;
    use crate::media;

    /// The four sample topics form a valid book.
    #[test]
    fn the_sample_topics_form_a_valid_book() {
        let book = book().expect("the sample topics are valid");
        assert_eq!(book.len(), 4);
        for id in ["getting-started", "core-widgets", "media-widgets", "appearance"] {
            assert!(book.get(id).is_some(), "the {id} topic must exist");
        }
    }

    /// Every F1 context tag the pages register has a topic.
    #[test]
    fn every_context_tag_has_a_topic() {
        let book = book().expect("the sample topics are valid");
        for tag in [core_page::CONTEXT_TAG, media::CONTEXT_TAG] {
            assert!(book.topic_for_context(tag).is_some(), "the {tag} tag must have a topic");
        }
    }
}
