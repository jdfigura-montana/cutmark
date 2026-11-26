//! The zipped XML inside a .docx or .odt file.

use std::io::{Cursor, Read};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use zip::ZipArchive;
use zip::result::ZipError;

use crate::store::decode;

/// The largest part that is unpacked. A script at the word limit needs far less, and a
/// small file can unpack to any size.
const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;

/// The most elements one part may hold. Each takes far more memory in the tree than the few
/// bytes it can be written in.
const MAX_ELEMENTS: usize = 4_000_000;

/// How many elements deep a part may go. The converters recurse over the tree, so this keeps
/// a hostile file from overflowing the stack.
const MAX_DEPTH: usize = 256;

/// How an OLE file starts. Word wraps a password-protected .docx in one. The old .doc
/// format is one too.
const OLE_MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

/// Why a .docx or .odt file could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DocumentError {
    #[error("This file is damaged or is not really a .docx or .odt file. Open it in your word processor, save a new copy and import again.")]
    Damaged,
    #[error("This file is password-protected or in an older Word format. Save it as .docx or .odt without a password and import again.")]
    Protected,
    #[error("This document is too large to import.")]
    TooLarge,
}

#[derive(Debug, Default)]
pub(super) struct Element {
    /// The name with its prefix, such as `w:p`.
    pub name: String,
    attributes: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug)]
pub(super) enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
    }

    /// The first child element with this name.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|element| element.name == name)
    }

    fn push_text(&mut self, text: &str) {
        match self.children.last_mut() {
            Some(Node::Text(last)) => last.push_str(text),
            _ => self.children.push(Node::Text(text.to_owned())),
        }
    }
}

/// An open .docx or .odt file.
pub(super) struct Package<'a> {
    archive: ZipArchive<Cursor<&'a [u8]>>,
}

impl<'a> Package<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, DocumentError> {
        if bytes.starts_with(&OLE_MAGIC) {
            return Err(DocumentError::Protected);
        }
        let archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| DocumentError::Damaged)?;
        Ok(Self { archive })
    }

    /// The root element of the XML part with this name, or `None` if there is no such part.
    pub fn part(&mut self, name: &str) -> Result<Option<Element>, DocumentError> {
        let file = match self.archive.by_name(name) {
            Ok(file) => file,
            Err(ZipError::FileNotFound) => return Ok(None),
            Err(ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED)) => return Err(DocumentError::Protected),
            Err(_) => return Err(DocumentError::Damaged),
        };
        let mut bytes = Vec::new();
        // The size the zip states cannot be trusted, so the limit is on what comes out.
        file.take(MAX_PART_BYTES + 1).read_to_end(&mut bytes).map_err(|_| DocumentError::Damaged)?;
        if bytes.len() as u64 > MAX_PART_BYTES {
            return Err(DocumentError::TooLarge);
        }
        let xml = decode(bytes).ok_or(DocumentError::Damaged)?;
        parse(&xml).map(Some)
    }
}

/// Reads XML into a tree. Comments, processing instructions and the DTD are skipped, so no
/// entity the file defines is ever expanded.
fn parse(xml: &str) -> Result<Element, DocumentError> {
    let mut reader = Reader::from_str(xml);
    // The element being read. At first it stands for the document itself.
    let mut current = Element::default();
    let mut parents: Vec<Element> = Vec::new();
    let mut count = 0_usize;
    loop {
        let event = reader.read_event().map_err(|_| DocumentError::Damaged)?;
        if matches!(event, Event::Start(_) | Event::Empty(_)) {
            count += 1;
            if count > MAX_ELEMENTS {
                return Err(DocumentError::TooLarge);
            }
        }
        match event {
            Event::Start(tag) => {
                if parents.len() >= MAX_DEPTH {
                    return Err(DocumentError::Damaged);
                }
                parents.push(std::mem::replace(&mut current, element(&tag)?));
            }
            Event::Empty(tag) => current.children.push(Node::Element(element(&tag)?)),
            Event::End(_) => {
                let parent = parents.pop().ok_or(DocumentError::Damaged)?;
                let done = std::mem::replace(&mut current, parent);
                current.children.push(Node::Element(done));
            }
            Event::Text(text) => current.push_text(&text),
            Event::CData(data) => current.push_text(&data),
            Event::GeneralRef(entity) => match entity.resolve_char_ref().map_err(|_| DocumentError::Damaged)? {
                Some(c) => current.push_text(c.encode_utf8(&mut [0; 4])),
                None => current.push_text(resolve_predefined_entity(&entity).ok_or(DocumentError::Damaged)?),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    // A file cut off in the middle ends with elements still open.
    if !parents.is_empty() {
        return Err(DocumentError::Damaged);
    }
    let root = current.children.into_iter().find_map(|node| match node {
        Node::Element(element) => Some(element),
        Node::Text(_) => None,
    });
    root.ok_or(DocumentError::Damaged)
}

fn element(tag: &BytesStart<'_>) -> Result<Element, DocumentError> {
    let mut attributes = Vec::new();
    for attribute in tag.attributes() {
        let attribute = attribute.map_err(|_| DocumentError::Damaged)?;
        let value = attribute.normalized_value(XmlVersion::Implicit1_0).map_err(|_| DocumentError::Damaged)?;
        attributes.push((attribute.key.0.to_owned(), value.into_owned()));
    }
    Ok(Element { name: tag.name().0.to_owned(), attributes, children: Vec::new() })
}
