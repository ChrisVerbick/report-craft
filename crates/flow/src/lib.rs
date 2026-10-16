//! DesignCraft Flow: keeps linked text in step with its source file.
//!
//! [`diff_text`] finds what changed between two texts as [`Hunk`]s: byte ranges that start and end
//! on grapheme cluster boundaries, so a letter with its accents, an emoji or a flag is never split.
//! [`apply`] rebuilds the new text from the old one and those hunks, refusing hunks that don't line
//! up. Merging three texts and comparing two documents build on them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod merge;
mod story;

use std::ops::Range;

pub use merge::{Conflict, MergeOut, Segment, is_anchor, merge_text};
use similar::{Algorithm, DiffTag, TextDiff};
pub use story::{StoryMerge, merge_story};

/// One change from text `a` to text `b`: bytes `old` of `a` become bytes `new` of `b`. Both start
/// and end on grapheme cluster boundaries; one of them may be empty (an insertion or a deletion).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// The changes from `a` to `b`, in order.
///
/// Paragraphs are matched first (patience: lines found once in each text anchor the match), then
/// grapheme clusters inside each changed run of paragraphs. The finer diff takes time in proportion
/// to length × changes, so it never runs on a whole long story at once.
pub fn diff_text(a: &str, b: &str) -> Vec<Hunk> {
    // The offsets come from the diff itself; should one ever fall outside its text, a single hunk
    // replacing everything is still exact.
    by_paragraphs(a, b).unwrap_or_else(|| if a == b { Vec::new() } else { vec![Hunk { old: 0..a.len(), new: 0..b.len() }] })
}

fn by_paragraphs(a: &str, b: &str) -> Option<Vec<Hunk>> {
    let lines = TextDiff::configure().algorithm(Algorithm::Patience).diff_lines(a, b);
    let (la, lb) = (offsets(lines.iter_old_slices()), offsets(lines.iter_new_slices()));
    let mut out = Vec::new();
    for op in lines.ops().iter().filter(|op| op.tag() != DiffTag::Equal) {
        let (pa, pb) = (span(&la, op.old_range())?, span(&lb, op.new_range())?);
        let g = TextDiff::configure().algorithm(Algorithm::Myers).diff_graphemes(a.get(pa.clone())?, b.get(pb.clone())?);
        let (ga, gb) = (offsets(g.iter_old_slices()), offsets(g.iter_new_slices()));
        for op in g.ops().iter().filter(|op| op.tag() != DiffTag::Equal) {
            let (x, y) = (span(&ga, op.old_range())?, span(&gb, op.new_range())?);
            out.push(Hunk { old: pa.start + x.start..pa.start + x.end, new: pb.start + y.start..pb.start + y.end });
        }
    }
    Some(out)
}

/// Where each token starts in its text, and where the last one ends.
fn offsets<'s>(tokens: impl Iterator<Item = &'s str>) -> Vec<usize> {
    std::iter::once(0)
        .chain(tokens.scan(0, |at, t| {
            *at += t.len();
            Some(*at)
        }))
        .collect()
}

/// The bytes that tokens `r` cover.
fn span(offsets: &[usize], r: Range<usize>) -> Option<Range<usize>> {
    Some(*offsets.get(r.start)?..*offsets.get(r.end)?)
}

/// `a` rebuilt with `b`'s text in each hunk: equal to `b` exactly when the hunks cover every
/// difference between the two. `None` when the hunks are out of order or overlap, put an unchanged
/// stretch of `a` at another place in `b`, or cut through a character.
pub fn apply(a: &str, b: &str, hunks: &[Hunk]) -> Option<String> {
    let mut out = String::with_capacity(b.len());
    let (mut at, mut bt) = (0, 0);
    for h in hunks {
        let same = a.get(at..h.old.start)?;
        if h.new.start.checked_sub(bt)? != same.len() {
            return None;
        }
        a.get(h.old.clone())?;
        out.push_str(same);
        out.push_str(b.get(h.new.clone())?);
        (at, bt) = (h.old.end, h.new.end);
    }
    let rest = a.get(at..)?;
    if b.len().checked_sub(bt)? != rest.len() {
        return None;
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use unicode_segmentation::UnicodeSegmentation;

    use super::*;

    /// Pieces that stress grapheme handling: an accent as one character and as a combining mark,
    /// emoji with a skin tone and with joiners, a flag, CJK, CR LF, a soft hyphen, and the
    /// private-use characters stories use as markers.
    const PIECES: &[&str] =
        &["a", "b", " ", ".", "\n", "\r\n", "é", "e\u{301}", "中", "文", "👍🏽", "👨\u{200d}👩\u{200d}👧", "🇫🇷", "\u{AD}", "\u{E00A}", "\u{E010}"];

    fn pieces() -> impl Strategy<Value = Vec<usize>> {
        proptest::collection::vec(0..PIECES.len(), 0..40)
    }

    fn join(v: &[usize]) -> String {
        v.iter().map(|i| PIECES[*i]).collect()
    }

    fn boundaries(s: &str) -> Vec<usize> {
        s.grapheme_indices(true).map(|(i, _)| i).chain([s.len()]).collect()
    }

    fn check(a: &str, b: &str) -> Result<(), TestCaseError> {
        let hunks = diff_text(a, b);
        prop_assert_eq!(apply(a, b, &hunks), Some(b.to_string()));
        let (ga, gb) = (boundaries(a), boundaries(b));
        for h in &hunks {
            prop_assert!(!h.old.is_empty() || !h.new.is_empty(), "empty hunk {:?}", h);
            prop_assert!(ga.contains(&h.old.start) && ga.contains(&h.old.end), "{:?} splits a cluster of {:?}", h, a);
            prop_assert!(gb.contains(&h.new.start) && gb.contains(&h.new.end), "{:?} splits a cluster of {:?}", h, b);
        }
        if a == b {
            prop_assert!(hunks.is_empty());
        }
        Ok(())
    }

    proptest! {
        #[test]
        fn hunks_rebuild_any_new_text(a in pieces(), b in pieces()) {
            check(&join(&a), &join(&b))?;
        }

        /// Mostly equal texts, as revisions are: `b` is `a` with a few pieces inserted, deleted or
        /// replaced.
        #[test]
        fn hunks_rebuild_an_edited_text(a in pieces(), edits in proptest::collection::vec((any::<usize>(), 0..PIECES.len(), 0..3u8), 0..6)) {
            let mut b = a.clone();
            for (at, piece, kind) in edits {
                let at = at % (b.len() + 1);
                match kind {
                    1 if at < b.len() => {
                        b.remove(at);
                    }
                    2 if at < b.len() => b[at] = piece,
                    _ => b.insert(at, piece),
                }
            }
            check(&join(&a), &join(&b))?;
        }
    }

    #[test]
    fn a_word_added_is_one_small_hunk() {
        let (a, b) = ("The quick fox.\nSecond line.\n", "The quick brown fox.\nSecond line.\n");
        let hunks = diff_text(a, b);
        assert_eq!(hunks.len(), 1, "{hunks:?}");
        assert_eq!((hunks[0].old.len(), hunks[0].new.len()), (0, 6), "{hunks:?}");
    }

    #[test]
    fn hunks_that_dont_line_up_are_refused() {
        let (a, b) = ("abc", "abXc");
        assert_eq!(apply(a, b, &[Hunk { old: 2..2, new: 2..3 }]).as_deref(), Some("abXc"));
        assert_eq!(apply(a, b, &[Hunk { old: 1..1, new: 2..3 }]), None, "unchanged stretches out of step");
        assert_eq!(apply(a, b, &[Hunk { old: 2..2, new: 2..3 }, Hunk { old: 0..0, new: 0..0 }]), None, "out of order");
        assert_eq!(apply("é", "e", &[Hunk { old: 0..1, new: 0..1 }]), None, "cuts through a character");
    }
}
