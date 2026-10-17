//! Compare (Stage 4): the active document against an earlier version of it, opened as a redline.

use std::sync::Arc;

use designcraft_doc::{ChangeMark, Document, FOOTNOTE_REF, Story, TABLE_ANCHOR};
use designcraft_flow::{ChangeSet, Mark, Place, Run, Status, TableStatus};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc};
use crate::{DocState, Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            noundo "compare.exportPdf",
            "Export Changes as PDF…",
            [],
            None,
            "{path} — the active document's tracked changes (a Compare redline's) as a PDF: a summary page listing them by page, then each page with a change, added text highlighted and deleted text struck through (whether or not Show Deleted Text is on) → {path, pages, changes, bytes}",
            has_doc,
            export_pdf
        ),
        cmd!(
            noundo "compare.file",
            "Compare with Earlier Version",
            [],
            None,
            "{path | base64, name?} — the active document compared with an earlier version of it saved at `path` (a .designcraft): opens the redline, a new unsaved document where the earlier version's lost text is back, marked deleted, and the new text marked inserted, word by word, in stories, table cells and footnotes, for Track Changes to list, accept and reject; turns on Show Deleted Text → {redline: its document index, changes, stories (the redline's marked changes and their stories, as changes.list gives them), deletedStories: [story ids only the earlier version has, not shown], tables: [{story, table, status: reshaped|inserted|deleted}]}",
            has_doc,
            compare_file
        ),
    ]
}

/// "Show Deleted Text" on until dropped, then as it was (Compare's PDF shows the deleted text).
struct DeletedShown(std::sync::Arc<designcraft_compose::Cache>, bool);

impl Drop for DeletedShown {
    fn drop(&mut self) {
        self.0.set_show_deleted(self.1);
    }
}

fn export_pdf(s: &mut Session, p: &Value) -> Result<Value> {
    let path = super::str_param(p, "path").ok_or_else(|| bad("compare.exportPdf", "missing `path`"))?.to_string();
    let _shown = {
        let was = s.cache.shows_deleted();
        s.cache.set_show_deleted(true);
        DeletedShown(s.cache.clone(), was)
    };
    let doc = s.doc()?.doc.clone();
    let marked = super::changes::listed(&doc);
    if marked.is_empty() {
        return Err(bad("compare.exportPdf", "the document has no tracked changes"));
    }
    // Each change with its page, and the pages that have any.
    let lang_page = |p: Option<usize>| p.map_or_else(|| "Overset".to_string(), |p| format!("Page {}", doc.page_name(p)));
    let lines: Vec<(Option<usize>, String)> = marked
        .iter()
        .map(|((sid, ..), at, e)| {
            let page = super::changes::change_page(s, &doc, designcraft_doc::StoryId(*sid), *at, e["start"].as_u64().unwrap_or(0) as usize);
            let kind = if e["kind"] == "inserted" { "Added" } else { "Deleted" };
            let text: String = e["text"].as_str().unwrap_or_default().chars().filter(|c| !('\u{E000}'..='\u{F8FF}').contains(c)).take(60).collect();
            (page, format!("{}: {kind} “{}”", lang_page(page), text.trim().replace('\n', " ")))
        })
        .collect();
    let mut pages: Vec<usize> = lines.iter().filter_map(|(p, _)| *p).collect();
    pages.sort_unstable();
    pages.dedup();
    // The summary: a page after the last (one before the first would move every page's side),
    // exported first.
    let mut d = (*doc).clone();
    let last = d.page_count().saturating_sub(1);
    d.insert_pages(Some(last), 1, None)?;
    let summary = d.page_count().saturating_sub(1);
    let (si, pi) = d.page_loc(summary).ok_or_else(|| bad("compare.exportPdf", "no summary page"))?;
    let page = d.spreads.get(si).and_then(|sp| sp.pages.get(pi)).ok_or_else(|| bad("compare.exportPdf", "no summary page"))?;
    let rect = page.margin_rect();
    const SHOWN: usize = 40;
    let stories = marked.iter().map(|((sid, ..), ..)| *sid).collect::<std::collections::BTreeSet<_>>().len();
    let mut text = format!(
        "Changes in {}\n{} change{} in {} stor{}",
        if doc.title.is_empty() { "Untitled" } else { &doc.title },
        marked.len(),
        if marked.len() == 1 { "" } else { "s" },
        stories,
        if stories == 1 { "y" } else { "ies" }
    );
    for (_, l) in lines.iter().take(SHOWN) {
        text.push('\n');
        text.push_str(l);
    }
    if lines.len() > SHOWN {
        text.push_str(&format!("\n… and {} more", lines.len() - SHOWN));
    }
    let layer = d.default_layer();
    d.add_text_frame(designcraft_doc::SpreadRef::Doc(si), rect, layer, &text, designcraft_doc::ParaFormat::default())?;
    let order: Vec<usize> = std::iter::once(summary).chain(pages.iter().copied()).collect();
    let opts = designcraft_pdf::PdfOptions {
        pages: Some(order.clone()),
        change_markup: true,
        title: Some(format!("Changes in {}", if doc.title.is_empty() { "Untitled" } else { &doc.title })),
        ..Default::default()
    };
    let bytes =
        designcraft_pdf::export_pdf(&super::interchange::with_pdf_boxes(&d), &s.cache, &opts).map_err(|e| bad("compare.exportPdf", e.to_string()))?;
    #[cfg(not(target_arch = "wasm32"))]
    crate::io::write_file(&path, &bytes).map_err(crate::EngineError::Other)?;
    Ok(json!({"path": path, "pages": order.len(), "changes": marked.len(), "bytes": bytes.len()}))
}

fn compare_file(s: &mut Session, p: &Value) -> Result<Value> {
    let (bytes, name, _) = super::file::read_source(p)?;
    let a = designcraft_format::load(&bytes).map_err(|e| bad("compare.file", format!("{name}: {e}")))?;
    let b = s.doc()?.doc.clone();
    for (d, which) in [(&a, name.as_str()), (&b, "This document")] {
        if super::changes::tracked(d) {
            return Err(bad("compare.file", format!("{which} has tracked changes: accept or reject them first")));
        }
    }
    let changes = designcraft_flow::compare(&a, &b);
    let mut red = redline(&a, &b, &changes);
    red.title = format!("{} compared with {name}", if b.title.is_empty() { "Untitled" } else { &b.title });
    let deleted: Vec<u64> = changes.stories.iter().filter(|c| c.status == Status::Deleted).map(|c| c.story.0).collect();
    let status = |t: TableStatus| match t {
        TableStatus::Reshaped => "reshaped",
        TableStatus::Inserted => "inserted",
        TableStatus::Deleted => "deleted",
    };
    let tables: Vec<Value> = changes
        .stories
        .iter()
        .flat_map(|c| c.tables.iter().map(|t| json!({"story": c.story.0, "table": t.table, "status": status(t.status)})))
        .collect();
    // Counted as Track Changes lists them: each deleted and each inserted run.
    let marked = super::changes::listed(&red);
    let stories = marked.iter().map(|((story, ..), ..)| *story).collect::<std::collections::BTreeSet<_>>().len();
    let i = s.add_document(DocState::new(red, None));
    // A redline shows what was deleted.
    s.cache.set_show_deleted(true);
    Ok(json!({"redline": i, "changes": marked.len(), "stories": stories, "deletedStories": deleted, "tables": tables}))
}

/// The redline of `b` against `a` (architecture §4): `b` with what `a` had and `b` lost put back,
/// marked deleted, and what `b` added marked inserted, word by word, in its stories, table cells
/// and footnotes; a footnote or table whose reference or anchor is deleted or inserted is, all of
/// it. Track Changes then shows, lists and resolves the changes: accepting them all gives `b`'s
/// texts, rejecting them all `a`'s. Stories only `a` has (deleted with their frames) and tables
/// whose shape changed are listed by `compare.file`, not shown.
pub(crate) fn redline(a: &Document, b: &Document, changes: &ChangeSet) -> Document {
    let mut d = b.clone();
    for c in changes.stories.iter().filter(|c| c.status != Status::Deleted) {
        let empty = Story::new(c.story);
        let old = a.story(c.story).unwrap_or(&empty);
        let Some(st) = d.story_mut(c.story) else { continue };
        for t in &c.texts {
            match t.place {
                Place::Text => mark(st, old, &t.runs),
                // A table or footnote whose anchor or reference the text change moved or replaced
                // (an id can be used again) goes whole with it, below.
                Place::Cell { table, row, col } => {
                    let anchor = st
                        .text
                        .match_indices(TABLE_ANCHOR)
                        .map(|(at, _)| at)
                        .find(|at| st.paras.get(st.para_at(*at)).and_then(|p| p.table) == Some(table));
                    if anchor.is_some_and(|at| changed(st, at).is_some()) {
                        continue;
                    }
                    let Some(from) = old.tables.get(&table).and_then(|x| x.cell(row, col)).map(|x| x.text.clone()) else { continue };
                    if let Some(cell) = st.table_mut(table).and_then(|x| x.cell_mut(row, col)) {
                        mark(&mut cell.text, &from, &t.runs);
                    }
                }
                Place::Footnote { id } => {
                    if st.note_anchor(id).is_some_and(|at| changed(st, at).is_some()) {
                        continue;
                    }
                    let Some(from) = old.note(id).map(|n| n.text.clone()) else { continue };
                    if let Some(n) = st.note_mut(id) {
                        mark(&mut n.text, &from, &t.runs);
                    }
                }
            }
        }
        anchored(st);
    }
    d
}

/// `runs` (from `old`'s text to `new`'s) marked in `new`: the inserted words, and the deleted ones
/// put back where they were, with their formatting and anything anchored in them.
fn mark(new: &mut Story, old: &Story, runs: &[Run]) {
    for r in runs.iter().rev() {
        match r.mark {
            Mark::Inserted => new.format_chars(r.new.clone(), |f| f.over.change = Some(ChangeMark::Inserted)),
            Mark::Deleted => {
                let at = r.new.start;
                let end = new.insert_story(at, &old.extract(r.old.clone()));
                new.format_chars(at..end, |f| f.over.change = Some(ChangeMark::Deleted));
            }
            Mark::Same => {}
        }
    }
}

/// The change mark on the character at `at` in `st`, if any.
fn changed(st: &Story, at: usize) -> Option<ChangeMark> {
    st.format_after(at).over.change.filter(|m| *m != ChangeMark::None)
}

/// Footnotes and tables whose reference or anchor is deleted or inserted, marked so all through.
fn anchored(st: &mut Story) {
    let notes: Vec<(usize, ChangeMark)> =
        st.text.match_indices(FOOTNOTE_REF).enumerate().filter_map(|(k, (at, _))| Some((k, changed(st, at)?))).collect();
    let tables: Vec<(u64, ChangeMark)> =
        st.text.match_indices(TABLE_ANCHOR).filter_map(|(at, _)| Some((st.paras.get(st.para_at(at))?.table?, changed(st, at)?))).collect();
    let all = |text: &mut Story, m: ChangeMark| {
        let n = text.len();
        text.format_chars(0..n, |f| f.over.change = Some(m));
    };
    for (k, m) in notes {
        if let Some(n) = st.notes.get_mut(k) {
            all(&mut Arc::make_mut(n).text, m);
        }
    }
    for (id, m) in tables {
        if let Some(t) = st.table_mut(id) {
            for c in &mut t.cells {
                all(&mut c.text, m);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use designcraft_doc::{CellAddr, Document, StoryId};
    use serde_json::{Value, json};

    use crate::Session;

    /// A story's texts: its own, its cells' (table by table) and its footnotes', in order.
    fn texts(d: &Document, sid: StoryId) -> (String, Vec<String>, Vec<String>) {
        let st = d.story(sid).unwrap();
        let cells = st.tables.values().flat_map(|t| t.cells.iter().map(|c| c.text.text.clone())).collect();
        (st.text.clone(), cells, st.notes.iter().map(|n| n.text.text.clone()).collect())
    }

    /// Text `from` in story `sid` (in a cell or footnote when `at` names one) replaced by `to`.
    fn set(s: &mut Session, sid: StoryId, at: Option<CellAddr>, from: &str, to: &str) {
        s.edit(|d, _| {
            let st = d.text_story_mut(sid, at).unwrap();
            let i = st.text.find(from).unwrap();
            st.replace(i..i + from.len(), to);
            Ok(Value::Null)
        })
        .unwrap();
    }

    /// Compare's redline: the active document with what an earlier version had and lost put back,
    /// deleted, and what it added marked inserted, in its text, table cells and footnotes. Track
    /// Changes lists each change; accepting them all gives the document, rejecting them all the
    /// earlier version.
    #[test]
    fn the_redline_marks_every_change_and_gives_either_version_back() {
        let dir = std::env::temp_dir().join(format!("dc-compare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s
            .execute("frame.create", &json!({"rect": [72, 72, 400, 500], "content": "text", "text": "Revenue grew. Costs were flat. Staff held."}))
            .unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        // A footnote after "grew.", one at the end, and a table at the start.
        s.execute("text.select", &json!({"story": sid.0, "anchor": 13, "focus": 13})).unwrap();
        let kept = s.execute("footnote.insert", &json!({"text": "Source: 2025."})).unwrap()["id"].as_u64().unwrap();
        let end = s.doc().unwrap().doc.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
        let gone = s.execute("footnote.insert", &json!({"text": "Old note."})).unwrap()["id"].as_u64().unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("table.insert", &json!({"rows": 2, "cols": 2})).unwrap();
        let table = *s.doc().unwrap().doc.story(sid).unwrap().tables.keys().next().unwrap();
        let price = CellAddr { table, row: 1, col: 1 };
        set(&mut s, sid, Some(price), "", "3.50");
        let path = dir.join("v1.designcraft");
        std::fs::write(&path, designcraft_format::save(&s.doc().unwrap().doc).unwrap()).unwrap();
        let a = s.doc().unwrap().doc.clone();

        // The new version: words changed, a footnote changed, one deleted with its reference and
        // one added, a cell changed, and a new text frame.
        set(&mut s, sid, None, "were flat", "fell");
        set(&mut s, sid, Some(CellAddr::footnote(kept)), "2025", "2026");
        set(&mut s, sid, Some(price), "3.50", "3.75");
        s.edit(|d, _| {
            let st = d.story_mut(sid).unwrap();
            let at = st.note_anchor(gone).unwrap();
            st.delete(at..at + designcraft_doc::FOOTNOTE_REF.len_utf8());
            Ok(Value::Null)
        })
        .unwrap();
        let held = s.doc().unwrap().doc.story(sid).unwrap().text.find("held").unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": held, "focus": held})).unwrap();
        s.execute("footnote.insert", &json!({"text": "New note."})).unwrap();
        let added = StoryId(
            s.execute("frame.create", &json!({"rect": [72, 520, 400, 600], "content": "text", "text": "Added box."})).unwrap()["story"]
                .as_u64()
                .unwrap(),
        );
        let b = s.doc().unwrap().doc.clone();
        let compared = s.active_index();

        let r = s.execute("compare.file", &json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(s.active_index(), r["redline"].as_u64().map(|i| i as usize), "{r}");
        assert_ne!(s.active_index(), compared, "a new document");
        assert!(s.doc().unwrap().path.is_none(), "unsaved");
        let listed = s.execute("changes.list", &json!({})).unwrap();
        for (kind, text) in [
            ("deleted", "were flat"),
            ("inserted", "fell"),
            ("deleted", "2025"),
            ("inserted", "2026"),
            ("deleted", "3.50"),
            ("inserted", "3.75"),
            ("deleted", "Old note."),
            ("inserted", "New note."),
            ("inserted", "Added box."),
        ] {
            assert!(listed.as_array().unwrap().iter().any(|c| c["kind"] == kind && c["text"] == text), "{kind} {text:?} in {listed}");
        }
        s.execute("changes.acceptAll", &json!({})).unwrap();
        assert_eq!(texts(&s.doc().unwrap().doc, sid), texts(&b, sid), "accepted: the document");
        assert_eq!(s.doc().unwrap().doc.story(added).unwrap().text, "Added box.");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("changes.rejectAll", &json!({})).unwrap();
        assert_eq!(texts(&s.doc().unwrap().doc, sid), texts(&a, sid), "rejected: the earlier version");
        assert_eq!(s.doc().unwrap().doc.story(added).unwrap().text, "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What the redline can't show is listed: a story deleted with its frame, a table whose rows
    /// or columns changed. Tracked changes in either version are resolved first.
    #[test]
    fn deleted_stories_and_reshaped_tables_are_listed_and_tracked_changes_refused() {
        let dir = std::env::temp_dir().join(format!("dc-compare-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 400, 300], "content": "text", "text": "Prices"})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        let other = s.execute("frame.create", &json!({"rect": [72, 320, 400, 400], "content": "text", "text": "Goes away."})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 6})).unwrap();
        s.execute("table.insert", &json!({"rows": 2, "cols": 2})).unwrap();
        let path = dir.join("v1.designcraft");
        std::fs::write(&path, designcraft_format::save(&s.doc().unwrap().doc).unwrap()).unwrap();
        s.execute("edit.clear", &json!({"ids": [other["id"]]})).unwrap();
        let table = *s.doc().unwrap().doc.story(sid).unwrap().tables.keys().next().unwrap();
        s.edit(|d, _| {
            d.story_mut(sid).unwrap().table_mut(table).unwrap().insert_cols(2, 1, 72.0);
            Ok(Value::Null)
        })
        .unwrap();
        let r = s.execute("compare.file", &json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["deletedStories"], json!([other["story"]]), "{r}");
        assert_eq!(r["tables"], json!([{"story": sid.0, "table": table, "status": "reshaped"}]));
        s.execute("file.close", &json!({})).unwrap();
        s.execute("changes.track", &json!({"on": true})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("text.insert", &json!({"text": "New "})).unwrap();
        let e = s.execute("compare.file", &json!({"path": path.to_string_lossy()})).unwrap_err().to_string();
        assert!(e.contains("tracked changes"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "Show deleted text" is the screen's view: the layout then has the deleted text (struck
    /// through), output never does. Compare's redline turns it on.
    #[test]
    fn deleted_text_shows_on_screen_but_never_in_output() {
        let dir = std::env::temp_dir().join(format!("dc-show-deleted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 400, 200], "content": "text", "text": "The cat sat"})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        s.execute("changes.track", &json!({"on": true})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 4, "focus": 7})).unwrap();
        s.execute("text.insert", &json!({"text": "dog"})).unwrap();
        let shown = |s: &Session| {
            let d = s.doc().unwrap().doc.clone();
            let cs = s.cache.get(&d, sid, None);
            cs.frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).any(|g| g.visible && g.adv > 0.0 && (4..7).contains(&g.byte))
        };
        let preview = |s: &mut Session, name: &str| {
            let path = dir.join(name);
            s.execute("file.saveACopy", &json!({"path": path.to_string_lossy()})).unwrap();
            designcraft_format::read_preview(std::io::Cursor::new(std::fs::read(&path).unwrap())).expect("a preview")
        };
        assert!(!shown(&s));
        let plain = preview(&mut s, "plain.designcraft");
        assert_eq!(s.execute("view.showDeletedText", &json!({"on": true})).unwrap()["on"], true);
        assert!(shown(&s), "on screen, the deleted text takes room");
        assert_eq!(preview(&mut s, "shown.designcraft"), plain, "output leaves it out");
        assert!(shown(&s), "and the view stays on");
        s.execute("view.showDeletedText", &json!({})).unwrap();
        assert!(!shown(&s), "toggled off");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Compare's PDF: a summary page listing the changes, then each page with a change on it,
    /// deleted text shown. The redline itself is left as it was, and so is the view.
    #[test]
    fn the_changes_export_as_a_pdf_after_a_summary() {
        let dir = std::env::temp_dir().join(format!("dc-compare-pdf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 2})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 400, 200], "content": "text", "text": "Costs were flat."})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        s.execute("frame.create", &json!({"rect": [72, 72, 400, 200], "content": "text", "text": "Unchanged.", "spread": 1})).unwrap();
        let earlier = dir.join("sent.designcraft");
        s.execute("file.saveACopy", &json!({"path": earlier.to_string_lossy()})).unwrap();
        set(&mut s, sid, None, "were flat", "fell");
        s.execute("compare.file", &json!({"path": earlier.to_string_lossy()})).unwrap();
        s.execute("view.showDeletedText", &json!({"on": false})).unwrap();
        let out = dir.join("changes.pdf");
        let r = s.execute("compare.exportPdf", &json!({"path": out.to_string_lossy()})).unwrap();
        assert_eq!((r["pages"].clone(), r["changes"].clone()), (json!(2), json!(2)), "the summary and page 1: {r}");
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(designcraft_render::pdf_page_count(&bytes), Some(2));
        assert_eq!(s.doc().unwrap().doc.page_count(), 2, "the redline keeps its pages");
        assert!(!s.cache.shows_deleted(), "and the view as it was");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
