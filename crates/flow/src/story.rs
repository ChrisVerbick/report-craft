//! Three-way merge of a linked story: the merged text (see [`crate::merge_text`]) with its
//! character formats, paragraph formats, tables and anchored items carried across.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use designcraft_doc::{
    ANCHOR_MARK, CharFormat, CharRun, ENDNOTE_REF, EditorialNote, FOOTNOTE_REF, INDEX_MARK, NOTE_MARK, OBJECT_MARK, ParaFormat, Story, Table,
    XREF_MARK,
};

use crate::{Hunk, MergeOut, diff_text, merge_text};

/// A merged story and what its update report needs.
#[derive(Clone, Debug)]
pub struct StoryMerge {
    /// The layout's story (`mine`: its id, frames and link) with the merged content.
    pub story: Story,
    /// The text merge: deviations from the file and conflicts.
    pub text: MergeOut,
    /// More for the report: tables the file replaced although the layout had edited them, and
    /// the layout's tables kept over the file's.
    pub warnings: Vec<String>,
    /// The cells of tables merged cell by cell (tables that kept their shape).
    pub cells: Vec<CellMerge>,
}

/// One cell of a table merged cell by cell: where it is (the merged story's table id, row and
/// column), its text merge, and the cell's text in the file and in the layout before the update.
#[derive(Clone, Debug)]
pub struct CellMerge {
    pub table: u64,
    pub row: usize,
    pub col: usize,
    pub text: MergeOut,
    pub file: String,
    pub layout: String,
}

/// Merges the layout's story (`mine`) with the file's new import (`theirs`), both descended from
/// the story the last import left (`base`). `None` when the result would not be a valid story or
/// the merge can't account for it.
pub fn merge_story(base: &Story, mine: &Story, theirs: &Story) -> Option<StoryMerge> {
    merge_story_at(base, mine, theirs, 0)
}

/// Nesting limit for tables inside table cells.
const MAX_DEPTH: usize = 4;

fn merge_story_at(base: &Story, mine: &Story, theirs: &Story, depth: usize) -> Option<StoryMerge> {
    let text = merge_text(&base.text, &mine.text, &theirs.text)?;
    let chars = char_runs(&text, base, mine, theirs)?;

    // Paragraph formats. A table all three have that kept its shape is merged cell by cell in the
    // layout's table; any other table comes with the format that names it (the file's under a new id).
    let mut warnings = Vec::new();
    let mut cells = Vec::new();
    let mut tables = BTreeMap::new();
    let mut next_table = mine.tables.keys().chain(theirs.tables.keys()).max().map_or(1, |k| k.saturating_add(1));
    let mut paras = Vec::new();
    for (i, start) in std::iter::once(0).chain(text.text.match_indices('\n').map(|(at, _)| at + 1)).enumerate() {
        let (mut p, side, three) = para_format(&text, start, base, mine, theirs)?;
        if let Some(id) = p.table {
            let ids = three.and_then(|[b, m, t]| Some((b?, m?, t?)));
            let cellwise = ids.and_then(|(b, m, t)| {
                merge_cells(base.tables.get(&b)?, mine.tables.get(&m)?, theirs.tables.get(&t)?, m, depth).map(|merged| (m, merged))
            });
            let id = match (cellwise, side) {
                (Some((m, (table, merged))), _) => {
                    tables.insert(m, Arc::new(table));
                    cells.extend(merged);
                    m
                }
                (None, Side::Mine) => {
                    if ids.is_some() {
                        warnings.push(format!(
                            "paragraph {}: the layout's table was kept; the file's version (its rows or columns changed) was not applied",
                            i + 1
                        ));
                    }
                    tables.insert(id, mine.tables.get(&id)?.clone());
                    id
                }
                (None, Side::Theirs) => {
                    if let Some((b, m, _)) = ids
                        && mine.tables.get(&m) != base.tables.get(&b)
                    {
                        warnings.push(format!(
                            "paragraph {}: the file's table (its rows or columns changed) replaced edits made to it in the layout",
                            i + 1
                        ));
                    }
                    let new = next_table;
                    next_table = next_table.saturating_add(1);
                    tables.insert(new, Arc::new(Table { id: new, ..(**theirs.tables.get(&id)?).clone() }));
                    new
                }
            };
            p.table = Some(id);
        }
        paras.push(p);
    }

    // Anchored items follow their marks; a conflict mark gets a Flow note with the layout's version.
    let mut next_note = mine.editorial.iter().map(|n| n.id).max().map_or(1, |id| id.saturating_add(1));
    let mut conflicts = text.conflicts.iter();
    let editorial = follow(&text, NOTE_MARK, (&mine.text, &mine.editorial), (&theirs.text, &theirs.editorial))?
        .into_iter()
        .map(|n| match n {
            Some(n) => Some(n),
            None => {
                let c = conflicts.next()?;
                let layout: String = mine.text.get(c.mine.clone())?.chars().filter(|ch| !('\u{E000}'..='\u{E012}').contains(ch)).collect();
                let id = next_note;
                next_note = next_note.saturating_add(1);
                Some(Arc::new(EditorialNote { id, author: "Flow".into(), text: format!("Layout had: {}", layout.trim()) }))
            }
        })
        .collect::<Option<Vec<_>>>()?;
    let mut story = Story {
        text: text.text.clone(),
        paras,
        chars,
        tables,
        notes: all(follow(&text, FOOTNOTE_REF, (&mine.text, &mine.notes), (&theirs.text, &theirs.notes))?)?,
        endnotes: all(follow(&text, ENDNOTE_REF, (&mine.text, &mine.endnotes), (&theirs.text, &theirs.endnotes))?)?,
        anchors: all(follow(&text, ANCHOR_MARK, (&mine.text, &mine.anchors), (&theirs.text, &theirs.anchors))?)?,
        xrefs: all(follow(&text, XREF_MARK, (&mine.text, &mine.xrefs), (&theirs.text, &theirs.xrefs))?)?,
        index_refs: all(follow(&text, INDEX_MARK, (&mine.text, &mine.index_refs), (&theirs.text, &theirs.index_refs))?)?,
        objects: all(follow(&text, OBJECT_MARK, (&mine.text, &mine.objects), (&theirs.text, &theirs.objects))?)?,
        editorial,
        rev: mine.rev.wrapping_add(1),
        ..mine.clone()
    };
    keep_moved_formats(&mut story, &text, base, mine)?;
    story.check().ok()?;
    Some(StoryMerge { story, text, warnings, cells })
}

/// The 0% check for a merge: its text differs from the file's only by the reported deviations, and
/// the changes from the layout's text rebuild it exactly. (Table cells merged cell by cell were
/// checked as they were merged.)
pub fn verified(theirs: &Story, mine: &Story, m: &StoryMerge) -> bool {
    let out = m.story.text.as_str();
    crate::apply(&theirs.text, out, &m.text.deviations).as_deref() == Some(out)
        && crate::apply(&mine.text, out, &crate::diff_text(&mine.text, out)).as_deref() == Some(out)
}

/// The layout's table with each cell's text merged with the file's by Flow's rules, keeping the
/// layout's table formatting (style, widths, heights, fills, strokes); a graphic cell keeps the
/// layout's. `None` when the three tables don't have the same rows, columns and merged cells, or
/// a cell's merge can't be accounted for.
fn merge_cells(base: &Table, mine: &Table, theirs: &Table, id: u64, depth: usize) -> Option<(Table, Vec<CellMerge>)> {
    let shape = |t: &Table| (t.nrows(), t.ncols(), t.cells.iter().map(|c| (c.row_span, c.col_span)).collect::<Vec<_>>());
    if depth >= MAX_DEPTH || shape(base) != shape(mine) || shape(mine) != shape(theirs) {
        return None;
    }
    let cols = mine.ncols().max(1);
    let mut table = mine.clone();
    let mut merged = Vec::new();
    for (i, cell) in table.cells.iter_mut().enumerate() {
        let (b, t) = (base.cells.get(i)?, theirs.cells.get(i)?);
        if cell.graphic.is_some() || b.graphic.is_some() || t.graphic.is_some() {
            continue;
        }
        let m = merge_story_at(&b.text, &cell.text, &t.text, depth + 1)?;
        if !verified(&t.text, &cell.text, &m) {
            return None;
        }
        merged.push(CellMerge {
            table: id,
            row: i / cols,
            col: i % cols,
            text: m.text.clone(),
            file: t.text.text.clone(),
            layout: cell.text.text.clone(),
        });
        merged.extend(m.cells);
        cell.text = m.story;
    }
    Some((table, merged))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Mine,
    Theirs,
}

/// Character formats over the merged text, stretch by stretch from where each came from. Where all
/// three have the text unchanged, the file's format wins unless the layout changed it.
fn char_runs(m: &MergeOut, base: &Story, mine: &Story, theirs: &Story) -> Option<Vec<CharRun>> {
    let (fb, fm, ft) = (Formats::new(base), Formats::new(mine), Formats::new(theirs));
    let mut out: Vec<CharRun> = Vec::new();
    for s in &m.segments {
        let mut cuts = vec![0, s.out.len()];
        for (f, r) in [(&fb, &s.base), (&fm, &s.mine), (&ft, &s.theirs)] {
            if let Some(r) = r {
                cuts.extend(f.ends_inside(r).iter().map(|e| e - r.start));
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let &[a, b] = w else { continue };
            let format = match (format_in(&fb, &s.base, a)?, format_in(&fm, &s.mine, a)?, format_in(&ft, &s.theirs, a)?) {
                (Some(x), Some(y), Some(z)) => {
                    if y == x {
                        z.clone()
                    } else {
                        y.clone()
                    }
                }
                (_, Some(y), _) => y.clone(),
                (_, None, Some(z)) => z.clone(),
                (_, None, None) => out.last().map(|r| r.format.clone()).unwrap_or_default(),
            };
            match out.last_mut() {
                Some(r) if r.format == format => r.len += b - a,
                _ => out.push(CharRun { len: b - a, format }),
            }
        }
    }
    if out.is_empty() {
        out.push(CharRun { len: 0, format: mine.chars.first().map(|r| r.format.clone()).unwrap_or_default() });
    }
    Some(out)
}

/// The format at offset `a` into range `r` of `f`; `Some(None)` without a range.
fn format_in<'s>(f: &Formats<'s>, r: &Option<Range<usize>>, a: usize) -> Option<Option<&'s CharFormat>> {
    match r {
        Some(r) => f.at(r.start + a).map(Some),
        None => Some(None),
    }
}

/// A story's run ends and formats, to find the format at a position quickly.
struct Formats<'s> {
    ends: Vec<usize>,
    formats: Vec<&'s CharFormat>,
}

impl<'s> Formats<'s> {
    fn new(st: &'s Story) -> Self {
        let mut end = 0;
        let (ends, formats) = st
            .chars
            .iter()
            .map(|r| {
                end += r.len;
                (end, &r.format)
            })
            .unzip();
        Formats { ends, formats }
    }

    /// The format of the character starting at `pos`.
    fn at(&self, pos: usize) -> Option<&'s CharFormat> {
        self.formats.get(self.ends.partition_point(|e| *e <= pos)).copied()
    }

    /// The run ends strictly inside `r`.
    fn ends_inside(&self, r: &Range<usize>) -> &[usize] {
        let (i, j) = (self.ends.partition_point(|e| *e <= r.start), self.ends.partition_point(|e| *e < r.end));
        self.ends.get(i..j).unwrap_or_default()
    }
}

/// The format of the merged paragraph starting at `pos` and the side it comes from: the source of
/// its first character (a conflict mark belongs to the text after it); where all three have that
/// paragraph, the file's format unless the layout changed it.
fn para_format(m: &MergeOut, pos: usize, base: &Story, mine: &Story, theirs: &Story) -> Option<(ParaFormat, Side, Option<[Option<u64>; 3]>)> {
    let from = m.segments.get(m.segments.partition_point(|s| s.out.end <= pos)..)?.iter().find(|s| s.mine.is_some() || s.theirs.is_some());
    let (pb, pm, pt) = match from {
        Some(s) => {
            let off = pos.saturating_sub(s.out.start);
            match (para_in(base, &s.base, off), para_in(mine, &s.mine, off), para_in(theirs, &s.theirs, off)) {
                (Some(b), Some(m), Some(t)) => (b?, m?, t?),
                (_, Some(m), _) => return Some((m?.clone(), Side::Mine, None)),
                (_, None, Some(t)) => return Some((t?.clone(), Side::Theirs, None)),
                _ => return None,
            }
        }
        // An empty last paragraph: the last paragraph on each side.
        None => (base.paras.last()?, mine.paras.last()?, theirs.paras.last()?),
    };
    let tables = Some([pb.table, pm.table, pt.table]);
    if pm != pb {
        return Some((pm.clone(), Side::Mine, tables));
    }
    Some((pt.clone(), Side::Theirs, tables))
}

/// The format of the paragraph at offset `off` into range `r` of `st`; `None` without a range.
fn para_in<'s>(st: &'s Story, r: &Option<Range<usize>>, off: usize) -> Option<Option<&'s ParaFormat>> {
    r.as_ref().map(|r| st.paras.get(st.para_at(r.start + off)))
}

/// The items anchored by `mark` in the merged text, in order, each from the side its mark came
/// from (the layout's when both have it). `None` for a conflict mark.
fn follow<T: Clone>(m: &MergeOut, mark: char, mine: (&str, &[T]), theirs: (&str, &[T])) -> Option<Vec<Option<T>>> {
    let positions = |text: &str| text.match_indices(mark).map(|(at, _)| at).collect::<Vec<_>>();
    let (pm, pt) = (positions(mine.0), positions(theirs.0));
    m.text
        .match_indices(mark)
        .map(|(at, _)| {
            let s = m.segments.get(m.segments.partition_point(|s| s.out.end <= at))?;
            let off = at.checked_sub(s.out.start)?;
            let item = |r: &Range<usize>, pos: &[usize], items: &[T]| items.get(pos.binary_search(&(r.start + off)).ok()?).cloned();
            match (&s.mine, &s.theirs) {
                (Some(r), _) => Some(Some(item(r, &pm, mine.1)?)),
                (None, Some(r)) => Some(Some(item(r, &pt, theirs.1)?)),
                (None, None) => Some(None),
            }
        })
        .collect()
}

/// `items` with every entry present.
fn all<T>(items: Vec<Option<T>>) -> Option<Vec<T>> {
    items.into_iter().collect()
}

/// Paragraphs the file moved keep the layout's formats: a paragraph only the file's text has
/// (wholly from one file-only stretch), equal to a base paragraph that no unchanged stretch keeps
/// and whose text the layout left alone, takes the layout's paragraph and character formats.
fn keep_moved_formats(story: &mut Story, m: &MergeOut, base: &Story, mine: &Story) -> Option<()> {
    let to_mine = diff_text(&base.text, &mine.text);
    let kept: Vec<&Range<usize>> = m.segments.iter().filter_map(|s| s.base.as_ref()).collect();
    let mut gone: Vec<Range<usize>> =
        base.para_ranges().into_iter().filter(|r| !r.is_empty() && !kept.iter().any(|k| k.start < r.end && r.start < k.end)).collect();
    for (i, r) in story.para_ranges().into_iter().enumerate() {
        let from_file = m.segments.iter().any(|s| s.mine.is_none() && s.theirs.is_some() && s.out.start <= r.start && r.end <= s.out.end);
        if r.is_empty() || !from_file {
            continue;
        }
        let text = story.text.get(r.clone())?;
        let Some(k) = gone.iter().position(|g| base.text.get(g.clone()) == Some(text)) else { continue };
        let Some(mr) = map_range(&to_mine, &gone.remove(k)) else { continue };
        let p = mine.paras.get(mine.para_at(mr.start))?;
        if p.table.is_some() {
            continue;
        }
        *story.paras.get_mut(i)? = p.clone();
        for (run, f) in mine.runs().filter(|(run, _)| run.start < mr.end && mr.start < run.end) {
            let (a, b) = (run.start.max(mr.start) - mr.start, run.end.min(mr.end) - mr.start);
            story.format_chars(r.start + a..r.start + b, |x| *x = f.clone());
        }
    }
    Some(())
}

/// Where base range `r` is in the layout's text, if the layout changed nothing in or at it.
fn map_range(hunks: &[Hunk], r: &Range<usize>) -> Option<Range<usize>> {
    if hunks.iter().any(|h| h.old.start <= r.end && r.start <= h.old.end) {
        return None;
    }
    let (old_end, new_end) = hunks.iter().take_while(|h| h.old.end <= r.start).last().map_or((0, 0), |h| (h.old.end, h.new.end));
    let start = r.start.checked_sub(old_end)?.checked_add(new_end)?;
    Some(start..start.checked_add(r.len())?)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use designcraft_doc::{EditorialNote, NOTE_MARK, ParaFormat, Story, StoryId};

    use super::*;

    /// A story of `text`, every paragraph in style Body.
    fn story(text: &str) -> Story {
        Story::with_text(StoryId(1), text, ParaFormat { style: "Body".into(), ..Default::default() })
    }

    fn bold(st: &mut Story, word: &str) {
        let at = st.text.find(word).unwrap();
        st.format_chars(at..at + word.len(), |f| f.over.font_style = Some("Bold".into()));
    }

    fn is_bold(st: &Story, word: &str) -> bool {
        let at = st.text.find(word).unwrap();
        st.runs().filter(|(r, _)| r.start < at + word.len() && at < r.end).all(|(_, f)| f.over.font_style.as_deref() == Some("Bold"))
    }

    fn merged(base: &Story, mine: &Story, theirs: &Story) -> StoryMerge {
        let m = merge_story(base, mine, theirs).expect("a merge");
        m.story.check().expect("a valid story");
        m
    }

    #[test]
    fn a_word_bolded_in_the_layout_stays_bold_when_the_file_edits_around_it() {
        let base = story("The quick fox.");
        let mut mine = base.clone();
        bold(&mut mine, "quick");
        let m = merged(&base, &mine, &story("The very quick brown fox."));
        assert_eq!(m.story.text, "The very quick brown fox.");
        assert!(is_bold(&m.story, "quick"));
        assert!(!is_bold(&m.story, "very") && !is_bold(&m.story, "brown"));
    }

    #[test]
    fn a_word_the_file_bolds_comes_through_when_the_layout_left_it_alone() {
        let base = story("The quick fox.");
        let mut theirs = base.clone();
        bold(&mut theirs, "quick");
        let m = merged(&base, &base.clone(), &theirs);
        assert!(is_bold(&m.story, "quick") && !is_bold(&m.story, "fox"));
    }

    #[test]
    fn a_paragraph_the_file_moved_keeps_its_layout_style() {
        let base = story("Alpha one\nBravo two\nCharlie three");
        let mut mine = base.clone();
        mine.paras[1].style = "Highlight".into();
        bold(&mut mine, "Bravo");
        let m = merged(&base, &mine, &story("Alpha one\nCharlie three\nBravo two"));
        assert_eq!(m.story.text, "Alpha one\nCharlie three\nBravo two");
        let styles: Vec<&str> = m.story.paras.iter().map(|p| p.style.as_str()).collect();
        assert_eq!(styles, ["Body", "Body", "Highlight"]);
        assert!(is_bold(&m.story, "Bravo"));
    }

    #[test]
    fn an_editorial_note_survives_an_update() {
        let base = story("One two three.");
        let mut mine = base.clone();
        mine.insert(7, &NOTE_MARK.to_string());
        mine.editorial = vec![Arc::new(EditorialNote { id: 7, author: "Ana".into(), text: "Check this".into() })];
        let m = merged(&base, &mine, &story("One two three, four."));
        assert_eq!(m.story.text, format!("One two{NOTE_MARK} three, four."));
        assert_eq!(m.story.editorial.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["Check this"]);
    }

    #[test]
    fn a_sentence_rewritten_in_the_layout_survives_and_is_reported() {
        let base = story("First sentence. Second sentence.");
        let mut mine = base.clone();
        mine.replace(16..32, "Rewritten in layout.");
        let m = merged(&base, &mine, &story("First sentence, revised. Second sentence."));
        assert_eq!(m.story.text, "First sentence, revised. Rewritten in layout.");
        assert!(!m.text.deviations.is_empty() && m.text.conflicts.is_empty());
        assert!(m.story.editorial.is_empty());
    }

    #[test]
    fn when_both_rewrite_a_sentence_the_file_wins_and_the_layouts_goes_in_a_flow_note() {
        let base = story("First sentence. Second sentence.");
        let mut mine = base.clone();
        mine.replace(16..32, "Layout version.");
        let m = merged(&base, &mine, &story("First sentence. File version."));
        assert_eq!(m.story.text.replace(NOTE_MARK, ""), "First sentence. File version.");
        assert_eq!(m.text.conflicts.len(), 1);
        let notes: Vec<(&str, &str)> = m.story.editorial.iter().map(|n| (n.author.as_str(), n.text.as_str())).collect();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, "Flow");
        assert!(notes[0].1.starts_with("Layout had: "), "{notes:?}");
    }

    /// A story of `text` and then a table of `rows`.
    fn with_table(text: &str, rows: &[&[&str]]) -> Story {
        let mut st = story(text);
        let data: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|c| (*c).to_string()).collect()).collect();
        let end = st.len();
        st.insert_table(end, designcraft_doc::Table::from_strings(1, &data, 300.0, &ParaFormat::default(), &designcraft_doc::CharFormat::default()));
        st
    }

    fn table(st: &Story) -> &designcraft_doc::Table {
        st.tables.values().next().unwrap()
    }

    fn cell(st: &Story, r: usize, c: usize) -> String {
        table(st).cell(r, c).unwrap().text.text.replace(NOTE_MARK, "")
    }

    fn edit_cell(st: &mut Story, r: usize, c: usize, text: &str) {
        let id = *st.tables.keys().next().unwrap();
        let cell = Arc::make_mut(st.tables.get_mut(&id).unwrap()).cell_mut(r, c).unwrap();
        let n = cell.text.len();
        cell.text.replace(0..n, text);
    }

    #[test]
    fn a_table_that_kept_its_shape_merges_cell_by_cell_and_keeps_its_formatting() {
        let base = with_table("Prices.", &[&["Item", "Price"], &["Tea", "3.50"]]);
        let mut mine = base.clone();
        edit_cell(&mut mine, 1, 0, "Green tea");
        let id = *mine.tables.keys().next().unwrap();
        let t = Arc::make_mut(mine.tables.get_mut(&id).unwrap());
        t.style = "Data".into();
        t.cell_mut(0, 0).unwrap().fill = "Black".into();
        let theirs = with_table("Prices.", &[&["Item", "Price"], &["Tea", "3.75"]]);
        let m = merged(&base, &mine, &theirs);
        assert_eq!((cell(&m.story, 1, 0), cell(&m.story, 1, 1)), ("Green tea".to_string(), "3.75".to_string()));
        assert_eq!((table(&m.story).style.as_str(), table(&m.story).cell(0, 0).unwrap().fill.as_str()), ("Data", "Black"));
        let kept: Vec<(usize, usize)> = m.cells.iter().filter(|c| !c.text.deviations.is_empty()).map(|c| (c.row, c.col)).collect();
        assert_eq!(kept, [(1, 0)], "the layout's cell edit is reported");
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    }

    #[test]
    fn a_cell_both_changed_takes_the_files_text_with_a_flow_note_in_the_cell() {
        let base = with_table("Prices.", &[&["Item", "Price"], &["Tea", "3.50"]]);
        let mut mine = base.clone();
        edit_cell(&mut mine, 1, 1, "3.60");
        let m = merged(&base, &mine, &with_table("Prices.", &[&["Item", "Price"], &["Tea", "3.75"]]));
        assert_eq!(cell(&m.story, 1, 1), "3.75");
        let notes = &table(&m.story).cell(1, 1).unwrap().text.editorial;
        assert_eq!(notes.iter().map(|n| (n.author.as_str(), n.text.as_str())).collect::<Vec<_>>(), [("Flow", "Layout had: 3.60")]);
        assert_eq!(m.cells.iter().map(|c| c.text.conflicts.len()).sum::<usize>(), 1);
    }

    #[test]
    fn a_table_whose_shape_changed_comes_from_the_file_with_a_warning() {
        let base = with_table("Prices.", &[&["Item", "Price"], &["Tea", "3.50"]]);
        let mut mine = base.clone();
        edit_cell(&mut mine, 1, 0, "Green tea");
        let m = merged(&base, &mine, &with_table("Prices.", &[&["Item", "Price"], &["Tea", "3.50"], &["Cake", "12.00"]]));
        assert_eq!((table(&m.story).nrows(), cell(&m.story, 1, 0)), (3, "Tea".to_string()));
        assert!(m.warnings.iter().any(|w| w.contains("rows or columns")), "{:?}", m.warnings);
    }

    const PIECES: &[&str] = &["a", "b", " ", ".", "\n", "é", "中", "👍🏽"];

    /// `st` after random edits: insertions and deletions at character positions, bold ranges,
    /// paragraph styles.
    fn edit(st: &Story, ops: &[(u8, usize, usize)]) -> Story {
        let mut st = st.clone();
        for &(kind, a, b) in ops {
            let at: Vec<usize> = st.text.char_indices().map(|(i, _)| i).chain([st.text.len()]).collect();
            let (x, y) = (at[a % at.len()], at[b % at.len()]);
            let (x, y) = (x.min(y), x.max(y));
            match kind % 4 {
                0 => st.insert(x, PIECES[b % PIECES.len()]),
                1 => st.delete(x..y),
                2 => st.format_chars(x..y, |f| f.over.font_style = Some("Bold".into())),
                _ => {
                    let n = st.paras.len();
                    st.paras[a % n].style = "Other".into();
                }
            }
        }
        st
    }

    proptest::proptest! {
        /// Tables that keep their shape merge cell by cell: a cell only the file changed takes the
        /// file's text, one only the layout changed keeps the layout's, one both changed differently
        /// takes the file's with a Flow note in the cell.
        #[test]
        fn merging_edited_tables_follows_the_rules_in_every_cell(
            cells in proptest::collection::vec(0..PIECES.len(), 4),
            mine in proptest::collection::vec((0..4usize, 0..PIECES.len()), 0..4),
            theirs in proptest::collection::vec((0..4usize, 0..PIECES.len()), 0..4),
        ) {
            let text: Vec<String> = cells.iter().map(|i| format!("v{}", PIECES[*i].trim())).collect();
            let base = with_table("T.", &[&[&text[0], &text[1]], &[&text[2], &text[3]]]);
            let edit = |edits: &[(usize, usize)]| {
                let mut st = base.clone();
                for &(i, p) in edits {
                    let now = cell(&st, i / 2, i % 2);
                    edit_cell(&mut st, i / 2, i % 2, &format!("{now}{}", PIECES[p].trim()));
                }
                st
            };
            let (m_story, t_story) = (edit(&mine), edit(&theirs));
            let m = merge_story(&base, &m_story, &t_story).ok_or_else(|| proptest::test_runner::TestCaseError::fail("no merge"))?;
            proptest::prop_assert!(m.story.check().is_ok());
            for i in 0..4 {
                let (b, y, t, out) = (cell(&base, i / 2, i % 2), cell(&m_story, i / 2, i % 2), cell(&t_story, i / 2, i % 2), cell(&m.story, i / 2, i % 2));
                let expected = if y == b || y == t { t } else if t == b { y } else { t };
                proptest::prop_assert_eq!(out, expected, "cell {}", i);
            }
        }

        #[test]
        fn merging_edited_stories_always_gives_a_valid_exact_story(
            base in proptest::collection::vec(0..PIECES.len(), 0..24),
            mine in proptest::collection::vec(proptest::prelude::any::<(u8, usize, usize)>(), 0..6),
            theirs in proptest::collection::vec(proptest::prelude::any::<(u8, usize, usize)>(), 0..6),
        ) {
            let base = story(&base.iter().map(|i| PIECES[*i]).collect::<String>());
            let (mine, theirs) = (edit(&base, &mine), edit(&base, &theirs));
            let m = merge_story(&base, &mine, &theirs).ok_or_else(|| proptest::test_runner::TestCaseError::fail("no merge"))?;
            proptest::prop_assert!(m.story.check().is_ok(), "{:?}", m.story.check());
            proptest::prop_assert_eq!(crate::apply(&theirs.text, &m.story.text, &m.text.deviations), Some(m.story.text.clone()));
        }
    }
}
