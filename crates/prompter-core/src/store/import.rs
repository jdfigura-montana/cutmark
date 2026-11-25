//! Importing .md and .txt files (docs/suite/prompter.md, "Import"). The source file is only
//! read.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use super::{Imported, Result, ScriptMeta, Store, StoreError, io_error, write_atomic};
use crate::limits::{MAX_WORDS, split_at_heading};
use crate::script;

impl Store {
    /// Copies a .md or .txt file into a new script, or into several if it is over
    /// [`MAX_WORDS`]. The file must be UTF-8, or UTF-16 with a byte order mark. The new
    /// script is always UTF-8.
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
        let bytes = fs::read(source).map_err(io_error(source))?;
        let raw = decode(bytes).ok_or(StoreError::ImportEncoding)?;
        let text = if plain { markdown_from_text(&raw) } else { raw };

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

/// The file's text without its byte order mark, or `None` if it is neither UTF-8 nor UTF-16
/// with a mark.
fn decode(bytes: Vec<u8>) -> Option<String> {
    let text = match bytes.as_slice() {
        // The UTF-32 LE mark starts with the UTF-16 LE one.
        [0xff, 0xfe, 0, 0, ..] => return None,
        [0xff, 0xfe, rest @ ..] => decode_utf16(rest, u16::from_le_bytes)?,
        [0xfe, 0xff, rest @ ..] => decode_utf16(rest, u16::from_be_bytes)?,
        _ => {
            let mut text = String::from_utf8(bytes).ok()?;
            // A byte order mark is not part of the text, and would hide a heading on line 1.
            if text.starts_with('\u{feff}') {
                text.remove(0);
            }
            text
        }
    };
    // A NUL means another encoding, such as UTF-16 without a mark, which also reads as UTF-8.
    (!text.contains('\0')).then_some(text)
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> Option<String> {
    let (pairs, odd) = bytes.as_chunks::<2>();
    if !odd.is_empty() {
        return None;
    }
    char::decode_utf16(pairs.iter().map(|&pair| unit(pair))).collect::<std::result::Result<_, _>>().ok()
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
    // A lone '\r' ends a line in Markdown, so it has to end one here too.
    for line in text.replace("\r\n", "\n").split(['\n', '\r']) {
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

/// Appends a line with a backslash before each character Markdown would read as syntax.
/// `[[...]]` cues are copied as they are.
fn push_escaped(markdown: &mut String, line: &str) {
    let marker = list_marker(line);
    let mut after_cue = false;
    let mut at = 0;
    while let Some(c) = line[at..].chars().next() {
        if let Some(length) = cue_length(&line[at..]) {
            markdown.push_str(&line[at..at + length]);
            at += length;
            after_cue = true;
            continue;
        }
        let active = match c {
            // Code, emphasis, links, images, HTML, entities, strikethrough and tables. With
            // every '[' and '<' escaped, '!' and a mid-line '>' do nothing.
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '&' | '~' | '|' => true,
            // Headings, quotes, lists, rules and heading underlines.
            '#' | '>' | '-' | '+' | '=' => at == 0,
            // A cue followed by "(...)" would turn into a link.
            '(' => after_cue,
            _ => marker == Some(at),
        };
        if active {
            markdown.push('\\');
        }
        markdown.push(c);
        at += c.len_utf8();
        after_cue = false;
    }
}

/// The position of the '.' or ')' that would make the line a numbered list item.
fn list_marker(line: &str) -> Option<usize> {
    let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
    let at = line.len() - rest.len();
    let mut after = rest.chars();
    let marker = at > 0 && matches!(after.next(), Some('.' | ')')) && after.next().is_none_or(char::is_whitespace);
    marker.then_some(at)
}

/// The length of the cue at the start of `text`, if there is one. As in the parser, a longer
/// run of ']' does not close a cue.
fn cue_length(text: &str) -> Option<usize> {
    let inside = text.strip_prefix("[[")?;
    let close = inside.find("]]")?;
    let closes = !inside[..close].contains("[[") && !inside[close + 2..].starts_with(']');
    closes.then_some(close + 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{BlockKind, Cue};

    const REFUSAL: &str = "This file is not UTF-8 or UTF-16. Save it as UTF-8 and import again.";

    /// Imports `bytes` as a .txt file and returns the new script's Markdown.
    fn import_txt(bytes: &[u8]) -> Result<String> {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("source.txt");
        fs::write(&source, bytes).unwrap();
        let data = folder.path().join("data");
        let mut store = Store::open(&data)?;
        let imported = store.import(&source)?;
        Ok(fs::read_to_string(store.script_path(&imported.scripts[0].id)).unwrap())
    }

    /// Checks that `markdown` shows `text` as one unstyled paragraph.
    fn assert_shows(markdown: &str, text: &str) {
        let blocks = script::parse(markdown).blocks;
        assert_eq!(blocks.len(), 1, "{markdown:?} gave {blocks:?}");
        assert_eq!(blocks[0].kind, BlockKind::Paragraph, "{markdown:?}");
        assert!(blocks[0].spans.iter().all(|span| !span.bold && !span.italic), "{markdown:?}");
        assert_eq!(blocks[0].text(), text, "{markdown:?}");
    }

    #[test]
    fn txt_lines_read_like_the_file() {
        let lines = [
            "press <Enter> now",
            "*word*",
            "1. not a list",
            "# not a heading",
            "2) not a list",
            "> not a quote",
            "- not a list",
            "+ not a list",
            "---",
            "===",
            "```",
            "~~~",
            "a ~~b~~ __c__ **d** _e_ `f` g",
            "[a](b) ![c](d) [e][f]",
            "[g]: http://example.com",
            "<http://example.com> &amp; &#65; R&D",
            "a | b",
            r"C:\new\*folder* ends with \",
            "{a} (b) c! d. e # f + g - h = i > j",
            "[[not closed",
            "[[pause]]] is text",
        ];
        for line in lines {
            assert_shows(&import_txt(line.as_bytes()).unwrap(), line);
        }
    }

    #[test]
    fn txt_escapes_only_what_would_take_effect() {
        let line = "Wait (a moment). It's 3.5 - or #1!";
        assert_eq!(import_txt(line.as_bytes()).unwrap(), format!("{line}\n"));
    }

    #[test]
    fn txt_lone_carriage_return_ends_a_line() {
        assert_eq!(import_txt(b"one\r# two").unwrap(), "one\n\\# two\n");
    }

    #[test]
    fn txt_keeps_a_cue_inside_a_line() {
        let markdown = import_txt(b"*wait* [[pause]](now) <go>").unwrap();
        assert!(markdown.contains("[[pause]]"), "{markdown:?}");
        let blocks = script::parse(&markdown).blocks;
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        assert_eq!(blocks[0].text().trim(), "*wait*");
        assert_eq!(blocks[1].kind, BlockKind::Cue(Cue::Pause));
        assert_eq!(blocks[2].text().trim(), "(now) <go>");
    }

    #[test]
    fn utf8_and_utf16_files_are_decoded() {
        let line = "# caf\u{e9} \u{1f3ac} <Enter>";
        let marked = format!("\u{feff}{line}\r\n");
        let little: Vec<u8> = marked.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let big: Vec<u8> = marked.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for bytes in [line.as_bytes(), marked.as_bytes(), &little, &big] {
            assert_shows(&import_txt(bytes).unwrap(), line);
        }
    }

    #[test]
    fn other_encodings_are_refused() {
        let files: [&[u8]; 5] = [
            // Windows-1252.
            b"caf\xe9",
            // UTF-16 without a mark.
            b"h\0i\0",
            // UTF-32 LE.
            b"\xff\xfe\0\0h\0\0\0",
            // UTF-16 cut off in the middle of a unit.
            b"\xff\xfeh\0i",
            // UTF-16 with half a surrogate pair.
            b"\xff\xfe\x00\xd8",
        ];
        for bytes in files {
            let error = import_txt(bytes).unwrap_err();
            assert_eq!(error.to_string(), REFUSAL, "{bytes:?}");
        }
    }
}
