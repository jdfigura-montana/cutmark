//! The app's own view settings, kept in a small file beside the library.

use std::path::{Path, PathBuf};
use std::{fs, io};

const FILE_NAME: &str = "editor.txt";
const SHOW_MARKDOWN: &str = "show_markdown=";

/// The editor's settings. The default has no folder, so it is not kept.
#[derive(Debug, Default)]
pub struct EditorPrefs {
    folder: Option<PathBuf>,
    /// The View menu's Show Markdown.
    show_markdown: bool,
}

impl EditorPrefs {
    /// The settings kept in `folder`. A file that is missing or cannot be read gives the
    /// defaults.
    pub fn load(folder: &Path) -> Self {
        let text = fs::read_to_string(folder.join(FILE_NAME)).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::NotFound {
                log::warn!("the editor settings could not be read: {error}");
            }
            String::new()
        });
        let show_markdown = text.lines().any(|line| line.trim().strip_prefix(SHOW_MARKDOWN) == Some("true"));
        Self { folder: Some(folder.to_owned()), show_markdown }
    }

    pub fn show_markdown(&self) -> bool {
        self.show_markdown
    }

    /// Changes the setting and writes it. It changes even if the write fails.
    pub fn set_show_markdown(&mut self, show: bool) -> io::Result<()> {
        self.show_markdown = show;
        let Some(folder) = &self.folder else {
            return Ok(());
        };
        // Written whole beside the file, then moved over it.
        let temp = folder.join(format!("{FILE_NAME}.tmp"));
        fs::write(&temp, format!("{SHOW_MARKDOWN}{show}\n"))?;
        fs::rename(&temp, folder.join(FILE_NAME))
    }
}
