//! Compare: what changed from one document to another (Stage 4). Stories are matched by id and
//! their texts diffed into whole sentences, as an update's report groups its changes. Tables that
//! kept their rows, columns and merged cells are compared cell by cell, footnotes by id; a story,
//! table or footnote in only one document is all inserted or all deleted.
//!
//! Not compared yet: formatting, frames and graphics, endnotes, editorial notes, and tables inside
//! table cells.

use std::collections::{BTreeMap, BTreeSet};

use designcraft_doc::{Document, FOOTNOTE_REF, Story, StoryId, TABLE_ANCHOR, Table};

use crate::story::shape;
use crate::{Hunk, Mark, apply, diff_text, sentences, words};

/// Where a text is: a story's own, a cell of one of its tables (row and column from 0), or one of
/// its footnotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Text,
    Cell { table: u64, row: usize, col: usize },
    Footnote { id: u64 },
}

/// One text's changes in whole sentences, with both texts: applying `hunks` to `a` gives `b`
/// exactly (checked as the change set is made). `runs` are the same changes word by word, for
/// marking them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChanges {
    pub place: Place,
    pub a: String,
    pub b: String,
    pub hunks: Vec<Hunk>,
    pub runs: Vec<Run>,
}

/// Words only `a` has (deleted) or only `b` has (inserted): where they are in `a` (`old`) and in
/// `b` (`new`); the other range is empty, where they would be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub mark: Mark,
    pub old: std::ops::Range<usize>,
    pub new: std::ops::Range<usize>,
}

/// A story in both documents, or in one only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Changed,
    /// Only in `b`: all its text is inserted.
    Inserted,
    /// Only in `a`: all its text is deleted.
    Deleted,
}

/// A table listed rather than compared cell by cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableStatus {
    /// In both, with other rows, columns or merged cells.
    Reshaped,
    Inserted,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableChange {
    pub table: u64,
    pub status: TableStatus,
}

/// How one story differs: its texts that changed (its own, its cells', its footnotes'), and its
/// tables listed whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoryChanges {
    pub story: StoryId,
    pub status: Status,
    pub texts: Vec<TextChanges>,
    pub tables: Vec<TableChange>,
}

/// What changed from document `a` to `b`: the stories that differ, by id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub stories: Vec<StoryChanges>,
}

impl ChangeSet {
    /// How many changes: runs of whole sentences, and tables listed whole.
    pub fn len(&self) -> usize {
        self.stories.iter().map(|s| s.texts.iter().map(|t| t.hunks.len()).sum::<usize>() + s.tables.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.stories.is_empty()
    }
}

/// What changed from document `a` to `b`, story by story.
pub fn compare(a: &Document, b: &Document) -> ChangeSet {
    let ids: BTreeSet<StoryId> = a.stories.keys().chain(b.stories.keys()).copied().collect();
    ChangeSet { stories: ids.into_iter().filter_map(|id| story_changes(id, a.story(id), b.story(id))).collect() }
}

fn story_changes(id: StoryId, a: Option<&Story>, b: Option<&Story>) -> Option<StoryChanges> {
    let status = match (a, b) {
        (Some(_), Some(_)) => Status::Changed,
        (None, Some(_)) => Status::Inserted,
        (Some(_), None) => Status::Deleted,
        (None, None) => return None,
    };
    let empty = Story::new(id);
    let (a, b) = (a.unwrap_or(&empty), b.unwrap_or(&empty));
    let (ka, kb) = keyed(a, b);
    let mut texts: Vec<TextChanges> = changes(Place::Text, (&a.text, &b.text), (&ka, &kb)).into_iter().collect();
    let mut tables = Vec::new();
    for t in a.tables.keys().chain(b.tables.keys()).copied().collect::<BTreeSet<u64>>() {
        let status = match (a.tables.get(&t), b.tables.get(&t)) {
            (Some(x), Some(y)) if shape(x) == shape(y) => {
                texts.extend(cells(t, x, y));
                continue;
            }
            (Some(_), Some(_)) => TableStatus::Reshaped,
            (None, Some(_)) => TableStatus::Inserted,
            _ => TableStatus::Deleted,
        };
        tables.push(TableChange { table: t, status });
    }
    let notes: BTreeSet<u64> = a.notes.iter().chain(&b.notes).map(|n| n.id).collect();
    for n in notes {
        let text = |s: &Story| s.notes.iter().find(|f| f.id == n).map(|f| f.text.text.clone()).unwrap_or_default();
        texts.extend(text_changes(Place::Footnote { id: n }, &text(a), &text(b)));
    }
    (status != Status::Changed || !texts.is_empty() || !tables.is_empty()).then_some(StoryChanges { story: id, status, texts, tables })
}

/// The two stories' texts with each footnote reference and table anchor made a character of its
/// own, the same in both for the same footnote or table: the diff then keeps a reference only where
/// its footnote stays, and shows one put in the place of another as replaced. Each stand-in is as
/// long as its marker in bytes, so the diff's offsets hold for the texts themselves. Should the
/// private-use characters run out, the texts are diffed as they are.
fn keyed(a: &Story, b: &Story) -> (String, String) {
    let used: BTreeSet<char> = a.text.chars().chain(b.text.chars()).collect();
    let mut free = ('\u{E100}'..='\u{F8FF}').filter(|c| !used.contains(c));
    let mut keys: BTreeMap<(char, u64), char> = BTreeMap::new();
    let mut key = |st: &Story| -> Option<String> {
        let mut note = 0;
        let mut out = String::with_capacity(st.text.len());
        for (at, c) in st.text.char_indices() {
            let id = match c {
                FOOTNOTE_REF => {
                    note += 1;
                    st.notes.get(note - 1).map(|n| n.id)
                }
                TABLE_ANCHOR => st.paras.get(st.para_at(at)).and_then(|p| p.table),
                _ => None,
            };
            out.push(match id {
                Some(id) => match keys.get(&(c, id)) {
                    Some(k) => *k,
                    None => *keys.entry((c, id)).or_insert(free.next()?),
                },
                None => c,
            });
        }
        Some(out)
    };
    match (key(a), key(b)) {
        (Some(x), Some(y)) => (x, y),
        _ => (a.text.clone(), b.text.clone()),
    }
}

/// The changes from text `a` to `b` in whole sentences.
fn text_changes(place: Place, a: &str, b: &str) -> Option<TextChanges> {
    changes(place, (a, b), (a, b))
}

/// [`text_changes`] of texts `a` and `b`, diffed as `keyed` (see [`keyed`]); one change from all
/// of `a` to all of `b` should the sentences ever fail to rebuild `b`.
fn changes(place: Place, (a, b): (&str, &str), (ka, kb): (&str, &str)) -> Option<TextChanges> {
    let raw = diff_text(ka, kb);
    if raw.is_empty() {
        return None;
    }
    let hunks = sentences(a, b, &raw);
    let hunks = if apply(a, b, &hunks).as_deref() == Some(b) { hunks } else { vec![Hunk { old: 0..a.len(), new: 0..b.len() }] };
    let runs = word_runs(ka, kb, &hunks);
    Some(TextChanges { place, a: a.to_string(), b: b.to_string(), hunks, runs })
}

/// The words each hunk deletes and inserts, from the texts as diffed.
fn word_runs(a: &str, b: &str, hunks: &[Hunk]) -> Vec<Run> {
    let mut out = Vec::new();
    for h in hunks {
        let (mut at_a, mut at_b) = (h.old.start, h.new.start);
        for (mark, t) in words(a.get(h.old.clone()).unwrap_or_default(), b.get(h.new.clone()).unwrap_or_default()) {
            let n = t.len();
            match mark {
                Mark::Same => {
                    at_a += n;
                    at_b += n;
                }
                Mark::Deleted => {
                    out.push(Run { mark, old: at_a..at_a + n, new: at_b..at_b });
                    at_a += n;
                }
                Mark::Inserted => {
                    out.push(Run { mark, old: at_a..at_a, new: at_b..at_b + n });
                    at_b += n;
                }
            }
        }
    }
    out
}

/// The cells of two tables of the same shape that differ.
fn cells(table: u64, a: &Table, b: &Table) -> Vec<TextChanges> {
    let cols = a.ncols().max(1);
    a.cells
        .iter()
        .zip(&b.cells)
        .enumerate()
        .filter_map(|(i, (x, y))| text_changes(Place::Cell { table, row: i / cols, col: i % cols }, &x.text.text, &y.text.text))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use designcraft_doc::build::NewDocument;
    use designcraft_doc::{Document, Footnote, ParaFormat, Story, StoryId, Table};
    use proptest::prelude::*;

    use super::*;
    use crate::{Mark, apply};

    /// A document holding `stories` (id, text).
    fn doc(stories: &[(u64, &str)]) -> Document {
        let mut d = Document::new(&NewDocument::default());
        for (id, text) in stories {
            d.stories.insert(StoryId(*id), Arc::new(Story::with_text(StoryId(*id), text, ParaFormat::default())));
        }
        d
    }

    /// Each change as the texts it replaces and puts in.
    fn pairs(t: &TextChanges) -> Vec<(&str, &str)> {
        t.hunks.iter().map(|h| (&t.a[h.old.clone()], &t.b[h.new.clone()])).collect()
    }

    #[test]
    fn changed_sentences_are_listed_whole() {
        let (a, b) = (doc(&[(1, "One. Two. Three.")]), doc(&[(1, "One. Two more. Three.")]));
        let c = compare(&a, &b);
        assert_eq!(c.stories.len(), 1);
        let s = &c.stories[0];
        assert_eq!((s.story, s.status), (StoryId(1), Status::Changed));
        assert_eq!(s.texts.len(), 1);
        assert_eq!(s.texts[0].place, Place::Text);
        assert_eq!(pairs(&s.texts[0]), [("Two. ", "Two more. ")]);
        assert_eq!(c.len(), 1);
        assert!(compare(&a, &a).is_empty(), "nothing changed");
    }

    #[test]
    fn stories_in_one_document_only_are_all_inserted_or_all_deleted() {
        let c = compare(&doc(&[(1, "Same."), (2, "Gone.")]), &doc(&[(1, "Same."), (3, "New.")]));
        let found: Vec<(StoryId, Status, Vec<(&str, &str)>)> = c.stories.iter().map(|s| (s.story, s.status, pairs(&s.texts[0]))).collect();
        assert_eq!(found, [(StoryId(2), Status::Deleted, vec![("Gone.", "")]), (StoryId(3), Status::Inserted, vec![("", "New.")])]);
    }

    /// A 2 × 2 table of `cells`, row by row.
    fn table(id: u64, cells: [&str; 4]) -> Arc<Table> {
        let rows: Vec<Vec<String>> = cells.chunks(2).map(|r| r.iter().map(|c| c.to_string()).collect()).collect();
        Arc::new(Table::from_strings(id, &rows, 144.0, &ParaFormat::default(), &Default::default()))
    }

    #[test]
    fn tables_compare_cell_by_cell_while_they_keep_their_shape() {
        let mut a = doc(&[(1, "Prices.")]);
        let mut b = doc(&[(1, "Prices.")]);
        let with = |d: &mut Document, tables: Vec<(u64, Arc<Table>)>| {
            Arc::make_mut(d.stories.get_mut(&StoryId(1)).unwrap()).tables = tables.into_iter().collect()
        };
        with(&mut a, vec![(7, table(7, ["Tea", "3.50", "Cake", "3.10"])), (8, table(8, ["A", "B", "C", "D"])), (9, table(9, ["x", "y", "z", "w"]))]);
        let mut wider = (*table(8, ["A", "B", "C", "D"])).clone();
        wider.insert_cols(2, 1, 72.0);
        with(&mut b, vec![(7, table(7, ["Tea", "3.75", "Cake", "3.10"])), (8, Arc::new(wider)), (10, table(10, ["n", "e", "w", "!"]))]);
        let c = compare(&a, &b);
        let s = &c.stories[0];
        assert_eq!(s.texts.len(), 1);
        assert_eq!(s.texts[0].place, Place::Cell { table: 7, row: 0, col: 1 });
        assert_eq!(pairs(&s.texts[0]), [("3.50", "3.75")]);
        let tables: Vec<(u64, TableStatus)> = s.tables.iter().map(|t| (t.table, t.status)).collect();
        assert_eq!(tables, [(8, TableStatus::Reshaped), (9, TableStatus::Deleted), (10, TableStatus::Inserted)]);
        assert_eq!(c.len(), 4);
    }

    #[test]
    fn footnotes_compare_by_id() {
        let note = |id: u64, text: &str| Arc::new(Footnote { id, text: Story::with_text(StoryId(0), text, ParaFormat::default()) });
        let mut a = doc(&[(1, "Body.")]);
        let mut b = doc(&[(1, "Body.")]);
        Arc::make_mut(a.stories.get_mut(&StoryId(1)).unwrap()).notes = vec![note(1, "Source: 2025."), note(2, "Old note.")];
        Arc::make_mut(b.stories.get_mut(&StoryId(1)).unwrap()).notes = vec![note(1, "Source: 2026."), note(3, "New note.")];
        let s = &compare(&a, &b).stories[0];
        let found: Vec<(Place, Vec<(&str, &str)>)> = s.texts.iter().map(|t| (t.place, pairs(t))).collect();
        assert_eq!(
            found,
            [
                (Place::Footnote { id: 1 }, vec![("Source: 2025.", "Source: 2026.")]),
                (Place::Footnote { id: 2 }, vec![("Old note.", "")]),
                (Place::Footnote { id: 3 }, vec![("", "New note.")]),
            ]
        );
    }

    fn story_mut(d: &mut Document) -> &mut Story {
        Arc::make_mut(d.stories.get_mut(&StoryId(1)).unwrap())
    }

    /// A footnote or table put in the place of another (a new one, under a new id) is the old one
    /// deleted and the new one inserted: the story's text change shows its reference or anchor
    /// replaced, so a redline can give either version back. (A new footnote can get the id of one
    /// deleted before it; then it is that footnote, its text changed.)
    #[test]
    fn a_footnote_or_table_replaced_in_place_is_one_deleted_and_one_inserted() {
        let mut a = doc(&[(1, "Revenue grew. Prices:")]);
        let st = story_mut(&mut a);
        let old = st.insert_note(13, "Old note.", ParaFormat::default()).unwrap();
        let end = st.len();
        st.insert_note(end, "Kept.", ParaFormat::default()).unwrap();
        let end = st.len();
        st.insert_table(end, (*table(7, ["Tea", "3.50", "Cake", "3.10"])).clone());
        let mut b = a.clone();
        let st = story_mut(&mut b);
        let at = st.note_anchor(old).unwrap();
        st.delete(at..at + designcraft_doc::FOOTNOTE_REF.len_utf8());
        let new = st.insert_note(at, "New note.", ParaFormat::default()).unwrap();
        assert_ne!(new, old);
        let t = st.text.find(designcraft_doc::TABLE_ANCHOR).unwrap();
        st.delete(t..t + designcraft_doc::TABLE_ANCHOR.len_utf8());
        st.insert_table(t, (*table(11, ["Tea", "3.50", "Cake", "3.10"])).clone());
        let s = &compare(&a, &b).stories[0];
        let notes: Vec<(Place, Vec<(&str, &str)>)> =
            s.texts.iter().filter(|t| matches!(t.place, Place::Footnote { .. })).map(|t| (t.place, pairs(t))).collect();
        assert_eq!(notes, [(Place::Footnote { id: old }, vec![("Old note.", "")]), (Place::Footnote { id: new }, vec![("", "New note.")])]);
        let tables: Vec<(u64, TableStatus)> = s.tables.iter().map(|t| (t.table, t.status)).collect();
        assert_eq!(tables, [(7, TableStatus::Deleted), (11, TableStatus::Inserted)]);
        let text = s.texts.iter().find(|t| t.place == Place::Text).expect("the reference and the anchor changed");
        let changed: String = text.hunks.iter().map(|h| &text.a[h.old.clone()]).collect();
        assert!(changed.contains(designcraft_doc::FOOTNOTE_REF) && changed.contains(designcraft_doc::TABLE_ANCHOR), "{:?}", pairs(text));
        assert_eq!(apply(&text.a, &text.b, &text.hunks).as_deref(), Some(text.b.as_str()));
    }

    /// A text's changes word by word too, for marking them in a redline: the words only `a` has,
    /// and only `b` has, with where they are in each; a footnote put in another's place shows as
    /// its reference deleted and a reference inserted.
    #[test]
    fn changes_come_word_by_word_too() {
        let c = compare(&doc(&[(1, "Revenue grew. Costs were flat. Staff held.")]), &doc(&[(1, "Revenue grew. Costs fell. Staff held.")]));
        let t = &c.stories[0].texts[0];
        let runs: Vec<(Mark, &str, &str)> = t.runs.iter().map(|r| (r.mark, &t.a[r.old.clone()], &t.b[r.new.clone()])).collect();
        assert_eq!(runs, [(Mark::Deleted, "were flat", ""), (Mark::Inserted, "", "fell")]);
        let mut a = doc(&[(1, "Revenue grew.")]);
        let st = story_mut(&mut a);
        let old = st.insert_note(13, "Old.", ParaFormat::default()).unwrap();
        st.insert_note(0, "Kept.", ParaFormat::default()).unwrap();
        let mut b = a.clone();
        let st = story_mut(&mut b);
        let at = st.note_anchor(old).unwrap();
        st.delete(at..at + designcraft_doc::FOOTNOTE_REF.len_utf8());
        st.insert_note(at, "New.", ParaFormat::default()).unwrap();
        let s = &compare(&a, &b).stories[0];
        let t = s.texts.iter().find(|t| t.place == Place::Text).unwrap();
        let runs: Vec<(Mark, &str, &str)> = t.runs.iter().map(|r| (r.mark, &t.a[r.old.clone()], &t.b[r.new.clone()])).collect();
        let marker = designcraft_doc::FOOTNOTE_REF.to_string();
        assert_eq!(runs, [(Mark::Deleted, marker.as_str(), ""), (Mark::Inserted, "", marker.as_str())]);
    }

    /// Pieces that stress the diff: accents (composed and combining), emoji, CJK, paragraph and
    /// sentence breaks, and a marker character.
    const PIECES: &[&str] = &["The ", "fox", ". ", "\n", "é", "e\u{301}", "👍🏽", "👩‍👩‍👧", "漢字", "?", "\u{E00A}", "", "Report"];

    fn text() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(PIECES), 0..30).prop_map(|p| p.concat())
    }

    proptest! {
        /// Applying a change set to the old text gives the new text exactly.
        #[test]
        fn a_change_set_rebuilds_the_new_text(a in text(), b in text()) {
            let c = compare(&doc(&[(1, &a)]), &doc(&[(1, &b)]));
            prop_assert_eq!(c.is_empty(), a == b);
            for t in c.stories.iter().flat_map(|s| &s.texts) {
                let rebuilt = apply(&t.a, &t.b, &t.hunks);
                prop_assert_eq!(rebuilt.as_deref(), Some(t.b.as_str()));
            }
        }
    }
}
