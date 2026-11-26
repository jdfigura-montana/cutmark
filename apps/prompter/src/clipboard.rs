//! The clipboard, behind a trait so that tests can stand in for it: what a paste inserts, and
//! what is there for the Paste items.

use prompter_core::convert::html_to_markdown;
use prompter_platform::clipboard as system;

pub trait Clipboard {
    fn read_html(&self) -> Option<String>;
    fn read_text(&self) -> Option<String>;
    fn write_text(&self, text: &str) -> prompter_platform::Result<()>;
}

/// The Windows clipboard.
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn read_html(&self) -> Option<String> {
        system::read_html()
    }

    fn read_text(&self) -> Option<String> {
        system::read_text()
    }

    fn write_text(&self, text: &str) -> prompter_platform::Result<()> {
        system::write_text(text)
    }
}

/// What a paste into the editor inserts, or `None` with nothing to paste: the HTML as Markdown,
/// else the text. A plain-text paste takes only the text. HTML that converts to nothing, such
/// as a lone image, counts as no HTML. A source is read only if it is needed.
pub fn paste_text(plain: bool, html: impl FnOnce() -> Option<String>, text: impl FnOnce() -> Option<String>) -> Option<String> {
    let converted = (!plain).then(html).flatten().map(|html| {
        // The converter ends its last line, which would split the line pasted into.
        html_to_markdown(&html).trim_end_matches('\n').to_owned()
    });
    converted
        .filter(|markdown| !markdown.is_empty())
        .or_else(|| text().map(|text| text.replace("\r\n", "\n").replace('\r', "\n")))
        .filter(|pasted| !pasted.is_empty())
}

/// [`paste_text`] from a clipboard.
pub fn paste_from(clipboard: &dyn Clipboard, plain: bool) -> Option<String> {
    paste_text(plain, || clipboard.read_html(), || clipboard.read_text())
}

/// What the clipboard holds, for enabling the Paste items. It is read when a menu with them
/// opens.
#[derive(Debug, Default)]
pub struct MenuClipboard {
    has_text: bool,
    has_html: bool,
    /// A menu with Paste was open in the last frame.
    menu_open: bool,
}

impl MenuClipboard {
    pub fn has_text(&self) -> bool {
        self.has_text
    }

    pub fn has_html(&self) -> bool {
        self.has_html
    }

    /// Call once a frame with whether a menu with Paste is open.
    pub fn watch(&mut self, ctx: &egui::Context, clipboard: &dyn Clipboard, menu_open: bool) {
        if menu_open && !self.menu_open {
            self.has_text = clipboard.read_text().is_some_and(|text| !text.is_empty());
            self.has_html = clipboard.read_html().is_some_and(|html| !html.is_empty());
            // The menu, already drawn, has to catch up.
            ctx.request_repaint();
        }
        self.menu_open = menu_open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pasted(plain: bool, html: Option<&str>, text: Option<&str>) -> Option<String> {
        paste_text(plain, || html.map(str::to_owned), || text.map(str::to_owned))
    }

    #[test]
    fn html_is_pasted_as_markdown_when_there_is_some() {
        let html = Some("<h1>Title</h1><p>Some <b>bold</b> words</p>");
        assert_eq!(pasted(false, html, Some("Title\r\nSome bold words")).as_deref(), Some("# Title\n\nSome **bold** words"));
        // A few words must not break the line they are pasted into.
        assert_eq!(pasted(false, Some("<i>two words</i>"), Some("two words")).as_deref(), Some("*two words*"));
    }

    #[test]
    fn without_html_the_text_is_pasted_with_plain_line_ends() {
        assert_eq!(pasted(false, None, Some("one\r\ntwo\rthree\n")).as_deref(), Some("one\ntwo\nthree\n"));
        assert_eq!(pasted(false, Some("<img src=\"a.png\">"), Some("a caption")).as_deref(), Some("a caption"));
        assert_eq!(pasted(false, None, None), None);
        assert_eq!(pasted(false, None, Some("")), None);
    }

    #[test]
    fn a_plain_text_paste_never_reads_the_html() {
        let html = || panic!("the HTML was read");
        assert_eq!(paste_text(true, html, || Some("<b>*as is*</b>".to_owned())).as_deref(), Some("<b>*as is*</b>"));
        assert_eq!(pasted(true, Some("<b>bold</b>"), None), None);
    }

    #[test]
    fn the_text_is_not_read_when_the_html_serves() {
        let text = || panic!("the text was read");
        assert_eq!(paste_text(false, || Some("<p>words</p>".to_owned()), text).as_deref(), Some("words"));
    }
}
