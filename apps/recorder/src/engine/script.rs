//! The Prompter link's files for one take: `{base}_script.md`, a snapshot of the script, and `{base}_script.csv`, one row
//! per script position.
//!
//! A write error here never stops a recording. The controller shows it as a warning banner.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use thiserror::Error;

use super::marks::write_atomic;

/// How many characters of a script line a mark keeps.
const LINE_PREFIX_CHARS: usize = 60;

/// The text of the script file when no snapshot arrived before the first position.
const NO_SNAPSHOT_TEXT: &str = "No script snapshot received";

/// The header of the script position log.
const CSV_HEADER: &str = "ms,line,token,text";

/// A script as the Prompter sent it with `scriptSnapshot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSnapshot {
    /// The script's title.
    pub title: String,
    /// The script as Markdown.
    pub markdown: String,
}

/// Where the Prompter is in the script, stamped with the time the Recorder received it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionMsg {
    /// Identifies the script across messages.
    pub script_id: String,
    /// The script's title.
    pub title: String,
    /// The word or token the Prompter is at.
    pub token: String,
    /// The line number the Prompter is at.
    pub line: u64,
    /// The text of that line.
    pub text: String,
    /// The QPC time at which the Recorder received the message.
    pub qpc: i64,
}

/// A script message from an API client on its way to the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptMsg {
    /// The latest copy of a script.
    Snapshot {
        /// Identifies the script across messages.
        script_id: String,
        /// The script's title.
        title: String,
        /// The script as Markdown.
        markdown: String,
    },
    /// A new reading position.
    Position(PositionMsg),
}

/// Why a script file could not be written.
#[derive(Debug, Error)]
pub enum ScriptError {
    /// The script snapshot file could not be written.
    #[error("the script file could not be written: {0}")]
    Snapshot(io::Error),
    /// The script position log could not be written.
    #[error("the script position log could not be written: {0}")]
    Log(io::Error),
}

/// The script files of one take, and the line the Prompter is on.
pub struct TakeScript {
    dir: PathBuf,
    base_name: String,
    started_at: DateTime<Local>,
    snapshot_written: bool,
    log: Option<File>,
    current: Option<(u64, String)>,
}

impl TakeScript {
    /// The script files of a take that started at `started_at`, written into `dir` as `base_name`.
    pub fn new(dir: &Path, base_name: &str, started_at: DateTime<Local>) -> Self {
        Self { dir: dir.to_path_buf(), base_name: base_name.to_owned(), started_at, snapshot_written: false, log: None, current: None }
    }

    /// Takes in a position received `ms` into the take; `snapshot` is the cached script for the position's script id.
    ///
    /// The first call writes the snapshot file. Every call appends a row to the log. The current line is updated even
    /// when a write fails, and the first error is returned.
    pub fn position(&mut self, snapshot: Option<&ScriptSnapshot>, position: &PositionMsg, ms: u64) -> Result<(), ScriptError> {
        self.current = Some((position.line, line_prefix(&position.text)));
        let mut first_error = None;
        if !self.snapshot_written {
            self.snapshot_written = true;
            if let Err(err) = self.write_snapshot(snapshot, position) {
                first_error = Some(ScriptError::Snapshot(err));
            }
        }
        if let Err(err) = self.append_row(position, ms) {
            first_error.get_or_insert(ScriptError::Log(err));
        }
        first_error.map_or(Ok(()), Err)
    }

    /// The current line as its number and the label that marks carry, `L<line>: <first 60 characters>`.
    pub fn current(&self) -> Option<(u64, String)> {
        self.current.as_ref().map(|(line, text)| (*line, format!("L{line}: {text}").trim_end().to_owned()))
    }

    /// Writes `{base}_script.md`.
    fn write_snapshot(&self, snapshot: Option<&ScriptSnapshot>, position: &PositionMsg) -> io::Result<()> {
        let title = snapshot.map_or(position.title.as_str(), |snapshot| snapshot.title.as_str());
        let body = snapshot.map_or(NO_SNAPSHOT_TEXT, |snapshot| snapshot.markdown.as_str());
        let text = format!("# {title}\n\n- Script ID: {}\n- Take started: {}\n\n{body}\n", position.script_id, self.started_at.format("%Y-%m-%d %H:%M:%S"));
        write_atomic(&self.dir.join(format!("{}_script.md", self.base_name)), text.as_bytes())
    }

    /// Appends one row to `{base}_script.csv`, writing the header first when the file is new, and flushes it.
    fn append_row(&mut self, position: &PositionMsg, ms: u64) -> io::Result<()> {
        if self.log.is_none() {
            let file = OpenOptions::new().create(true).append(true).open(self.dir.join(format!("{}_script.csv", self.base_name)))?;
            self.log = Some(file);
        }
        let Some(file) = &mut self.log else { return Ok(()) };
        let mut rows = String::new();
        if file.metadata()?.len() == 0 {
            rows.push_str(CSV_HEADER);
            rows.push_str("\r\n");
        }
        let fields = [ms.to_string(), position.line.to_string(), position.token.clone(), position.text.clone()];
        let row: Vec<String> = fields.iter().map(|field| csv_field(field)).collect();
        rows.push_str(&row.join(","));
        rows.push_str("\r\n");
        file.write_all(rows.as_bytes())?;
        file.flush()
    }
}

/// The first 60 characters of a line's text, without surrounding spaces.
fn line_prefix(text: &str) -> String {
    text.trim().chars().take(LINE_PREFIX_CHARS).collect()
}

/// RFC 4180 quoting: quote only when the field has a comma, double quote, CR or LF.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\r', '\n']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text.to_owned() }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::TimeZone;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cutmark_script_{name}_{}_{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    fn started() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 19, 10, 30, 0).single().expect("a valid date")
    }

    fn at(line: u64, token: &str, text: &str) -> PositionMsg {
        PositionMsg { script_id: "s1".to_owned(), title: "Fake script".to_owned(), token: token.to_owned(), line, text: text.to_owned(), qpc: 0 }
    }

    #[test]
    fn the_first_position_writes_the_snapshot_and_every_position_adds_a_row() {
        let dir = scratch("rows");
        let mut script = TakeScript::new(&dir, "take", started());
        let snapshot = ScriptSnapshot { title: "Fake script".to_owned(), markdown: "# Intro\n\nHello there.".to_owned() };
        assert_eq!(script.current(), None);
        script.position(Some(&snapshot), &at(1, "Hello", "Hello there."), 1200).expect("first");
        script.position(Some(&snapshot), &at(2, "say, \"hi\"", "Say \"hi\", then go."), 1700).expect("second");
        let md = fs::read_to_string(dir.join("take_script.md")).expect("script file");
        assert_eq!(md, "# Fake script\n\n- Script ID: s1\n- Take started: 2026-11-19 10:30:00\n\n# Intro\n\nHello there.\n");
        let csv = fs::read_to_string(dir.join("take_script.csv")).expect("log");
        assert_eq!(csv, "ms,line,token,text\r\n1200,1,Hello,Hello there.\r\n1700,2,\"say, \"\"hi\"\"\",\"Say \"\"hi\"\", then go.\"\r\n");
        assert_eq!(script.current(), Some((2, "L2: Say \"hi\", then go.".to_owned())));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_snapshot_is_said_in_the_script_file() {
        let dir = scratch("nosnap");
        let mut script = TakeScript::new(&dir, "take", started());
        script.position(None, &at(3, "x", "Third line"), 10).expect("position");
        let md = fs::read_to_string(dir.join("take_script.md")).expect("script file");
        assert!(md.starts_with("# Fake script\n") && md.contains("- Script ID: s1") && md.trim_end().ends_with("No script snapshot received"), "{md}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_snapshot_is_written_only_once_and_the_current_line_is_cut_to_60_characters() {
        let dir = scratch("once");
        let mut script = TakeScript::new(&dir, "take", started());
        let first = ScriptSnapshot { title: "A".to_owned(), markdown: "first".to_owned() };
        let later = ScriptSnapshot { title: "A".to_owned(), markdown: "later".to_owned() };
        script.position(Some(&first), &at(1, "a", "x"), 0).expect("first");
        script.position(Some(&later), &at(2, "b", &format!("  {}  ", "é".repeat(80))), 5).expect("second");
        assert!(fs::read_to_string(dir.join("take_script.md")).expect("script file").contains("first"));
        let (line, label) = script.current().expect("a current line");
        assert_eq!(line, 2);
        assert_eq!(label, format!("L2: {}", "é".repeat(60)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_error_is_returned_but_the_current_line_still_updates() {
        let dir = scratch("gone").join("missing");
        let mut script = TakeScript::new(&dir, "take", started());
        let result = script.position(None, &at(4, "t", "Fourth"), 0);
        assert!(matches!(result, Err(ScriptError::Snapshot(_))));
        assert_eq!(script.current(), Some((4, "L4: Fourth".to_owned())));
    }
}
