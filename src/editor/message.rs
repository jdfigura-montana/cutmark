//! editor::message: a titled message for the app's messages dialog.

/// Title for messages about recording (arming, takes, the input device).
pub const TITLE_RECORDING: &str = "Recording";
/// Title for messages about imported, relinked or missing media.
pub const TITLE_MEDIA: &str = "Media";
/// Title for messages about the project itself (autosave, cleanup).
pub const TITLE_PROJECT: &str = "Project";
/// Dialog title when the messages shown together carry different titles.
pub const TITLE_MIXED: &str = "Messages";

/// One message for the user and the title it is shown under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Dialog title, one of the `TITLE_*` constants.
    pub title: &'static str,
    /// The text of the message.
    pub text: String,
}

impl Message {
    /// A recording message ("Arm a track first", "Recording too short.", ...).
    pub fn recording(text: impl Into<String>) -> Message {
        Message {
            title: TITLE_RECORDING,
            text: text.into(),
        }
    }

    /// A media message (import, relink, missing files).
    pub fn media(text: impl Into<String>) -> Message {
        Message {
            title: TITLE_MEDIA,
            text: text.into(),
        }
    }

    /// A project message (autosave, unused-media cleanup).
    pub fn project(text: impl Into<String>) -> Message {
        Message {
            title: TITLE_PROJECT,
            text: text.into(),
        }
    }
}

/// The dialog title for messages shown together: their own title when they
/// all share one, "Messages" when the titles differ.
pub fn dialog_title(messages: &[Message]) -> &'static str {
    match messages.split_first() {
        Some((first, rest)) if rest.iter().all(|m| m.title == first.title) => first.title,
        _ => TITLE_MIXED,
    }
}

/// The texts of `messages`, for tests that only care what was said.
#[cfg(test)]
pub fn texts(messages: Vec<Message>) -> Vec<String> {
    messages.into_iter().map(|m| m.text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_set_the_title() {
        assert_eq!(Message::recording("a").title, "Recording");
        assert_eq!(Message::media("a").title, "Media");
        assert_eq!(Message::project("a").title, "Project");
        assert_eq!(Message::media("text").text, "text");
    }

    #[test]
    fn the_dialog_title_is_the_shared_title_or_messages() {
        assert_eq!(
            dialog_title(&[Message::recording("a")]),
            "Recording",
            "one message"
        );
        assert_eq!(
            dialog_title(&[Message::project("a"), Message::project("b")]),
            "Project",
            "same title"
        );
        assert_eq!(
            dialog_title(&[Message::project("a"), Message::recording("b")]),
            "Messages",
            "different titles"
        );
        assert_eq!(dialog_title(&[]), "Messages", "nothing to show");
    }
}
