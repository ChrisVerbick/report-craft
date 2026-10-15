//! Three-way merge of a linked story's text. The file's new text (`theirs`) wins content and
//! order. The layout's own edits (`mine`, both measured against the text the last import brought
//! in, `base`) survive where the file left that stretch alone, and every one is reported. Where
//! both changed the same stretch, the file's text wins and a conflict mark records the layout's
//! version for a Flow note.

use std::ops::Range;

use designcraft_doc::{ANCHOR_MARK, ENDNOTE_REF, FOOTNOTE_REF, INDEX_MARK, NOTE_MARK, OBJECT_MARK, TABLE_ANCHOR, XREF_MARK};

use crate::{Hunk, diff_text};

/// A character that anchors data in a story (a footnote, an anchored object, an editorial note, a
/// table…): dropping it would lose that data, so a merge never drops the ones the layout added.
pub fn is_anchor(c: char) -> bool {
    matches!(c, FOOTNOTE_REF | ENDNOTE_REF | ANCHOR_MARK | XREF_MARK | INDEX_MARK | OBJECT_MARK | NOTE_MARK | TABLE_ANCHOR)
}

/// Where a stretch of the merged text comes from: the layout's text (`mine`), the file's
/// (`theirs`), both (unchanged on both sides), or neither (a conflict mark).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub out: Range<usize>,
    pub mine: Option<Range<usize>>,
    pub theirs: Option<Range<usize>>,
}

/// A stretch both sides changed: the merged text has a [`NOTE_MARK`] at `at`, then the file's
/// version (`theirs`); the layout's version (`mine`) is for the Flow note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub at: usize,
    pub mine: Range<usize>,
    pub theirs: Range<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeOut {
    pub text: String,
    /// In order, covering `text`.
    pub segments: Vec<Segment>,
    /// Every difference between the file's text and `text` (file → merged): the layout's edits
    /// that survived, the anchors it added that were kept, the conflict marks. Applying them to
    /// the file's text gives `text` exactly.
    pub deviations: Vec<Hunk>,
    pub conflicts: Vec<Conflict>,
}

/// Merges the layout's text (`mine`) and the file's new text (`theirs`), both descended from the
/// text the last import brought in (`base`). `None` only if the merge's own bookkeeping
/// disagrees, so a caller never applies a merge it can't account for.
pub fn merge_text(base: &str, mine: &str, theirs: &str) -> Option<MergeOut> {
    let (t, m) = (diff_text(base, theirs), diff_text(base, mine));
    let mut out = MergeOut::default();
    // The next hunk on each side; merged so far: up to `done` in base, `tp` in theirs, `mp` in mine.
    let (mut ti, mut mi) = (0, 0);
    let (mut done, mut tp, mut mp) = (0_usize, 0_usize, 0_usize);
    loop {
        // A chunk starts at the hunk that starts first (an insertion before a change at the same
        // spot) and takes in every hunk of either side that overlaps it.
        let take_t = match (t.get(ti), m.get(mi)) {
            (None, None) => break,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(a), Some(b)) => (a.old.start, !a.old.is_empty()) <= (b.old.start, !b.old.is_empty()),
        };
        let (t0, m0) = (ti, mi);
        let mut chunk = if take_t { t.get(ti)?.old.clone() } else { m.get(mi)?.old.clone() };
        if take_t {
            ti += 1;
        } else {
            mi += 1;
        }
        loop {
            if let Some(h) = t.get(ti).filter(|h| overlaps(&chunk, &h.old)) {
                chunk.end = chunk.end.max(h.old.end);
                ti += 1;
            } else if let Some(h) = m.get(mi).filter(|h| overlaps(&chunk, &h.old)) {
                chunk.end = chunk.end.max(h.old.end);
                mi += 1;
            } else {
                break;
            }
        }

        // The stretch before the chunk is unchanged on both sides.
        let n = chunk.start.checked_sub(done)?;
        let (tr, mr) = (tp..tp.checked_add(n)?, mp..mp.checked_add(n)?);
        let same = theirs.get(tr.clone())?;
        if mine.get(mr.clone())? != same {
            return None;
        }
        out.push(same, Some(mr.clone()), Some(tr.clone()));

        // The chunk on each side runs from the side's position to past its last hunk in the chunk.
        let end = |hunks: &[Hunk], first: usize, next: usize, at: usize| match next.checked_sub(1).filter(|last| *last >= first) {
            Some(last) => hunks.get(last).and_then(|h| h.new.end.checked_add(chunk.end.checked_sub(h.old.end)?)),
            None => at.checked_add(chunk.len()),
        };
        let tr = tr.end..end(&t, t0, ti, tr.end)?;
        let mr = mr.end..end(&m, m0, mi, mr.end)?;
        let (ts, ms) = (theirs.get(tr.clone())?, mine.get(mr.clone())?);
        match (ti > t0, mi > m0) {
            (true, false) => {
                out.push(ts, None, Some(tr.clone()));
            }
            (false, true) => {
                let new = out.push(ms, Some(mr.clone()), None);
                out.deviations.push(Hunk { old: tr.clone(), new });
            }
            _ if ts == ms => {
                out.push(ts, Some(mr.clone()), Some(tr.clone()));
            }
            _ => {
                // Both changed it: the file's text, after a conflict mark and the anchors the
                // layout added there (dropping those would lose a footnote, a note, a frame…).
                let at = out.text.len();
                out.push(NOTE_MARK.encode_utf8(&mut [0; 4]), None, None);
                for h in m.get(m0..mi)? {
                    for (i, c) in mine.get(h.new.clone())?.char_indices().filter(|(_, c)| is_anchor(*c)) {
                        let r = h.new.start.checked_add(i)?..h.new.start.checked_add(i + c.len_utf8())?;
                        out.push(mine.get(r.clone())?, Some(r), None);
                    }
                }
                out.deviations.push(Hunk { old: tr.start..tr.start, new: at..out.text.len() });
                out.conflicts.push(Conflict { at, mine: mr.clone(), theirs: tr.clone() });
                out.push(ts, None, Some(tr.clone()));
            }
        }
        (done, tp, mp) = (chunk.end, tr.end, mr.end);
    }

    // The rest is unchanged on both sides.
    let same = theirs.get(tp..)?;
    if same.len() != base.len().checked_sub(done)? || mine.get(mp..)? != same {
        return None;
    }
    out.push(same, Some(mp..mine.len()), Some(tp..theirs.len()));
    Some(out)
}

/// Whether hunk range `h` overlaps chunk `c` (both in base): changed stretches that share text,
/// two insertions at one spot, or an insertion inside a changed stretch. Stretches that only
/// touch are merged one after the other.
fn overlaps(c: &Range<usize>, h: &Range<usize>) -> bool {
    if c.is_empty() && h.is_empty() { c.start == h.start } else { h.start < c.end && c.start < h.end }
}

impl MergeOut {
    /// Appends `s`, recording where it came from; returns where it landed.
    fn push(&mut self, s: &str, mine: Option<Range<usize>>, theirs: Option<Range<usize>>) -> Range<usize> {
        let out = self.text.len()..self.text.len() + s.len();
        self.text.push_str(s);
        if !out.is_empty() {
            self.segments.push(Segment { out: out.clone(), mine, theirs });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use designcraft_doc::{FOOTNOTE_REF, NOTE_MARK};
    use proptest::prelude::*;

    use super::*;
    use crate::apply;

    /// What every merge must satisfy, whatever its inputs: every difference from the file is
    /// reported (the 0% rule); each stretch of the result comes from where its segment says;
    /// anchors the layout added survive, in order, with one conflict mark per conflict; an untouched
    /// layout gives the file, an untouched file gives the layout, the same edit on both sides is
    /// no conflict.
    fn check(base: &str, mine: &str, theirs: &str, m: &MergeOut) -> Result<(), String> {
        let out = m.text.as_str();
        if apply(theirs, out, &m.deviations).as_deref() != Some(out) {
            return Err(format!("a difference from the file isn't reported: {m:?}"));
        }
        let mut at = 0;
        for s in &m.segments {
            let piece = out.get(s.out.clone()).filter(|_| s.out.start == at).ok_or(format!("{s:?} doesn't follow on at {at}"))?;
            if s.mine.as_ref().is_some_and(|r| mine.get(r.clone()) != Some(piece))
                || s.theirs.as_ref().is_some_and(|r| theirs.get(r.clone()) != Some(piece))
            {
                return Err(format!("{s:?} isn't the text it says it comes from"));
            }
            at = s.out.end;
        }
        if at != out.len() {
            return Err(format!("segments end at {at} of {}", out.len()));
        }
        let anchors = |s: &str| s.chars().filter(|c| is_anchor(*c) && *c != NOTE_MARK).collect::<String>();
        if anchors(base).is_empty() && anchors(theirs).is_empty() && anchors(out) != anchors(mine) {
            return Err(format!("anchors the layout added: {:?}, kept: {:?}", anchors(mine), anchors(out)));
        }
        if !mine.contains(NOTE_MARK) && out.matches(NOTE_MARK).count() != m.conflicts.len() {
            return Err(format!("{} conflict marks for {} conflicts", out.matches(NOTE_MARK).count(), m.conflicts.len()));
        }
        if (mine == base || mine == theirs) && out != theirs {
            return Err(format!("an untouched or identical layout must give the file: {out:?}"));
        }
        if theirs == base && out != mine {
            return Err(format!("an untouched file must give the layout: {out:?}"));
        }
        Ok(())
    }

    fn merged(base: &str, mine: &str, theirs: &str) -> MergeOut {
        let m = merge_text(base, mine, theirs).expect("a merge");
        if let Err(e) = check(base, mine, theirs, &m) {
            panic!("{e}");
        }
        m
    }

    fn note() -> String {
        NOTE_MARK.to_string()
    }

    #[test]
    fn edits_that_dont_overlap_all_land_and_the_layouts_are_reported() {
        let m = merged("one two three", "one two 3", "ONE two three");
        assert_eq!(m.text, "ONE two 3");
        assert!(m.conflicts.is_empty());
        assert_eq!(m.deviations.len(), 1);
    }

    #[test]
    fn the_same_change_on_both_sides_is_no_conflict() {
        let m = merged("teh cat", "the cat", "the cat");
        assert_eq!((m.text.as_str(), m.deviations.len(), m.conflicts.len()), ("the cat", 0, 0));
    }

    #[test]
    fn inserts_at_the_same_spot_conflict_and_the_file_wins() {
        let m = merged("ab", "aXb", "aYb");
        assert_eq!(m.text, format!("a{}Yb", note()));
        assert_eq!(m.conflicts, vec![Conflict { at: 1, mine: 1..2, theirs: 1..2 }]);
    }

    #[test]
    fn an_edit_inside_text_the_file_deleted_conflicts() {
        let m = merged("abcdef", "abcXdef", "af");
        assert_eq!(m.text, format!("a{}f", note()));
        assert_eq!(m.conflicts.len(), 1);
    }

    #[test]
    fn both_replacing_one_span_conflicts() {
        let m = merged("the cat sat", "the cow sat", "the dog sat");
        assert_eq!(m.text, format!("the {}dog sat", note()));
        assert_eq!(m.conflicts.len(), 1);
    }

    #[test]
    fn a_footnote_added_inside_text_the_file_deleted_is_kept() {
        let m = merged("abcdef", &format!("abc{FOOTNOTE_REF}def"), "af");
        assert_eq!(m.text, format!("a{}{FOOTNOTE_REF}f", note()));
    }

    /// Pieces for the base and the file; the layout may also add anchors (a footnote reference,
    /// an anchored object).
    const TEXT: &[&str] = &["a", "b", " ", ".", "\n", "é", "中", "👍🏽", "🇫🇷"];
    const LAYOUT: &[&str] = &["a", "b", " ", ".", "\n", "é", "中", "👍🏽", "🇫🇷", "\u{E00A}", "\u{E00E}"];

    fn edited(base: &[&'static str], edits: &[(usize, usize, u8)], pieces: &[&'static str]) -> String {
        let mut v = base.to_vec();
        for &(at, p, kind) in edits {
            let at = at % (v.len() + 1);
            let piece = pieces[p % pieces.len()];
            match kind {
                1 if at < v.len() => {
                    v.remove(at);
                }
                2 if at < v.len() => v[at] = piece,
                _ => v.insert(at, piece),
            }
        }
        v.concat()
    }

    fn edit_list() -> impl Strategy<Value = Vec<(usize, usize, u8)>> {
        proptest::collection::vec((any::<usize>(), any::<usize>(), 0..3u8), 0..5)
    }

    proptest! {
        #[test]
        fn every_merge_is_exact(base in proptest::collection::vec(0..TEXT.len(), 0..30), theirs in edit_list(), mine in edit_list()) {
            let base: Vec<&'static str> = base.iter().map(|i| TEXT[*i]).collect();
            let (theirs, mine) = (edited(&base, &theirs, TEXT), edited(&base, &mine, LAYOUT));
            let base = base.concat();
            let m = merge_text(&base, &mine, &theirs).ok_or_else(|| TestCaseError::fail("no merge"))?;
            check(&base, &mine, &theirs, &m).map_err(TestCaseError::fail)?;
        }
    }
}
