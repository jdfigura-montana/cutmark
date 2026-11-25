//! Importing .md and .txt files (docs/suite/prompter.md, "Import"). The source file is only
//! read.

use std::ffi::OsStr;
use std::path::Path;

use super::{Imported, Result, ScriptMeta, Store, StoreError, read_text, write_atomic};
use crate::limits::{MAX_WORDS, split_at_heading};
use crate::script;

impl Store {
    /// Copies a .md or .txt file into a new script, or into several if it is over
    /// [`MAX_WORDS`]. The file must be UTF-8.
    ///
    /// The title is the first heading, else the file name without its extension. The parts of
    /// a split file share that title and are numbered.
    pub fn import(&mut self, source: &Path) -> Result<Imported> {
        let extension = source.extension().and_then(OsStr::to_str).map(str::to_ascii_lowercase);
        let plain = match extension.as_deref() {
            Some("md") => false,
            Some("txt") => true,
            _ => return Err(StoreError::UnsupportedImport(source.to_path_buf())),
        };
        let raw = read_text(source)?;
        // A byte order mark is not part of the text, and would hide a heading on line 1.
        let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
        let text = if plain { markdown_from_text(raw) } else { raw.to_owned() };

        let title = title_of(&text, source);
        let parts = split_at_heading(&text, MAX_WORDS);
        let count = parts.len();
        let mut scripts = Vec::with_capacity(count);
        for (number, part) in parts.iter().enumerate() {
            let title = if count > 1 { format!("{title} (part {} of {count})", number + 1) } else { title.clone() };
            let meta = ScriptMeta::create(&title, None);
            write_atomic(&self.script_path(&meta.id), part.as_bytes())?;
            scripts.push(meta);
        }
        // All the parts are listed at once, so a split file never shows up half imported.
        self.change(|library| library.scripts.extend_from_slice(&scripts))?;
        Ok(Imported { scripts, split: count > 1 })
    }
}

fn title_of(markdown: &str, source: &Path) -> String {
    let sections = script::sections(&script::parse(markdown));
    let heading = sections.iter().map(|section| section.title.trim()).find(|title| !title.is_empty());
    let file = || source.file_stem().map(|stem| stem.to_string_lossy().trim().to_owned()).filter(|stem| !stem.is_empty());
    heading.map(str::to_owned).or_else(file).unwrap_or_else(|| "Untitled".to_owned())
}

/// Turns plain text into Markdown with one paragraph for each block of lines between blank
/// lines.
fn markdown_from_text(text: &str) -> String {
    let mut markdown = String::new();
    let mut between = false;
    for line in text.lines() {
        // Indented lines would become code blocks, and two spaces at the end a line break.
        let line = line.trim();
        if line.is_empty() {
            between = !markdown.is_empty();
            continue;
        }
        if between {
            markdown.push('\n');
            between = false;
        }
        push_escaped(&mut markdown, line);
        markdown.push('\n');
    }
    markdown
}

/// Appends a line, escaping what Markdown would read as syntax at its start.
fn push_escaped(markdown: &mut String, line: &str) {
    let Some(first) = line.chars().next() else {
        return;
    };
    match first {
        // "[[" opens a cue and cannot start a link definition, so cues are left alone.
        '[' if line.starts_with("[[") => markdown.push_str(line),
        // The whole run is escaped: the rest of "```" or "***" would still mean something.
        '#' | '>' | '-' | '+' | '*' | '_' | '=' | '`' | '~' | '<' | '[' | '&' | '\\' => {
            let rest = line.trim_start_matches(first);
            for _ in 0..line.len() - rest.len() {
                markdown.push('\\');
                markdown.push(first);
            }
            markdown.push_str(rest);
        }
        // An image would show only its description.
        '!' if line.starts_with("![") => {
            markdown.push_str("!\\");
            markdown.push_str(&line[1..]);
        }
        '0'..='9' => {
            // "12. " or "12) " would start a numbered list.
            let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
            let mut after = rest.chars();
            let marker = matches!(after.next(), Some('.' | ')')) && after.next().is_none_or(char::is_whitespace);
            markdown.push_str(&line[..line.len() - rest.len()]);
            if marker {
                markdown.push('\\');
            }
            markdown.push_str(rest);
        }
        _ => markdown.push_str(line),
    }
}
