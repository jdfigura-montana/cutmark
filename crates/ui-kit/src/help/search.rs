//! The help search (spec: S1 Help "Help window": "matches title, keywords and
//! body text, case-insensitive, titles and keywords ranked first"). Every
//! matching word is returned so the result list can highlight it.

use super::topic::HelpBook;

/// One matching topic and the words that matched in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The topic's index into [`HelpBook::topics`].
    pub topic: usize,
    /// The query words the topic matched, in query order, lowercased.
    pub words: Vec<String>,
    /// True when a word matched the title or a keyword, so the hit ranks
    /// above the body-only hits.
    pub title_or_keyword: bool,
}

/// Searches the book for `query`, case-insensitively. Every word must match
/// somewhere in the topic. Title and keyword hits come first; within a rank
/// the book's reading order is kept. An empty query returns every topic in
/// reading order, with no words to highlight.
pub fn search(book: &HelpBook, query: &str) -> Vec<Hit> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return book.topics.iter().enumerate().map(|(topic, _)| Hit { topic, words: Vec::new(), title_or_keyword: false }).collect();
    }
    let mut hits = Vec::new();
    for (index, topic) in book.topics.iter().enumerate() {
        let title = topic.title.to_lowercase();
        let keywords = topic.keywords.join(" ").to_lowercase();
        let body = topic.body.to_lowercase();
        let mut matched = Vec::new();
        let mut title_or_keyword = false;
        for word in &words {
            let in_title = title.contains(word.as_str());
            let in_keywords = keywords.contains(word.as_str());
            if in_title || in_keywords {
                title_or_keyword = true;
            } else if !body.contains(word.as_str()) {
                // Every word must match somewhere, so this topic is out.
                matched.clear();
                break;
            }
            matched.push(word.clone());
        }
        if matched.len() == words.len() {
            hits.push(Hit { topic: index, words: matched, title_or_keyword });
        }
    }
    hits.sort_by_key(|hit| (!hit.title_or_keyword, hit.topic));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A book with a title hit, a keyword hit and two body-only hits.
    fn book() -> HelpBook {
        HelpBook::from_sources(&[
            ("alpha.md", "---\nid: alpha\ntitle: Alpha\norder: 10\n---\nPress Snap to lock the beat.\n"),
            ("beta.md", "---\nid: beta\ntitle: Beta\norder: 20\nkeywords: snap, grid\n---\nNothing here.\n"),
            ("gamma.md", "---\nid: gamma\ntitle: Gamma\norder: 30\n---\nPress Snap again to release it.\n"),
            ("delta.md", "---\nid: delta\ntitle: Delta\norder: 40\n---\nAn unrelated topic.\n"),
        ])
        .expect("the test book is valid")
    }

    /// The ids of the hits, in the order search returned them.
    fn ids<'a>(book: &'a HelpBook, hits: &'a [Hit]) -> Vec<&'a str> {
        hits.iter().map(|hit| book.topics[hit.topic].id.as_str()).collect()
    }

    /// A title hit and a keyword hit both rank above the body-only hits, and
    /// the body-only hits keep reading order among themselves.
    #[test]
    fn titles_and_keywords_rank_first() {
        let book = book();
        let hits = search(&book, "snap");
        assert_eq!(ids(&book, &hits), ["beta", "alpha", "gamma"]);
        assert!(hits[0].title_or_keyword, "a keyword hit ranks first");
        assert!(!hits[1].title_or_keyword, "a body-only hit ranks later");
    }

    /// The search ignores case in the query and in the topic.
    #[test]
    fn search_is_case_insensitive() {
        let book = book();
        assert_eq!(ids(&book, &search(&book, "SNAP")), ["beta", "alpha", "gamma"]);
    }

    /// Every word must match, so a second word narrows the hits.
    #[test]
    fn every_word_must_match() {
        let book = book();
        let hits = search(&book, "snap grid");
        assert_eq!(ids(&book, &hits), ["beta"]);
        assert_eq!(hits[0].words, ["snap", "grid"]);
    }

    /// A word that appears in no topic returns no hits.
    #[test]
    fn unknown_words_return_no_hits() {
        assert!(search(&book(), "trombone").is_empty());
    }

    /// An empty query returns every topic in reading order, with no words.
    #[test]
    fn empty_query_returns_every_topic() {
        let book = book();
        let hits = search(&book, "  ");
        assert_eq!(hits.len(), book.len());
        assert!(hits.iter().all(|hit| hit.words.is_empty()));
        assert_eq!(ids(&book, &hits), ["alpha", "beta", "gamma", "delta"]);
    }
}
