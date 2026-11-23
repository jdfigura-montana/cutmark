//! The help topic model: the front matter parser and the `HelpBook` that
//! checks ids, sections and links (spec: S1 Help "Topic files").

use std::collections::BTreeMap;
use std::fmt;

/// The section a topic without a `section:` line falls in.
const UNFILED_SECTION: &str = "Other";
/// The order a topic without an `order:` line sorts by.
const DEFAULT_ORDER: u32 = 1_000;

/// One help topic: its front matter and its Markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topic {
    /// The id links use, for example `recording`.
    pub id: String,
    /// The title shown in the topic list and as the page heading.
    pub title: String,
    /// The section the topic is grouped under in the left column.
    pub section: String,
    /// Position inside its section: lower comes first.
    pub order: u32,
    /// Words the search ranks above the body.
    pub keywords: Vec<String>,
    /// F1 area tags this topic answers, for example `timeline.record`.
    pub context: Vec<String>,
    /// The Markdown body, without the front matter.
    pub body: String,
}

/// One problem found while building a help book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpError {
    /// A source has no `id:` line.
    MissingId { file: String },
    /// A source has no `title:` line.
    MissingTitle { file: String },
    /// Two sources use the same id.
    DuplicateId { id: String, files: [String; 2] },
    /// A `[text](topic:id)` link points at no topic.
    MissingTopic { from: String, target: String },
}

impl fmt::Display for HelpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingId { file } => write!(f, "{file}: front matter has no id"),
            Self::MissingTitle { file } => write!(f, "{file}: front matter has no title"),
            Self::DuplicateId { id, files } => write!(f, "duplicate topic id {id:?} in {} and {}", files[0], files[1]),
            Self::MissingTopic { from, target } => write!(f, "{from}: link to missing topic {target:?}"),
        }
    }
}

impl std::error::Error for HelpError {}

/// A topic's section in the left column, in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The section name, for example `Basics`.
    pub name: String,
    /// The section's topics as indices into [`HelpBook::topics`].
    pub topics: Vec<usize>,
}

/// The parsed and checked topics of one app, in reading order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpBook {
    /// Every topic, ordered by section order, then topic order, then title.
    pub topics: Vec<Topic>,
    /// The section groups, in display order.
    pub sections: Vec<Section>,
}

impl Topic {
    /// Parses one Markdown source with its front matter.
    pub fn parse(file: &str, contents: &str) -> Result<Topic, HelpError> {
        let (front, body) = split_front_matter(contents);
        let mut id = None;
        let mut title = None;
        let mut section = None;
        let mut order = DEFAULT_ORDER;
        let mut keywords = Vec::new();
        let mut context = Vec::new();
        for line in front.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else { continue };
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "id" => id = Some(value.to_string()),
                "title" => title = Some(value.to_string()),
                "section" => section = Some(value.to_string()),
                "order" => order = value.parse().unwrap_or(DEFAULT_ORDER),
                "keywords" => keywords = split_list(value),
                "context" => context = split_list(value),
                _ => {}
            }
        }
        let id = id.filter(|id| !id.is_empty()).ok_or_else(|| HelpError::MissingId { file: file.to_string() })?;
        let title = title.filter(|title| !title.is_empty()).ok_or_else(|| HelpError::MissingTitle { file: file.to_string() })?;
        let section = section.filter(|section| !section.is_empty()).unwrap_or_else(|| UNFILED_SECTION.to_string());
        Ok(Topic { id, title, section, order, keywords, context, body: body.trim().to_string() })
    }
}

impl HelpBook {
    /// Parses and checks every source. Every problem found is returned.
    pub fn from_sources(sources: &[(&str, &str)]) -> Result<HelpBook, Vec<HelpError>> {
        let mut errors = Vec::new();
        let mut parsed: Vec<(&str, Topic)> = Vec::new();
        for (file, contents) in sources {
            match Topic::parse(file, contents) {
                Ok(topic) => parsed.push((file, topic)),
                Err(error) => errors.push(error),
            }
        }
        for (index, (file, topic)) in parsed.iter().enumerate() {
            if let Some((other_file, _)) = parsed.iter().skip(index + 1).find(|(_, other)| other.id == topic.id) {
                errors.push(HelpError::DuplicateId { id: topic.id.clone(), files: [(*file).to_string(), (*other_file).to_string()] });
            }
        }
        let ids: Vec<&str> = parsed.iter().map(|(_, topic)| topic.id.as_str()).collect();
        for (file, topic) in &parsed {
            for target in super::markdown::link_targets(&topic.body) {
                if !ids.contains(&target.as_str()) {
                    errors.push(HelpError::MissingTopic { from: (*file).to_string(), target });
                }
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(HelpBook::assemble(parsed.into_iter().map(|(_, topic)| topic).collect()))
    }

    /// The number of topics in the book.
    pub fn len(&self) -> usize {
        self.topics.len()
    }

    /// True when the book has no topics.
    pub fn is_empty(&self) -> bool {
        self.topics.is_empty()
    }

    /// The topic with `id`, or None.
    pub fn get(&self, id: &str) -> Option<&Topic> {
        self.topics.iter().find(|topic| topic.id == id)
    }

    /// The first topic in reading order, or None.
    pub fn first(&self) -> Option<&Topic> {
        self.topics.first()
    }

    /// The topic whose context list holds `tag`; the longest matching entry wins.
    pub fn topic_for_context(&self, tag: &str) -> Option<&Topic> {
        let mut best: Option<(&Topic, usize)> = None;
        for topic in &self.topics {
            for entry in &topic.context {
                let Some(length) = context_match(tag, entry) else { continue };
                if best.is_none_or(|(_, best_length)| length > best_length) {
                    best = Some((topic, length));
                }
            }
        }
        best.map(|(topic, _)| topic)
    }

    /// Sorts the topics into reading order and groups them into sections.
    fn assemble(mut topics: Vec<Topic>) -> HelpBook {
        let mut section_orders: BTreeMap<String, u32> = BTreeMap::new();
        for topic in &topics {
            let entry = section_orders.entry(topic.section.clone()).or_insert(topic.order);
            *entry = (*entry).min(topic.order);
        }
        topics.sort_by(|a, b| {
            section_orders[&a.section]
                .cmp(&section_orders[&b.section])
                .then_with(|| a.section.cmp(&b.section))
                .then_with(|| a.order.cmp(&b.order))
                .then_with(|| a.title.cmp(&b.title))
        });
        let mut sections: Vec<Section> = Vec::new();
        for (index, topic) in topics.iter().enumerate() {
            match sections.last_mut() {
                Some(section) if section.name == topic.section => section.topics.push(index),
                _ => sections.push(Section { name: topic.section.clone(), topics: vec![index] }),
            }
        }
        HelpBook { topics, sections }
    }
}

/// The length of the longest matching context entry when `tag` sits under
/// `entry`: the tag itself, or a more specific tag below it.
fn context_match(tag: &str, entry: &str) -> Option<usize> {
    if tag == entry {
        return Some(entry.len());
    }
    let rest = tag.strip_prefix(entry)?;
    rest.starts_with('.').then_some(entry.len())
}

/// Splits the text into the front matter and the body. A source without the
/// opening `---` line has no front matter and no body split.
fn split_front_matter(contents: &str) -> (&str, &str) {
    let Some(after_open) = contents.strip_prefix("---").and_then(|rest| rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n'))) else {
        return ("", contents);
    };
    let mut offset = 0;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            return (&after_open[..offset], &after_open[offset + line.len()..]);
        }
        offset += line.len();
    }
    ("", contents)
}

/// Splits a comma-separated front matter value into trimmed, non-empty items.
fn split_list(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|item| !item.is_empty()).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GETTING_STARTED: &str = "---\nid: getting-started\ntitle: Getting started\nsection: Basics\norder: 10\nkeywords: start, overview\ncontext: getting-started\n---\n\n# Getting started\n\nPress F1 for help.\n";

    /// A topic's front matter fills every field and the body drops the front matter.
    #[test]
    fn front_matter_parses_every_field() {
        let topic = Topic::parse("getting-started.md", GETTING_STARTED).unwrap();
        assert_eq!(topic.id, "getting-started");
        assert_eq!(topic.title, "Getting started");
        assert_eq!(topic.section, "Basics");
        assert_eq!(topic.order, 10);
        assert_eq!(topic.keywords, ["start", "overview"]);
        assert_eq!(topic.context, ["getting-started"]);
        assert!(topic.body.starts_with("# Getting started"));
        assert!(!topic.body.contains("id: getting-started"));
    }

    /// A topic without a section lands in "Other" and keeps the default order.
    #[test]
    fn missing_section_and_order_get_defaults() {
        let topic = Topic::parse("plain.md", "---\nid: plain\ntitle: Plain\n---\nBody.\n").unwrap();
        assert_eq!(topic.section, UNFILED_SECTION);
        assert_eq!(topic.order, DEFAULT_ORDER);
        assert!(topic.keywords.is_empty());
        assert!(topic.context.is_empty());
    }

    /// A source without an id or without a title is an error.
    #[test]
    fn missing_id_and_title_are_errors() {
        let no_id = Topic::parse("no-id.md", "---\ntitle: No id\n---\nBody.\n").unwrap_err();
        assert_eq!(no_id, HelpError::MissingId { file: "no-id.md".to_string() });
        let no_title = Topic::parse("no-title.md", "---\nid: no-title\n---\nBody.\n").unwrap_err();
        assert_eq!(no_title, HelpError::MissingTitle { file: "no-title.md".to_string() });
    }

    /// Two topics with one id are rejected and both files are named.
    #[test]
    fn duplicate_ids_are_errors() {
        let first = "---\nid: same\ntitle: First\n---\n";
        let second = "---\nid: same\ntitle: Second\n---\n";
        let errors = HelpBook::from_sources(&[("first.md", first), ("second.md", second)]).unwrap_err();
        assert_eq!(errors, [HelpError::DuplicateId { id: "same".to_string(), files: ["first.md".to_string(), "second.md".to_string()] }]);
    }

    /// A link to a topic that is not in the book is an error.
    #[test]
    fn broken_links_are_errors() {
        let source = "---\nid: one\ntitle: One\n---\nSee [Two](topic:two).\n";
        let errors = HelpBook::from_sources(&[("one.md", source)]).unwrap_err();
        assert_eq!(errors, [HelpError::MissingTopic { from: "one.md".to_string(), target: "two".to_string() }]);
    }

    /// A valid book sorts by section order, then topic order, then title.
    #[test]
    fn sections_and_topics_sort_in_reading_order() {
        let basics = "---\nid: basics\ntitle: Basics\nsection: Basics\norder: 10\n---\n";
        let later = "---\nid: later\ntitle: Later\nsection: Basics\norder: 20\n---\n";
        let look = "---\nid: look\ntitle: Look\nsection: Look\norder: 5\n---\n";
        let book = HelpBook::from_sources(&[("basics.md", basics), ("later.md", later), ("look.md", look)]).unwrap();
        assert_eq!(book.topics.iter().map(|topic| topic.id.as_str()).collect::<Vec<_>>(), ["look", "basics", "later"]);
        assert_eq!(book.sections.iter().map(|section| section.name.as_str()).collect::<Vec<_>>(), ["Look", "Basics"]);
        assert_eq!(book.sections[1].topics, [1, 2]);
    }

    /// A more specific context tag beats its parent, and the plain parent still matches.
    #[test]
    fn context_specificity_prefers_the_longest_match() {
        let timeline = "---\nid: timeline\ntitle: Timeline\ncontext: timeline\n---\n";
        let record = "---\nid: record\ntitle: Record\ncontext: timeline.record\n---\n";
        let book = HelpBook::from_sources(&[("timeline.md", timeline), ("record.md", record)]).unwrap();
        assert_eq!(book.topic_for_context("timeline.record").map(|topic| topic.id.as_str()), Some("record"));
        assert_eq!(book.topic_for_context("timeline").map(|topic| topic.id.as_str()), Some("timeline"));
        assert_eq!(book.topic_for_context("timeline.record.arm").map(|topic| topic.id.as_str()), Some("record"));
        assert_eq!(book.topic_for_context("timelinex"), None);
    }
}
