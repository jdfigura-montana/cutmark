//! .odt to Markdown. Paragraphs, headings, lists, line breaks, and the bold, italic,
//! underline and strikethrough of spans are read from `content.xml`; everything else is
//! dropped. A table's paragraphs are kept as paragraphs.
//!
//! `styles.xml` is read only for the list styles that have a name, so that their numbered
//! lists stay numbered.

use std::collections::HashMap;

use super::package::{DocumentError, Element, Node, Package};
use super::{Item, MAX_NUMBER, Reader, Style, write};

/// Elements whose paragraphs and text are not the document's: comments, footnotes, deleted
/// text, and drawings with their frames and text boxes.
const SKIPPED: [&str; 9] =
    ["draw:a", "draw:custom-shape", "draw:frame", "draw:g", "office:annotation", "text:note", "text:ruby-text", "text:sequence-decls", "text:tracked-changes"];

/// What an automatic text style says. `None` leaves what the text already has.
#[derive(Debug, Clone, Copy, Default)]
struct TextStyle {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
}

/// For each level of a list style, counted from 1, the first number, or `None` for a bullet.
type ListStyle = HashMap<usize, Option<u64>>;

#[derive(Debug, Default)]
struct Odt {
    reader: Reader,
    text_styles: HashMap<String, TextStyle>,
    list_styles: HashMap<String, ListStyle>,
    /// The style name of each open list.
    open: Vec<String>,
    /// The next number for each list style and level.
    next: HashMap<(String, usize), u64>,
}

/// Converts the bytes of an .odt file to the Markdown a script uses.
pub(crate) fn odt_to_markdown(bytes: &[u8]) -> Result<String, DocumentError> {
    let mut package = Package::open(bytes)?;
    // The zip of a protected .odt opens, but its parts are encrypted. The manifest says so.
    if package.part("META-INF/manifest.xml")?.is_some_and(|manifest| is_encrypted(&manifest)) {
        return Err(DocumentError::Protected);
    }
    let content = package.part("content.xml")?.ok_or(DocumentError::Damaged)?;
    let body = content.child("office:body").and_then(|body| body.child("office:text"));
    let body = body.filter(|_| content.name == "office:document-content").ok_or(DocumentError::Damaged)?;

    let mut odt = Odt::default();
    if let Some(styles) = package.part("styles.xml")?
        && let Some(named) = styles.child("office:styles")
    {
        odt.read_styles(named);
    }
    if let Some(automatic) = content.child("office:automatic-styles") {
        odt.read_styles(automatic);
    }
    odt.children(body, Style::default());
    odt.reader.flush();
    Ok(write(&odt.reader.blocks))
}

fn is_encrypted(manifest: &Element) -> bool {
    manifest.name == "manifest:encryption-data" || manifest.elements().any(is_encrypted)
}

/// The number in an attribute, no larger than Markdown can write.
fn number(element: &Element, name: &str) -> Option<u64> {
    element.attr(name)?.parse::<u64>().ok().map(|number| number.min(MAX_NUMBER))
}

fn text_style(style: &Element) -> TextStyle {
    let Some(properties) = style.child("style:text-properties") else {
        return TextStyle::default();
    };
    let weight = |weight: &str| weight == "bold" || weight.parse::<u16>().is_ok_and(|weight| weight >= 600);
    TextStyle {
        bold: properties.attr("fo:font-weight").map(weight),
        italic: properties.attr("fo:font-style").map(|style| matches!(style, "italic" | "oblique")),
        underline: properties.attr("style:text-underline-style").map(|line| line != "none"),
        strike: properties.attr("style:text-line-through-style").map(|line| line != "none"),
    }
}

fn list_style(style: &Element) -> ListStyle {
    let mut levels = ListStyle::new();
    for level in style.elements() {
        let Some(at) = level.attr("text:level").and_then(|at| at.parse().ok()) else { continue };
        // A number level with an empty format shows no number.
        let numbered = level.name == "text:list-level-style-number" && level.attr("style:num-format") != Some("");
        levels.insert(at, numbered.then(|| number(level, "text:start-value").unwrap_or(1)));
    }
    levels
}

impl Odt {
    fn read_styles(&mut self, styles: &Element) {
        for style in styles.elements() {
            let Some(name) = style.attr("style:name") else { continue };
            if style.name == "text:list-style" {
                self.list_styles.insert(name.to_owned(), list_style(style));
            } else if style.name == "style:style" && style.attr("style:family") == Some("text") {
                self.text_styles.insert(name.to_owned(), text_style(style));
            }
        }
    }

    /// Reads the elements outside a paragraph. Text between them is only layout.
    fn children(&mut self, element: &Element, style: Style) {
        for child in element.elements() {
            self.element(child, style);
        }
    }

    /// Reads what is inside a paragraph or heading, text included.
    fn content(&mut self, element: &Element, style: Style) {
        for node in &element.children {
            match node {
                Node::Element(child) => self.inline(child, style),
                Node::Text(text) => self.reader.push_str(text, style),
            }
        }
    }

    /// An element outside a paragraph.
    fn element(&mut self, element: &Element, style: Style) {
        match element.name.as_str() {
            "text:p" => {
                self.reader.boundary();
                self.content(element, style);
                self.reader.boundary();
            }
            "text:h" => self.heading(element, style),
            "text:list" => self.list(element, style),
            "text:list-item" => self.list_item(element, style),
            name if SKIPPED.contains(&name) => {}
            // Sections, tables, indexes and the like.
            _ => self.children(element, style),
        }
    }

    /// An element inside a paragraph or heading.
    fn inline(&mut self, element: &Element, style: Style) {
        match element.name.as_str() {
            "text:span" => {
                let named = element.attr("text:style-name").and_then(|name| self.text_styles.get(name)).copied().unwrap_or_default();
                let style = Style {
                    bold: named.bold.unwrap_or(style.bold),
                    italic: named.italic.unwrap_or(style.italic),
                    underline: named.underline.unwrap_or(style.underline),
                    strike: named.strike.unwrap_or(style.strike),
                };
                self.content(element, style);
            }
            // A run of spaces and a tab both come out as one space.
            "text:s" | "text:tab" => self.reader.push(' ', style),
            "text:line-break" => self.reader.push_break(),
            name if SKIPPED.contains(&name) => {}
            // Links and fields are their text.
            _ => self.content(element, style),
        }
    }

    /// A heading stays a heading even inside a list, and leaves the item's marker for the
    /// text after it.
    fn heading(&mut self, element: &Element, style: Style) {
        self.reader.flush();
        let item = self.reader.item.take();
        let level = element.attr("text:outline-level").and_then(|level| level.parse::<usize>().ok()).unwrap_or(1);
        self.reader.heading = Some(level.clamp(1, 6));
        for node in &element.children {
            match node {
                // The heading's number, where the file has it written out.
                Node::Element(child) if child.name == "text:number" => {
                    self.content(child, style);
                    self.reader.push(' ', style);
                }
                Node::Element(child) => self.inline(child, style),
                Node::Text(text) => self.reader.push_str(text, style),
            }
        }
        self.reader.flush();
        self.reader.heading = None;
        self.reader.item = item;
    }

    fn list(&mut self, list: &Element, style: Style) {
        self.reader.flush();
        let outer = self.reader.item.take();
        // A nested list without a style of its own uses the one around it.
        let name = list.attr("text:style-name").map(str::to_owned).or_else(|| self.open.last().cloned()).unwrap_or_default();
        self.open.push(name.clone());
        let continues = list.attr("text:continue-list").is_some() || list.attr("text:continue-numbering") == Some("true");
        if !continues {
            self.next.remove(&(name, self.open.len()));
        }
        self.children(list, style);
        self.reader.flush();
        self.open.pop();
        self.reader.item = outer;
    }

    fn list_item(&mut self, element: &Element, style: Style) {
        let level = self.open.len().max(1);
        let name = self.open.last().cloned().unwrap_or_default();
        let first = self.list_styles.get(&name).and_then(|levels| levels.get(&level)).copied().flatten();
        let number = first.map(|first| {
            let next = self.next.entry((name, level)).or_insert(first);
            if let Some(restart) = number(element, "text:start-value") {
                *next = restart;
            }
            let number = *next;
            *next = number.saturating_add(1).min(MAX_NUMBER);
            number
        });
        self.reader.flush();
        let outer = self.reader.item.replace(Item { depth: level - 1, number });
        for child in element.elements() {
            // A written-out number would repeat the marker.
            if child.name != "text:number" {
                self.element(child, style);
            }
        }
        self.reader.flush();
        self.reader.item = outer;
    }
}
