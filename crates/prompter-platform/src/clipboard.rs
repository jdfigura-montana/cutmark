//! The system clipboard: HTML and plain text. Windows backend only; elsewhere reads find nothing.
//! Other programs write the clipboard, so data that does not parse reads as nothing.

use crate::error::Result;

/// The HTML another program copied: the fragment of the "HTML Format" entry, or the whole
/// document if the fragment is not marked. `None` if there is no HTML or it does not parse.
#[cfg(windows)]
pub fn read_html() -> Option<String> {
    use clipboard_win::{Clipboard, raw};

    let format = raw::register_format("HTML Format")?;
    let _open = Clipboard::new_attempts(OPEN_ATTEMPTS).ok()?;
    let mut data = Vec::new();
    raw::get_vec(format.get(), &mut data).ok()?;
    parse_cf_html(&data)
}

/// The clipboard's Unicode text. `None` if there is none or it is not valid UTF-16.
#[cfg(windows)]
pub fn read_text() -> Option<String> {
    use clipboard_win::{Clipboard, formats, raw};

    let _open = Clipboard::new_attempts(OPEN_ATTEMPTS).ok()?;
    let mut data = Vec::new();
    raw::get_vec(formats::CF_UNICODETEXT, &mut data).ok()?;
    decode_unicode_text(&data)
}

/// Replaces the clipboard's contents with this text.
#[cfg(windows)]
pub fn write_text(text: &str) -> Result<()> {
    clipboard_win::set_clipboard_string(text).map_err(|error| crate::error::PlatformError::Win32 { call: "set clipboard text", code: error.raw_code() as u32 })
}

/// Clipboard HTML is not read outside Windows.
#[cfg(not(windows))]
pub fn read_html() -> Option<String> {
    None
}

/// Clipboard text is not read outside Windows.
#[cfg(not(windows))]
pub fn read_text() -> Option<String> {
    None
}

/// The clipboard is not written outside Windows.
#[cfg(not(windows))]
pub fn write_text(_text: &str) -> Result<()> {
    Err(crate::error::PlatformError::Unsupported)
}

/// Another program may hold the clipboard open for a moment.
#[cfg(windows)]
const OPEN_ATTEMPTS: usize = 10;

/// Reads a CF_HTML entry: "Key:Value" header lines, then UTF-8 HTML. The header's offsets count
/// bytes from the start of the entry. The fragment pair wins; without a usable one, the whole
/// HTML pair is used.
#[cfg(any(windows, test))]
fn parse_cf_html(data: &[u8]) -> Option<String> {
    let (mut start_fragment, mut end_fragment, mut start_html, mut end_html) = (None, None, None, None);
    // Splitting on both characters takes CRLF, LF and CR; the empty pieces are skipped.
    for line in data.split(|&byte| byte == b'\r' || byte == b'\n').filter(|line| !line.is_empty()) {
        // The header ends where the HTML starts.
        if line.starts_with(b"<") {
            break;
        }
        let Some(colon) = line.iter().position(|&byte| byte == b':') else {
            break;
        };
        let (key, value) = line.split_at(colon);
        let slot = match key {
            b"StartFragment" => &mut start_fragment,
            b"EndFragment" => &mut end_fragment,
            b"StartHTML" => &mut start_html,
            b"EndHTML" => &mut end_html,
            _ => continue,
        };
        // The first one counts, so text after the header cannot replace it. A value of -1, which
        // means "not given", does not parse and stays unset.
        if slot.is_none() {
            *slot = std::str::from_utf8(&value[1..]).ok().and_then(|text| text.trim().parse::<usize>().ok());
        }
    }
    html_between(data, start_fragment, end_fragment).or_else(|| html_between(data, start_html, end_html))
}

/// The text between two offsets, if both are given, in range, in order and on UTF-8 boundaries.
#[cfg(any(windows, test))]
fn html_between(data: &[u8], start: Option<usize>, end: Option<usize>) -> Option<String> {
    let bytes = data.get(start?..end?)?;
    // Some writers count the closing NUL.
    std::str::from_utf8(bytes).ok().map(|text| text.trim_end_matches('\0').to_owned())
}

/// CF_UNICODETEXT: UTF-16 up to the first NUL. The entry's memory may be longer than the text,
/// and a stray last byte is ignored.
#[cfg(any(windows, test))]
fn decode_unicode_text(data: &[u8]) -> Option<String> {
    let units: Vec<u16> = data.as_chunks::<2>().0.iter().map(|&pair| u16::from_le_bytes(pair)).take_while(|&unit| unit != 0).collect();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: &str = "<html><body><!--StartFragment-->";
    const AFTER: &str = "<!--EndFragment--></body></html>";

    /// A CF_HTML entry around `fragment`, with true byte offsets and `eol` ending each header line.
    fn entry(fragment: &str, eol: &str, with_fragment_offsets: bool) -> Vec<u8> {
        let header = |offsets: [usize; 4]| {
            let [start_html, end_html, start_fragment, end_fragment] = offsets;
            let mut text = format!("Version:0.9{eol}StartHTML:{start_html:010}{eol}EndHTML:{end_html:010}{eol}");
            if with_fragment_offsets {
                text += &format!("StartFragment:{start_fragment:010}{eol}EndFragment:{end_fragment:010}{eol}");
            }
            text + &format!("SourceURL:https://example.com/a:b{eol}")
        };
        // Fixed-width numbers, so the header's length does not depend on them.
        let start_html = header([0; 4]).len();
        let start_fragment = start_html + BEFORE.len();
        let end_fragment = start_fragment + fragment.len();
        let end_html = end_fragment + AFTER.len();
        (header([start_html, end_html, start_fragment, end_fragment]) + BEFORE + fragment + AFTER).into_bytes()
    }

    #[test]
    fn the_fragment_is_read_with_crlf_lf_or_cr() {
        for eol in ["\r\n", "\n", "\r"] {
            assert_eq!(parse_cf_html(&entry("<b>bold</b>", eol, true)).as_deref(), Some("<b>bold</b>"), "line end {eol:?}");
        }
    }

    #[test]
    fn without_fragment_offsets_the_whole_html_is_read() {
        let whole = format!("{BEFORE}<i>x</i>{AFTER}");
        for eol in ["\r\n", "\n"] {
            assert_eq!(parse_cf_html(&entry("<i>x</i>", eol, false)), Some(whole.clone()), "line end {eol:?}");
        }
    }

    #[test]
    fn offsets_count_bytes_in_non_ascii_text() {
        let fragment = "<p>Señor, 日本語 and 🎬, “quoted”</p>";
        for eol in ["\r\n", "\n"] {
            assert_eq!(parse_cf_html(&entry(fragment, eol, true)).as_deref(), Some(fragment));
            assert_eq!(parse_cf_html(&entry(fragment, eol, false)), Some(format!("{BEFORE}{fragment}{AFTER}")));
        }
    }

    #[test]
    fn bytes_after_the_html_are_ignored() {
        let mut data = entry("a", "\r\n", true);
        data.extend_from_slice(&[0, 0xFF, 0xFE]);
        assert_eq!(parse_cf_html(&data).as_deref(), Some("a"));
    }

    #[test]
    fn the_fragment_is_enough_when_the_html_pair_is_not_given() {
        let header = "StartHTML:-1\r\nEndHTML:-1\r\nStartFragment:0000000076\r\nEndFragment:0000000079\r\n";
        assert_eq!(header.len(), 76);
        assert_eq!(parse_cf_html(format!("{header}abc").as_bytes()).as_deref(), Some("abc"));
    }

    #[test]
    fn a_bad_fragment_pair_falls_back_to_the_html_pair() {
        // The HTML pair spans "héllo", which starts at byte 80; the é is bytes 81 and 82.
        for (start, end) in [("90", "81"), ("80", "999"), ("82", "86"), ("x", "86"), ("80", "-1")] {
            let mut header = format!("StartHTML:80\nEndHTML:86\nStartFragment:{start}\nEndFragment:{end}\nPad:");
            header += &"-".repeat(79 - header.len());
            header += "\n";
            assert_eq!(parse_cf_html(format!("{header}héllo").as_bytes()).as_deref(), Some("héllo"), "{start}..{end}");
        }
    }

    #[test]
    fn data_that_does_not_parse_is_nothing() {
        let cases: [&[u8]; 8] = [
            b"",
            b"\r\n\r\n",
            b"<html>no header</html>",
            b"Version:0.9\r\n<html></html>",
            b"StartFragment:5\r\n",
            b"StartFragment:0\r\nEndFragment:99999999999999999999999999\r\n",
            b"StartHTML:9\nEndHTML:3\n<p></p>",
            &[0xFF, 0x00, 0x3A, 0x80],
        ];
        for data in cases {
            assert_eq!(parse_cf_html(data), None, "{data:?}");
        }
        // Offsets are fine, the bytes are not UTF-8.
        assert_eq!(parse_cf_html(b"StartFragment:32\nEndFragment:34\n\xC3\x28"), None);
    }

    #[test]
    fn offsets_written_after_the_header_are_not_read() {
        assert_eq!(parse_cf_html(b"Version:0.9\n<p>\nStartFragment:0\nEndFragment:3\n</p>"), None);
    }

    #[test]
    fn unicode_text_stops_at_the_first_nul() {
        let bytes = |text: &str| text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        assert_eq!(decode_unicode_text(&bytes("Señor 🎬\r\nnext\0left over")).as_deref(), Some("Señor 🎬\r\nnext"));
        assert_eq!(decode_unicode_text(&bytes("no terminator")).as_deref(), Some("no terminator"));
        assert_eq!(decode_unicode_text(&[b'a', 0, b'b']).as_deref(), Some("a"));
        assert_eq!(decode_unicode_text(&[]).as_deref(), Some(""));
        // A lone surrogate is not text.
        assert_eq!(decode_unicode_text(&[0x00, 0xD8, 0, 0]), None);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "overwrites the real clipboard; run by hand on a Windows desktop"]
    fn written_text_reads_back() -> std::result::Result<(), Box<dyn std::error::Error>> {
        for text in ["Cutmark clipboard test: Señor, 日本語, 🎬\r\nsecond line", ""] {
            write_text(text)?;
            assert_eq!(read_text().as_deref(), Some(text));
            // Writing text empties the clipboard first, so no older HTML is left behind.
            assert_eq!(read_html(), None);
        }
        Ok(())
    }
}
