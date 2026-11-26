//! .docx to Markdown. Paragraphs, headings, bold, italic, underline, strikethrough, lists
//! and line breaks are read from `word/document.xml`; everything else is dropped. A table's
//! paragraphs are kept as paragraphs.
//!
//! Only what a paragraph or run says itself is read. Formatting and numbering that come
//! from a style in `word/styles.xml` are not.

use std::collections::HashMap;

use super::package::{DocumentError, Element, Node, Package};
use super::{Item, MAX_NUMBER, Reader, Style, write};

/// Word has nine list levels, 0 to 8.
const MAX_LEVEL: usize = 8;

/// Elements with no text of the document: what tracked changes removed or moved away,
/// drawings and text boxes, the second copy of alternate content, and properties.
const SKIPPED: [&str; 9] = ["mc:Fallback", "w:del", "w:drawing", "w:moveFrom", "w:object", "w:pPr", "w:pict", "w:rPr", "w:txbxContent"];

/// One level of a list definition.
#[derive(Debug, Clone, Copy)]
struct ListLevel {
    bullet: bool,
    start: u64,
}

/// A `w:num`: the list definition it uses, and what it changes in it.
#[derive(Debug, Default)]
struct Num {
    definition: String,
    levels: HashMap<usize, ListLevel>,
    starts: HashMap<usize, u64>,
}

/// `word/numbering.xml`, and how far each list has counted.
#[derive(Debug, Default)]
struct Numbering {
    definitions: HashMap<String, HashMap<usize, ListLevel>>,
    nums: HashMap<String, Num>,
    /// The next number for each list and level.
    next: HashMap<(String, usize), u64>,
}

/// Converts the bytes of a .docx file to the Markdown a script uses.
pub(crate) fn docx_to_markdown(bytes: &[u8]) -> Result<String, DocumentError> {
    let mut package = Package::open(bytes)?;
    let document = package.part("word/document.xml")?.ok_or(DocumentError::Damaged)?;
    let body = document.child("w:body").filter(|_| document.name == "w:document").ok_or(DocumentError::Damaged)?;
    let mut numbering = package.part("word/numbering.xml")?.map(|root| Numbering::read(&root)).unwrap_or_default();
    let mut reader = Reader::default();
    blocks(&mut reader, &mut numbering, body);
    Ok(write(&reader.blocks))
}

/// Finds the paragraphs under `element`, in tables and content controls too.
fn blocks(reader: &mut Reader, numbering: &mut Numbering, element: &Element) {
    for child in element.elements() {
        if child.name == "w:p" {
            paragraph(reader, numbering, child);
        } else if !SKIPPED.contains(&child.name.as_str()) {
            blocks(reader, numbering, child);
        }
    }
}

fn paragraph(reader: &mut Reader, numbering: &mut Numbering, paragraph: &Element) {
    let properties = paragraph.child("w:pPr");
    let named = properties.and_then(|properties| value(properties, "w:pStyle"));
    reader.heading = named.and_then(heading_level);
    // A heading stays a heading even if it is numbered.
    reader.item = match reader.heading {
        Some(_) => None,
        None => properties.and_then(|properties| properties.child("w:numPr")).and_then(|list| numbering.item(list)),
    };
    inline(reader, paragraph, Style::default());
    reader.flush();
    reader.heading = None;
    reader.item = None;
}

fn heading_level(style: &str) -> Option<usize> {
    if style == "Title" {
        return Some(1);
    }
    let level = style.strip_prefix("Heading")?.parse::<usize>().ok()?;
    (1..=6).contains(&level).then_some(level)
}

/// Reads the text of a paragraph: its runs, and those inside links, fields and insertions.
fn inline(reader: &mut Reader, element: &Element, style: Style) {
    for child in element.elements() {
        match child.name.as_str() {
            "w:r" => inline(reader, child, run_style(child)),
            // Only `w:t` holds text. Field codes and deleted text have elements of their own.
            "w:t" => {
                for node in &child.children {
                    if let Node::Text(text) = node {
                        reader.push_str(text, style);
                    }
                }
            }
            "w:br" | "w:cr" => reader.push_break(),
            "w:tab" => reader.push(' ', style),
            // Part of a word, so dropping it would change the word.
            "w:noBreakHyphen" => reader.push('-', style),
            name if SKIPPED.contains(&name) => {}
            _ => inline(reader, child, style),
        }
    }
}

fn run_style(run: &Element) -> Style {
    let properties = run.child("w:rPr");
    let property = |name: &str| properties.and_then(|properties| properties.child(name));
    let on = |name: &str| property(name).is_some_and(|property| !matches!(property.attr("w:val"), Some("0" | "false" | "off")));
    // The value of `w:u` is a kind of line, not on or off.
    let underline = property("w:u").is_some_and(|property| property.attr("w:val") != Some("none"));
    Style { bold: on("w:b"), italic: on("w:i"), underline, strike: on("w:strike") }
}

/// The `w:val` of a child element.
fn value<'a>(element: &'a Element, child: &str) -> Option<&'a str> {
    element.child(child)?.attr("w:val")
}

fn level_of(element: &Element) -> Option<usize> {
    element.attr("w:ilvl")?.parse().ok()
}

fn list_level(level: &Element) -> ListLevel {
    ListLevel {
        // A level with no format is numbered; "none" shows no number.
        bullet: matches!(value(level, "w:numFmt"), Some("bullet" | "none")),
        start: value(level, "w:start").and_then(|start| start.parse().ok()).unwrap_or(1),
    }
}

impl Numbering {
    fn read(root: &Element) -> Self {
        let mut numbering = Self::default();
        for element in root.elements() {
            match element.name.as_str() {
                "w:abstractNum" => {
                    let Some(id) = element.attr("w:abstractNumId") else { continue };
                    let levels = element.elements().filter(|child| child.name == "w:lvl");
                    let levels = levels.filter_map(|level| Some((level_of(level)?, list_level(level)))).collect();
                    numbering.definitions.insert(id.to_owned(), levels);
                }
                "w:num" => {
                    let (Some(id), Some(definition)) = (element.attr("w:numId"), value(element, "w:abstractNumId")) else { continue };
                    let mut num = Num { definition: definition.to_owned(), ..Num::default() };
                    for changed in element.elements().filter(|child| child.name == "w:lvlOverride") {
                        let Some(at) = level_of(changed) else { continue };
                        if let Some(level) = changed.child("w:lvl") {
                            num.levels.insert(at, list_level(level));
                        }
                        if let Some(start) = value(changed, "w:startOverride").and_then(|start| start.parse().ok()) {
                            num.starts.insert(at, start);
                        }
                    }
                    numbering.nums.insert(id.to_owned(), num);
                }
                _ => {}
            }
        }
        numbering
    }

    /// The list item a `w:numPr` stands for, counting it. A list that is not defined is a
    /// bullet list.
    fn item(&mut self, properties: &Element) -> Option<Item> {
        let id = value(properties, "w:numId")?;
        // Word writes 0 to take a paragraph out of a list.
        if id == "0" {
            return None;
        }
        let depth = value(properties, "w:ilvl").and_then(|level| level.parse::<usize>().ok()).unwrap_or(0).min(MAX_LEVEL);
        let num = self.nums.get(id);
        let defined = num.and_then(|num| num.levels.get(&depth).or_else(|| self.definitions.get(&num.definition)?.get(&depth))).copied();
        // An item restarts the lists under it.
        self.next.retain(|(list, level), _| list != id || *level <= depth);
        let number = defined.filter(|level| !level.bullet).map(|level| {
            let start = num.and_then(|num| num.starts.get(&depth)).copied().unwrap_or(level.start);
            let next = self.next.entry((id.to_owned(), depth)).or_insert(start.min(MAX_NUMBER));
            let number = *next;
            *next = number.saturating_add(1).min(MAX_NUMBER);
            number
        });
        Some(Item { depth, number })
    }
}
