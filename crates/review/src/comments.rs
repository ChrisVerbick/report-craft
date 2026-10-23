//! Comments as the review service sends them, and where their text is now.

use designcraft_doc::CellAddr;
use serde::{Deserialize, Serialize};

/// A comment from `comments.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Comment {
    pub id: String,
    /// The service's document and version ids.
    pub document: String,
    pub version: String,
    /// `pin`, `highlight`, `strike`, `replace`, `note` or `reply`.
    pub kind: String,
    pub page: i64,
    pub story: Option<u64>,
    pub cell: Option<CellAddr>,
    pub footnote: Option<u64>,
    /// UTF-8 byte range in the story (or the cell's or footnote's text) as reviewed.
    pub start: Option<u32>,
    pub end: Option<u32>,
    pub quote: Quote,
    pub rects: Option<Vec<[f64; 4]>>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub body: String,
    pub replacement: Option<String>,
    /// `open`, `resolved`, `applied` or `orphaned`.
    pub status: String,
    pub created: i64,
    pub modified: i64,
    pub in_reply_to: Option<String>,
    /// The review round the comment was made in, when rounds are used.
    pub round: Option<String>,
    /// A drawing's points, page points from the page's top-left corner.
    pub path: Option<Vec<[f64; 2]>>,
    pub author: Author,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quote {
    pub exact: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: String,
}

impl Comment {
    /// The cell or footnote the comment's text is in, as `text.select` takes it.
    pub fn address(&self) -> Option<CellAddr> {
        self.footnote.map(CellAddr::footnote).or(self.cell)
    }

    /// Where the comment's text is in `text` (the story's, cell's or footnote's current text).
    pub fn anchored_in(&self, text: &str) -> Option<Anchored> {
        let (start, end, exact) = (self.start?, self.end?, self.quote.exact.as_deref()?);
        Some(reanchor(
            text,
            start as usize,
            end as usize,
            exact,
            self.quote.prefix.as_deref().unwrap_or(""),
            self.quote.suffix.as_deref().unwrap_or(""),
        ))
    }
}

/// Where a comment's text is now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum Anchored {
    /// Still where the reviewer saw it.
    Found { start: usize, end: usize },
    /// Elsewhere in the text, found by its quote.
    Moved { start: usize, end: usize },
    /// Nowhere in the text any more.
    Orphaned,
}

impl Anchored {
    pub fn range(&self) -> Option<(usize, usize)> {
        match self {
            Anchored::Found { start, end } | Anchored::Moved { start, end } => Some((*start, *end)),
            Anchored::Orphaned => None,
        }
    }
}

/// Find `exact` in `text`: at `start..end` if it is still there, else by searching, with the
/// `prefix` and `suffix` the reviewer saw around it deciding between repeats (the nearest to
/// `start` when they can't).
pub fn reanchor(text: &str, start: usize, end: usize, exact: &str, prefix: &str, suffix: &str) -> Anchored {
    if exact.is_empty() {
        return Anchored::Orphaned;
    }
    if text.get(start..end) == Some(exact) {
        return Anchored::Found { start, end };
    }
    let score = |at: usize| {
        let before = text.get(..at).unwrap_or("");
        let after = text.get(at + exact.len()..).unwrap_or("");
        let p = before.chars().rev().zip(prefix.chars().rev()).take_while(|(a, b)| a == b).count();
        let s = after.chars().zip(suffix.chars()).take_while(|(a, b)| a == b).count();
        (p + s, std::cmp::Reverse(at.abs_diff(start)))
    };
    match text.match_indices(exact).map(|(i, _)| i).max_by_key(|&i| score(i)) {
        Some(at) => Anchored::Moved { start: at, end: at + exact.len() },
        None => Anchored::Orphaned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_still_in_place_is_found() {
        assert_eq!(reanchor("Hello review world", 6, 12, "review", "Hello ", " world"), Anchored::Found { start: 6, end: 12 });
    }

    #[test]
    fn text_that_moved_is_found_by_its_quote() {
        assert_eq!(reanchor("Oh, Hello review world", 6, 12, "review", "Hello ", " world"), Anchored::Moved { start: 10, end: 16 });
    }

    #[test]
    fn repeats_are_told_apart_by_their_context_then_by_distance() {
        let text = "the cat sat. the cat ran. the cat slept.";
        assert_eq!(reanchor(text, 2, 9, "the cat", "", " ran"), Anchored::Moved { start: 13, end: 20 });
        assert_eq!(reanchor(text, 28, 35, "the cat", "", " slept"), Anchored::Moved { start: 26, end: 33 });
        assert_eq!(reanchor(text, 24, 31, "the cat", "zzz", "zzz"), Anchored::Moved { start: 26, end: 33 }, "no context matches: the nearest");
    }

    #[test]
    fn text_that_is_gone_is_orphaned() {
        assert_eq!(reanchor("Nothing left", 0, 5, "Hello", "", ""), Anchored::Orphaned);
        assert_eq!(reanchor("Hello", 0, 0, "", "", ""), Anchored::Orphaned, "an empty quote anchors nothing");
    }

    #[test]
    fn a_comment_knows_its_address_and_anchor() {
        let mut c = Comment {
            story: Some(3),
            start: Some(0),
            end: Some(5),
            quote: Quote { exact: Some("Hello".into()), ..Default::default() },
            ..Default::default()
        };
        assert_eq!(c.anchored_in("Hello world"), Some(Anchored::Found { start: 0, end: 5 }));
        assert_eq!(c.address(), None);
        c.footnote = Some(7);
        assert_eq!(c.address(), Some(CellAddr::footnote(7)));
        let pin = Comment { x: Some(1.0), y: Some(2.0), ..Default::default() };
        assert_eq!(pin.anchored_in("Hello"), None);
    }
}
