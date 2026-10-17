//! Type › Track Changes: record edits as inserted / deleted text; accept or reject them. Changes
//! in table cells and footnotes (Compare's redline marks them) are listed and resolved too.

use std::sync::Arc;

use designcraft_doc::{CellAddr, ChangeMark, Story, StoryId};
use serde_json::{Value, json};

use super::{CommandSpec, cmd, has_doc};
use crate::{Result, Session};

/// Mark `r` deleted (text this session inserted while tracking is simply removed). Returns where
/// the text after the range now starts.
pub(crate) fn mark_deleted(st: &mut Story, r: std::ops::Range<usize>) -> usize {
    if r.is_empty() {
        return r.start;
    }
    // Inserted pieces inside the range go away; the rest is marked deleted.
    let mut ins: Vec<std::ops::Range<usize>> = st
        .runs()
        .filter(|(rr, f)| f.over.change == Some(ChangeMark::Inserted) && rr.start < r.end && rr.end > r.start)
        .map(|(rr, _)| rr.start.max(r.start)..rr.end.min(r.end))
        .collect();
    let mut end = r.end;
    ins.sort_by_key(|x| std::cmp::Reverse(x.start));
    for x in ins {
        st.delete(x.clone());
        end -= x.len();
    }
    if end > r.start {
        st.format_chars(r.start..end, |f| f.over.change = Some(ChangeMark::Deleted));
    }
    end
}

/// Ranges of `mark` in `st`, last first.
fn ranges(st: &Story, mark: ChangeMark) -> Vec<std::ops::Range<usize>> {
    let mut v: Vec<std::ops::Range<usize>> = st.runs().filter(|(_, f)| f.over.change == Some(mark)).map(|(r, _)| r).collect();
    v.sort_by_key(|r| std::cmp::Reverse(r.start));
    v
}

/// Accept (or reject) every change in `st`.
fn resolve(st: &mut Story, accept: bool) -> usize {
    let (remove, keep) = if accept { (ChangeMark::Deleted, ChangeMark::Inserted) } else { (ChangeMark::Inserted, ChangeMark::Deleted) };
    let removed = ranges(st, remove);
    let n = removed.len() + ranges(st, keep).len();
    for r in removed {
        st.delete(r);
    }
    let len = st.len();
    st.format_chars(0..len, |f| {
        if f.over.change.is_some() {
            f.over.change = None;
        }
    });
    n
}

/// The texts of `st` Track Changes reaches: its own (no address), its tables' cells, its footnotes.
fn texts(st: &Story) -> Vec<(Option<CellAddr>, &Story)> {
    let mut out = vec![(None, st)];
    for (id, t) in &st.tables {
        let cols = t.ncols().max(1);
        out.extend(t.cells.iter().enumerate().map(|(i, c)| (Some(CellAddr { table: *id, row: i / cols, col: i % cols }), &c.text)));
    }
    out.extend(st.notes.iter().map(|n| (Some(CellAddr::footnote(n.id)), &n.text)));
    out
}

/// Where a text of `st` comes in the list's order: its own text, then its cells (by table, then
/// cell), then its footnotes.
fn rank(st: &Story, at: Option<CellAddr>) -> (u8, u64, usize) {
    match at {
        None => (0, 0, 0),
        Some(a) => match a.footnote_id() {
            Some(n) => (2, st.notes.iter().position(|x| x.id == n).unwrap_or(usize::MAX) as u64, 0),
            None => (1, a.table, a.row.saturating_mul(st.tables.get(&a.table).map_or(1, |t| t.ncols().max(1))).saturating_add(a.col)),
        },
    }
}

/// Every tracked change in `d` as changes.list gives them, in order, each with its sort key
/// (story, its text's rank, start) and where it is.
pub(super) fn listed(d: &designcraft_doc::Document) -> Vec<((u64, (u8, u64, usize), usize), Option<CellAddr>, Value)> {
    let mut out = Vec::new();
    for st in d.stories.values() {
        for (at, text) in texts(st) {
            for (r, f) in text.runs() {
                let kind = match f.over.change {
                    Some(ChangeMark::Inserted) => "inserted",
                    Some(ChangeMark::Deleted) => "deleted",
                    _ => continue,
                };
                let mut e = json!({"story": st.id.0, "start": r.start, "end": r.end, "kind": kind, "text": text.text[r.clone()].to_string()});
                match at.map(|a| (a, a.footnote_id())) {
                    Some((_, Some(n))) => e["footnote"] = json!(n),
                    Some((a, None)) => e["cell"] = json!(a),
                    None => {}
                }
                out.push(((st.id.0, rank(st, at), r.start), at, e));
            }
        }
    }
    out
}

/// The page of a change at `start` in story `sid`'s text (`at`: in a cell, the page of its table's
/// anchor; in a footnote, of its reference). `None` when it's overset.
pub(super) fn change_page(s: &Session, d: &designcraft_doc::Document, sid: StoryId, at: Option<CellAddr>, start: usize) -> Option<usize> {
    let st = d.story(sid)?;
    let pos = match at {
        None => Some(start),
        Some(a) => match a.footnote_id() {
            Some(n) => st.note_anchor(n),
            None => st
                .text
                .match_indices(designcraft_doc::TABLE_ANCHOR)
                .map(|(i, _)| i)
                .find(|i| st.paras.get(st.para_at(*i)).and_then(|p| p.table) == Some(a.table)),
        },
    }?;
    let cs = s.cache.get(d, sid, None);
    cs.frames.iter().find(|f| f.range.start <= pos && pos <= f.range.end).and_then(|f| d.page_of_item(f.frame))
}

/// Next (or previous) Change: select the change after (before) the selection in the list's order,
/// round to the first (last) past the end, and say its page.
fn step(s: &mut Session, next: bool) -> Result<Value> {
    let id = if next { "changes.next" } else { "changes.previous" };
    let d = s.doc()?.doc.clone();
    let all = listed(&d);
    let here = s.doc()?.selection.text.and_then(|t| Some((t.story.0, rank(d.story(t.story)?, t.cell), t.range().start)));
    let found = match here {
        Some(h) if next => all.iter().find(|(k, ..)| *k > h),
        Some(h) => all.iter().rev().find(|(k, ..)| *k < h),
        None => None,
    };
    let wrapped = found.is_none() && here.is_some();
    let ((sid, ..), at, entry) =
        found.or(if next { all.first() } else { all.last() }).ok_or_else(|| super::bad(id, "there are no tracked changes"))?.clone();
    let (start, end) = (entry["start"].as_u64().unwrap_or(0) as usize, entry["end"].as_u64().unwrap_or(0) as usize);
    let sid = StoryId(sid);
    let page = change_page(s, &d, sid, at, start);
    let st = s.doc_mut()?;
    st.selection = designcraft_doc::Selection::text(designcraft_doc::TextSel { story: sid, anchor: start, focus: end, frame: None, cell: at });
    st.bump_revision();
    let mut out = entry;
    out["page"] = json!(page);
    out["wrapped"] = json!(wrapped);
    Ok(out)
}

fn has_changes(st: &Story) -> bool {
    st.runs().any(|(_, f)| matches!(f.over.change, Some(ChangeMark::Inserted | ChangeMark::Deleted)))
}

/// Whether `d` has tracked changes left to accept or reject, in any story, table cell or footnote.
pub(crate) fn tracked(d: &designcraft_doc::Document) -> bool {
    d.stories.values().any(|st| texts(st).into_iter().any(|(_, t)| has_changes(t)))
}

/// The text a command names in its story: `cell: {table, row, col}`, `footnote: id`, or neither.
pub(super) fn address(p: &Value, id: &str) -> Result<Option<CellAddr>> {
    let given = |k: &str| p.get(k).filter(|v| !v.is_null());
    match (given("cell"), given("footnote")) {
        (Some(_), Some(_)) => Err(super::bad(id, "give `cell` or `footnote`, not both")),
        (Some(c), None) => serde_json::from_value(c.clone()).map(Some).map_err(|e| super::bad(id, format!("cell: {{table, row, col}} ({e})"))),
        (None, Some(n)) => n.as_u64().map(|n| Some(CellAddr::footnote(n))).ok_or_else(|| super::bad(id, "footnote: a footnote's id")),
        (None, None) => Ok(None),
    }
}

/// Accept or reject the one change at `start` in story `story` (in its cell or footnote when
/// given).
fn one(s: &mut Session, p: &Value, accept: bool) -> Result<Value> {
    let id = if accept { "changes.accept" } else { "changes.reject" };
    let sid = p.get("story").and_then(Value::as_u64).map(StoryId).ok_or_else(|| super::bad(id, "`story` required"))?;
    let start = p.get("start").and_then(Value::as_u64).ok_or_else(|| super::bad(id, "`start` required"))? as usize;
    let at = address(p, id)?;
    s.edit(|d, sel| {
        let st = d.text_story_mut(sid, at).ok_or_else(|| super::bad(id, "no such story, cell or footnote"))?;
        let (r, mark) = st
            .runs()
            .find(|(r, f)| matches!(f.over.change, Some(ChangeMark::Inserted | ChangeMark::Deleted)) && r.start <= start && start < r.end)
            .map(|(r, f)| (r, f.over.change.unwrap_or_default()))
            .ok_or_else(|| super::bad(id, "no change there"))?;
        // Accepting a deletion or rejecting an insertion removes the text; otherwise it stays, unmarked.
        if (mark == ChangeMark::Deleted) == accept {
            st.delete(r.clone());
        } else {
            st.format_chars(r.clone(), |f| f.over.change = None);
        }
        if let Some(t) = &mut sel.text
            && t.story == sid
            && t.cell == at
        {
            let len = d.text_story(sid, at).map_or(0, |x| x.len());
            t.anchor = t.anchor.min(len);
            t.focus = t.focus.min(len);
        }
        Ok(json!({"start": r.start, "end": r.end}))
    })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("changes.track", "Track Changes", ["Type", "Track Changes"], None, "{on?: bool (default: toggle)}", has_doc, |s, p| {
            let cur = s.doc()?.doc.settings.track_changes;
            let on = p.get("on").and_then(Value::as_bool).unwrap_or(!cur);
            s.edit(|d, _| {
                d.settings.track_changes = on;
                Ok(json!({"on": on}))
            })
        }),
        cmd!(query "changes.list", "Changes", [], None, "{} → [{story, start, end, kind: inserted|deleted, text, cell?: {table, row, col}, footnote?: id}] (offsets in the cell's or footnote's text when given)", has_doc, |s, _| {
            Ok(Value::Array(listed(&s.doc()?.doc).into_iter().map(|(.., e)| e).collect()))
        }),
        cmd!(
            noundo "changes.next",
            "Next Change",
            ["Type", "Track Changes"],
            None,
            "{} — selects the change after the selection (story by story: its text, its table cells, its footnotes), round to the first after the last → {story, start, end, kind, text, cell?, footnote?, page: of the change (null: overset), wrapped}",
            has_doc,
            |s, _| step(s, true)
        ),
        cmd!(
            noundo "changes.previous",
            "Previous Change",
            ["Type", "Track Changes"],
            None,
            "{} — selects the change before the selection, round to the last before the first → as changes.next",
            has_doc,
            |s, _| step(s, false)
        ),
        cmd!(
            noundo "view.showDeletedText",
            "Show Deleted Text",
            ["Type", "Track Changes"],
            None,
            "{on?: bool (default: toggle)} — lay out text tracked changes delete, struck through, on screen (Compare's redline turns it on); exports, printing and saved previews never have it → {on}",
            super::always,
            |s, p| {
                let on = p.get("on").and_then(Value::as_bool).unwrap_or(!s.cache.shows_deleted());
                s.cache.set_show_deleted(on);
                Ok(json!({"on": on}))
            }
        ),
        cmd!(
            "changes.accept",
            "Accept Change",
            [],
            None,
            "{story, start, cell?: {table, row, col}, footnote?: id} — the change at that position (in that cell's or footnote's text)",
            has_doc,
            |s, p| one(s, p, true)
        ),
        cmd!(
            "changes.reject",
            "Reject Change",
            [],
            None,
            "{story, start, cell?: {table, row, col}, footnote?: id} — the change at that position (in that cell's or footnote's text)",
            has_doc,
            |s, p| one(s, p, false)
        ),
        cmd!(
            "changes.acceptAll",
            "Accept All Changes",
            ["Type", "Track Changes"],
            None,
            "{story?} — deleted text goes, added text stays (in the stories, their table cells and footnotes)",
            has_doc,
            |s, p| all(s, p, true)
        ),
        cmd!(
            "changes.rejectAll",
            "Reject All Changes",
            ["Type", "Track Changes"],
            None,
            "{story?} — added text goes, deleted text comes back (in the stories, their table cells and footnotes)",
            has_doc,
            |s, p| all(s, p, false)
        ),
    ]
}

fn all(s: &mut Session, p: &Value, accept: bool) -> Result<Value> {
    let only = p.get("story").and_then(Value::as_u64).map(StoryId);
    s.edit(|d, sel| {
        let ids: Vec<StoryId> = d.stories.keys().copied().filter(|id| only.is_none_or(|o| o == *id)).collect();
        let mut n = 0;
        for id in ids {
            if let Some(st) = d.story_mut(id) {
                // Cells and footnotes first: accepting a story's deletion can take a table or note away.
                for t in st.tables.values_mut().filter(|t| t.cells.iter().any(|c| has_changes(&c.text))) {
                    for c in Arc::make_mut(t).cells.iter_mut() {
                        n += resolve(&mut c.text, accept);
                    }
                }
                for note in st.notes.iter_mut().filter(|n| has_changes(&n.text)) {
                    n += resolve(&mut Arc::make_mut(note).text, accept);
                }
                n += resolve(st, accept);
            }
        }
        // The caret may now be past the end.
        if let Some(t) = &mut sel.text
            && let Some(st) = d.story(t.story)
        {
            t.anchor = t.anchor.min(st.len());
            t.focus = t.focus.min(st.len());
        }
        Ok(json!({"changes": n}))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    /// Track Changes reaches table cells and footnotes (Compare's redline marks them): their changes
    /// are listed with where they are, and accepted or rejected one by one or all at once.
    #[test]
    fn changes_in_cells_and_footnotes_are_listed_accepted_and_rejected() {
        use designcraft_doc::{CellAddr, ChangeMark, StoryId};
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 400, 300], "content": "text", "text": "Body."})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        s.execute("text.select", &json!({"story": sid.0, "anchor": 5, "focus": 5})).unwrap();
        let note = s.execute("footnote.insert", &json!({"text": "Source"})).unwrap()["id"].as_u64().unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("table.insert", &json!({"rows": 2, "cols": 2})).unwrap();
        let table = *s.doc().unwrap().doc.story(sid).unwrap().tables.keys().next().unwrap();
        let cell = CellAddr { table, row: 0, col: 1 };
        // As Compare marks them: the old value deleted, the new one inserted.
        let marked = |s: &mut Session, at: CellAddr, text: &str, deleted: &str, inserted: &str| {
            s.edit(|d, _| {
                let st = d.text_story_mut(sid, Some(at)).unwrap();
                let n = st.len();
                st.replace(0..n, text);
                for (word, m) in [(deleted, ChangeMark::Deleted), (inserted, ChangeMark::Inserted)] {
                    let i = st.text.find(word).unwrap();
                    st.format_chars(i..i + word.len(), |f| f.over.change = Some(m));
                }
                Ok(serde_json::Value::Null)
            })
            .unwrap();
        };
        marked(&mut s, cell, "3.50 3.75", "3.50 ", "3.75");
        marked(&mut s, CellAddr::footnote(note), "Source: 2025 2026.", "2025 ", "2026");
        let listed = s.execute("changes.list", &json!({})).unwrap();
        let entries: Vec<(serde_json::Value, serde_json::Value, String, String)> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["cell"].clone(), c["footnote"].clone(), c["kind"].as_str().unwrap().into(), c["text"].as_str().unwrap().into()))
            .collect();
        let at_cell = json!({"table": table, "row": 0, "col": 1});
        assert_eq!(
            entries,
            [
                (at_cell.clone(), json!(null), "deleted".into(), "3.50 ".into()),
                (at_cell.clone(), json!(null), "inserted".into(), "3.75".into()),
                (json!(null), json!(note), "deleted".into(), "2025 ".into()),
                (json!(null), json!(note), "inserted".into(), "2026".into()),
            ]
        );
        let text = |s: &Session, at: CellAddr| s.doc().unwrap().doc.text_story(sid, Some(at)).unwrap().text.clone();
        s.execute("changes.accept", &json!({"story": sid.0, "cell": at_cell, "start": 0})).unwrap();
        assert_eq!(text(&s, cell), "3.75");
        s.execute("changes.reject", &json!({"story": sid.0, "footnote": note, "start": 13})).unwrap();
        assert_eq!(text(&s, CellAddr::footnote(note)), "Source: 2025 .");
        s.execute("changes.acceptAll", &json!({})).unwrap();
        assert_eq!((text(&s, cell), text(&s, CellAddr::footnote(note))), ("3.75".to_string(), "Source: .".to_string()));
        assert!(s.execute("changes.list", &json!({})).unwrap().as_array().unwrap().is_empty());
        marked(&mut s, cell, "old new", "old ", "new");
        s.execute("changes.rejectAll", &json!({})).unwrap();
        assert_eq!(text(&s, cell), "old ");
    }

    /// Next and Previous Change step through every change in the list's order (a story's text, its
    /// cells, its footnotes), selecting each and saying its page, and wrap round at the ends.
    #[test]
    fn next_and_previous_step_through_every_change() {
        use designcraft_doc::{CellAddr, ChangeMark, StoryId};
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 400, 300], "content": "text", "text": "One two three"})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        s.execute("text.select", &json!({"story": sid.0, "anchor": 13, "focus": 13})).unwrap();
        let note = s.execute("footnote.insert", &json!({"text": "Note"})).unwrap()["id"].as_u64().unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("table.insert", &json!({"rows": 1, "cols": 2})).unwrap();
        let table = *s.doc().unwrap().doc.story(sid).unwrap().tables.keys().next().unwrap();
        let cell = CellAddr { table, row: 0, col: 1 };
        s.edit(|d, _| {
            let st = d.story_mut(sid).unwrap();
            for word in ["two", "three"] {
                let i = st.text.find(word).unwrap();
                st.format_chars(i..i + word.len(), |f| f.over.change = Some(ChangeMark::Deleted));
            }
            let c = d.text_story_mut(sid, Some(cell)).unwrap();
            c.replace(0..0, "3.75");
            c.format_chars(0..4, |f| f.over.change = Some(ChangeMark::Inserted));
            let n = d.text_story_mut(sid, Some(CellAddr::footnote(note))).unwrap();
            n.format_chars(0..4, |f| f.over.change = Some(ChangeMark::Inserted));
            Ok(serde_json::Value::Null)
        })
        .unwrap();
        let step = |s: &mut Session, id: &str| {
            let r = s.execute(id, &json!({})).unwrap();
            let t = s.doc().unwrap().selection.text.unwrap();
            assert_eq!((t.story, t.range().start), (sid, r["start"].as_u64().unwrap() as usize), "{r}");
            assert_eq!(r["page"], 0, "{r}");
            (r["text"].as_str().unwrap().to_string(), r["wrapped"] == true)
        };
        // From the story's start (Create Table left the caret in its first cell).
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        let forward: Vec<(String, bool)> = (0..5).map(|_| step(&mut s, "changes.next")).collect();
        let names = |v: &[(String, bool)]| v.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>();
        assert_eq!(names(&forward), ["two", "three", "3.75", "Note", "two"], "{forward:?}");
        assert_eq!(forward.iter().map(|(_, w)| *w).collect::<Vec<_>>(), [false, false, false, false, true]);
        assert_eq!(s.doc().unwrap().selection.text.unwrap().cell, None);
        let back: Vec<(String, bool)> = (0..2).map(|_| step(&mut s, "changes.previous")).collect();
        assert_eq!(names(&back), ["Note", "3.75"]);
        assert_eq!(s.doc().unwrap().selection.text.unwrap().cell, Some(cell), "in the cell");
    }

    /// `text.select` reaches a cell's or footnote's text, as the changes there are addressed.
    #[test]
    fn text_selects_in_a_cell_or_footnote() {
        use designcraft_doc::{CellAddr, StoryId};
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 400, 300], "content": "text", "text": "Body."})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        s.execute("text.select", &json!({"story": sid.0, "anchor": 5, "focus": 5})).unwrap();
        let note = s.execute("footnote.insert", &json!({"text": "Source: 2025."})).unwrap()["id"].as_u64().unwrap();
        s.execute("text.select", &json!({"story": sid.0, "footnote": note, "anchor": 8, "focus": 12})).unwrap();
        let t = s.doc().unwrap().selection.text.unwrap();
        assert_eq!((t.cell, t.range()), (Some(CellAddr::footnote(note)), 8..12));
        assert!(s.execute("text.select", &json!({"story": sid.0, "cell": {"table": 99, "row": 0, "col": 0}, "anchor": 0, "focus": 0})).is_err());
    }

    #[test]
    fn tracked_edits_accept_and_reject() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 400, 200], "content": "text", "text": "The cat sat"})).unwrap();
        let sid = designcraft_doc::StoryId(r["story"].as_u64().unwrap());
        s.execute("changes.track", &json!({"on": true})).unwrap();
        // Replace "cat" with "dog", then type and erase a typo (gone without a trace).
        s.execute("text.select", &json!({"story": sid.0, "anchor": 4, "focus": 7})).unwrap();
        s.execute("text.insert", &json!({"text": "dog"})).unwrap();
        s.execute("text.insert", &json!({"text": "x"})).unwrap();
        s.execute("text.delete", &json!({})).unwrap();
        let text = |s: &Session| s.doc().unwrap().doc.stories[&sid].text.clone();
        assert_eq!(text(&s), "The catdog sat", "deleted text stays (marked)");
        let l = s.execute("changes.list", &json!({})).unwrap();
        assert_eq!(l.as_array().unwrap().len(), 2, "{l}");
        // Deleted text takes no room in the layout.
        let d = s.doc().unwrap().doc.clone();
        let cs = s.cache.get(&d, sid, None);
        assert!(cs.frames[0].lines[0].glyphs.iter().any(|g| !g.visible && g.byte >= 4 && g.byte < 7));
        // One at a time: accept the deletion of "cat", then reject the insertion of "dog".
        s.execute("changes.accept", &json!({"story": sid.0, "start": 4})).unwrap();
        assert_eq!(text(&s), "The dog sat");
        s.execute("changes.reject", &json!({"story": sid.0, "start": 4})).unwrap();
        assert_eq!(text(&s), "The  sat");
        assert!(s.execute("changes.accept", &json!({"story": sid.0, "start": 0})).is_err());
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(text(&s), "The catdog sat");
        s.execute("changes.rejectAll", &json!({})).unwrap();
        assert_eq!(text(&s), "The cat sat");
        s.execute("edit.undo", &json!({})).unwrap();
        let exported = s.execute("file.exportText", &json!({"story": sid.0})).unwrap()["text"].as_str().unwrap().to_string();
        assert_eq!(exported, "The dog sat", "deleted text isn't exported");
        s.execute("changes.acceptAll", &json!({})).unwrap();
        assert_eq!(text(&s), "The dog sat");
        assert!(s.execute("changes.list", &json!({})).unwrap().as_array().unwrap().is_empty());
    }
}
