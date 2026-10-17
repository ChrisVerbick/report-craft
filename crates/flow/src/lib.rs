//! DesignCraft Flow: keeps linked text in step with its source file.
//!
//! [`diff_text`] finds what changed between two texts as [`Hunk`]s: byte ranges that start and end
//! on grapheme cluster boundaries, so a letter with its accents, an emoji or a flag is never split.
//! [`apply`] rebuilds the new text from the old one and those hunks, refusing hunks that don't line
//! up. Merging three texts and comparing two documents build on them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod compare;
mod merge;
mod story;

use std::ops::Range;

pub use compare::{ChangeSet, Place, Status, StoryChanges, TableChange, TableStatus, TextChanges, compare};
pub use merge::{Conflict, MergeOut, Segment, is_anchor, merge_text};
use similar::{Algorithm, ChangeTag, DiffTag, TextDiff};
pub use story::{CellMerge, StoryMerge, merge_story, verified};
use unicode_segmentation::UnicodeSegmentation;

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

/// `hunks` (from `a` to `b`) widened to whole sentences of `a` and joined where they overlap: what a
/// person reads in a report. Still exact: should the groups ever fail to rebuild `b`, the hunks
/// come back as they were.
pub fn sentences(a: &str, b: &str, hunks: &[Hunk]) -> Vec<Hunk> {
    match by_sentence(a, hunks) {
        Some(groups) if apply(a, b, &groups).as_deref() == Some(b) => groups,
        _ => hunks.to_vec(),
    }
}

fn by_sentence(a: &str, hunks: &[Hunk]) -> Option<Vec<Hunk>> {
    let bounds: Vec<usize> = a.split_sentence_bound_indices().map(|(at, _)| at).chain([a.len()]).collect();
    let mut out: Vec<Hunk> = Vec::new();
    for h in hunks {
        // An insertion between sentences stays where it is; anything else takes in its sentences,
        // and the text it takes in is unchanged, the same in `b`.
        let (start, end) = if h.old.is_empty() && bounds.binary_search(&h.old.start).is_ok() {
            (h.old.start, h.old.end)
        } else {
            let start = bounds.get(bounds.partition_point(|x| *x <= h.old.start).wrapping_sub(1)).copied().unwrap_or(0);
            (start, bounds.get(bounds.partition_point(|x| *x < h.old.end)).copied().unwrap_or(h.old.end).max(h.old.end))
        };
        let new_end = h.new.end.checked_add(end.checked_sub(h.old.end)?)?;
        match out.last_mut() {
            // Joined: the group ends where the last hunk's sentence ends, as that hunk maps it.
            Some(last) if start < last.old.end => {
                if end >= last.old.end {
                    (last.old.end, last.new.end) = (end, new_end);
                }
            }
            _ => out.push(Hunk { old: start..end, new: h.new.start.checked_sub(h.old.start.checked_sub(start)?)?..new_end }),
        }
    }
    Some(out)
}

/// `s` with its marker characters shown as labels (`[note]`, `[footnote]`…), for a report.
pub fn shown(s: &str) -> String {
    use designcraft_doc::{ANCHOR_MARK, ENDNOTE_REF, FOOTNOTE_REF, INDEX_MARK, NOTE_MARK, OBJECT_MARK, TABLE_ANCHOR, XREF_MARK};
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let label = match c {
            NOTE_MARK => "[note]",
            FOOTNOTE_REF => "[footnote]",
            ENDNOTE_REF => "[endnote]",
            OBJECT_MARK => "[object]",
            TABLE_ANCHOR => "[table]",
            ANCHOR_MARK => "[anchor]",
            XREF_MARK => "[cross-reference]",
            INDEX_MARK => "[index entry]",
            '\u{E000}'..='\u{E012}' => "[special character]",
            _ => {
                out.push(c);
                continue;
            }
        };
        out.push_str(label);
    }
    out
}

/// A run of [`words`]: unchanged, only in the old text, or only in the new.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Same,
    Deleted,
    Inserted,
}

impl Mark {
    pub fn name(self) -> &'static str {
        match self {
            Mark::Same => "same",
            Mark::Deleted => "deleted",
            Mark::Inserted => "inserted",
        }
    }
}

/// How the text `a` became `b`, word by word, for marking a reported change: in order, the
/// unchanged runs and the runs only `a` has (deleted) or only `b` has (inserted). The unchanged and
/// deleted runs make `a`; the unchanged and inserted runs make `b`. Spaces and punctuation left
/// between changed words join them, so a rewritten stretch reads as one deletion and one insertion.
/// Texts over 20,000 bytes together aren't compared word by word: all of `a` is deleted and all of
/// `b` inserted.
pub fn words(a: &str, b: &str) -> Vec<(Mark, String)> {
    fn push(out: &mut Vec<(Mark, String)>, m: Mark, t: &str) {
        if t.is_empty() {
            return;
        }
        match out.last_mut() {
            Some((last, text)) if *last == m => text.push_str(t),
            _ => out.push((m, t.to_string())),
        }
    }
    let mut out = Vec::new();
    if a == b {
        push(&mut out, Mark::Same, a);
    } else if a.len().saturating_add(b.len()) > 20_000 {
        push(&mut out, Mark::Deleted, a);
        push(&mut out, Mark::Inserted, b);
    } else {
        let mut runs = Vec::new();
        for c in TextDiff::configure().algorithm(Algorithm::Patience).diff_unicode_words(a, b).iter_all_changes() {
            let m = match c.tag() {
                ChangeTag::Equal => Mark::Same,
                ChangeTag::Delete => Mark::Deleted,
                ChangeTag::Insert => Mark::Inserted,
            };
            push(&mut runs, m, c.value());
        }
        // Each unchanged run without a letter or digit between two changes joins them: it goes
        // into both the deletion and the insertion.
        let (mut del, mut ins) = (String::new(), String::new());
        for (i, (m, t)) in runs.iter().enumerate() {
            match m {
                Mark::Deleted => del.push_str(t),
                Mark::Inserted => ins.push_str(t),
                Mark::Same if (!del.is_empty() || !ins.is_empty()) && i + 1 < runs.len() && !t.chars().any(char::is_alphanumeric) => {
                    del.push_str(t);
                    ins.push_str(t);
                }
                Mark::Same => {
                    push(&mut out, Mark::Deleted, &del);
                    push(&mut out, Mark::Inserted, &ins);
                    (del, ins) = (String::new(), String::new());
                    push(&mut out, Mark::Same, t);
                }
            }
        }
        push(&mut out, Mark::Deleted, &del);
        push(&mut out, Mark::Inserted, &ins);
    }
    out
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
        // The grouping itself, not its fallback: it must rebuild `b` exactly.
        prop_assert_eq!(by_sentence(a, &hunks).and_then(|g| apply(a, b, &g)), Some(b.to_string()), "sentence groups stay exact");
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

    #[test]
    fn words_mark_what_changed_word_by_word() {
        let w = |v: &[(Mark, &str)]| v.iter().map(|(m, t)| (*m, t.to_string())).collect::<Vec<_>>();
        assert_eq!(
            words("We expect steady growth.", "We expect modest growth."),
            w(&[(Mark::Same, "We expect "), (Mark::Deleted, "steady"), (Mark::Inserted, "modest"), (Mark::Same, " growth.")])
        );
        assert_eq!(words("3.50", "3.75"), w(&[(Mark::Deleted, "3.50"), (Mark::Inserted, "3.75")]), "a number is one word");
        assert_eq!(words("", "New."), w(&[(Mark::Inserted, "New.")]));
        assert_eq!(words("Same.", "Same."), w(&[(Mark::Same, "Same.")]));
        assert_eq!(words("", ""), w(&[]));
        // A rewritten stretch reads as one deletion and one insertion, not word by word.
        assert_eq!(
            words("Costs were held steady.", "Costs fell by two percent."),
            w(&[(Mark::Same, "Costs "), (Mark::Deleted, "were held steady"), (Mark::Inserted, "fell by two percent"), (Mark::Same, ".")])
        );
    }

    proptest! {
        #[test]
        fn words_rebuild_both_texts(a in pieces(), b in pieces()) {
            let (a, b) = (join(&a), join(&b));
            let w = words(&a, &b);
            let side = |keep: Mark| w.iter().filter(|(m, _)| *m == Mark::Same || *m == keep).map(|(_, t)| t.as_str()).collect::<String>();
            prop_assert_eq!(side(Mark::Deleted), a);
            prop_assert_eq!(side(Mark::Inserted), b);
            prop_assert!(w.windows(2).all(|p| p[0].0 != p[1].0), "runs of one kind are joined: {:?}", w);
            prop_assert!(w.iter().all(|(_, t)| !t.is_empty()));
        }

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
    fn report_hunks_read_as_whole_sentences_and_stay_exact() {
        let (a, b) = ("One two. Three four. Five.", "One 2. Three four. Five six.");
        let s = sentences(a, b, &diff_text(a, b));
        let pairs: Vec<(&str, &str)> = s.iter().map(|h| (&a[h.old.clone()], &b[h.new.clone()])).collect();
        assert_eq!(pairs, [("One two. ", "One 2. "), ("Five.", "Five six.")]);
        assert_eq!(apply(a, b, &s).as_deref(), Some(b));
        // Several hunks in one sentence, the last a deletion: one group, the whole sentence.
        let (a, b) = ("The board approved the new plan. Rest.", "The board backed the plan. Rest.");
        let s = by_sentence(a, &diff_text(a, b)).unwrap();
        let pairs: Vec<(&str, &str)> = s.iter().map(|h| (&a[h.old.clone()], &b[h.new.clone()])).collect();
        assert_eq!(pairs, [("The board approved the new plan. ", "The board backed the plan. ")]);
    }

    #[test]
    fn markers_show_as_labels() {
        assert_eq!(shown("a\u{E010}b\u{E00A}c\u{E00E}"), "a[note]b[footnote]c[object]");
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
