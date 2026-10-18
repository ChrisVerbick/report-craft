//! History (Stage 5): the document's versions, kept in its file. Saving a changed document keeps
//! a version, as do File › Save a Version and a Flow update; any version can be shown, exported,
//! compared or restored. Restoring keeps a version too: history is never rewritten.

use designcraft_doc::Document;
use designcraft_format::{History, NewVersion, VersionKind};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc, str_param};
use crate::{DocState, EngineError, Result, Selection, Session};

/// Saving keeps this many saved versions (the oldest go first); the other kinds stay.
const SAVES_KEPT: usize = 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "version.list",
            "Versions",
            [],
            None,
            "{} → [{id, parent, author, ts (Unix seconds), label, kind: save|manual|linkUpdate|restore, full, size, format, reports? (a Flow update's)}] the document's versions, oldest first",
            has_doc,
            |s, _| serde_json::to_value(s.doc()?.versions.versions().collect::<Vec<_>>()).map_err(|e| EngineError::Other(e.to_string()))
        ),
        cmd!(
            noundo "version.commit",
            "Save a Version…",
            [],
            None,
            "{label?, path?} — keep the document as a version (manual: saving never prunes it) and save it as file.save does → {id, path, bytes}",
            has_doc,
            commit
        ),
        cmd!(noundo "version.show", "Show Version", [], None, "{id} — open version `id` as a new, unsaved document → {index}", has_doc, show),
        cmd!(
            "version.restore",
            "Restore Version",
            [],
            None,
            "{id} — make the document version `id` again (undoable) and keep a version saying so; the versions before stay as they were → {id: the new version, restored}",
            has_doc,
            restore
        ),
        cmd!(
            noundo "version.prune",
            "Prune Versions",
            [],
            None,
            "{keepLast?: 20, keepManual?: true} — drop the oldest saved versions past the newest `keepLast` (manual, Flow update and restore versions too when keepManual is false); every version kept stays as it was; saving writes the change → {removed}",
            has_doc,
            prune
        ),
        cmd!(
            query "version.export",
            "Export Version",
            [],
            None,
            "{id, path} — write version `id` as a document of its own, without history; a path without the .designcraft extension gets it → {path, bytes}",
            has_doc,
            export
        ),
    ]
}

fn id_param(p: &Value, cmd: &str, key: &str) -> Result<u32> {
    p.get(key).and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).ok_or_else(|| bad(cmd, format!("`{key}`: a version id")))
}

fn new_version(author: &str, kind: VersionKind, label: impl Into<String>) -> NewVersion {
    NewVersion { author: author.to_string(), ts: designcraft_doc::vars::now(), label: label.into(), kind, ..Default::default() }
}

fn kept(h: &mut History, doc: &Document, v: NewVersion) -> Result<u32> {
    h.commit(doc, v).map_err(|e| EngineError::Other(format!("could not keep a version of the document: {e}")))
}

/// The document's versions once `saved` (the document as file.save writes it) is saved: with a
/// version of it, manual (`manual`: its label) or, when it changed since it was last saved, a save
/// (keeping the last [`SAVES_KEPT`] saves). The first such save of a file kept without versions
/// keeps what the file had first.
pub(super) fn kept_on_save(st: &DocState, saved: &Document, manual: Option<String>, author: &str) -> Result<History> {
    let mut h = st.versions.clone();
    match manual {
        Some(label) => {
            kept(&mut h, saved, new_version(author, VersionKind::Manual, label))?;
        }
        None if st.is_dirty() => {
            if h.is_empty() && st.path.is_some() {
                kept(&mut h, &st.saved_doc, new_version(author, VersionKind::Save, "As opened"))?;
            }
            kept(&mut h, saved, new_version(author, VersionKind::Save, ""))?;
            h.prune(SAVES_KEPT, true);
        }
        None => {}
    }
    Ok(h)
}

/// Around a Flow update: the document `before` it, unless that is the latest version already,
/// and after it, with the update's reports (`r["reports"]`). A version that can't be kept is a
/// warning in `r`; the update stands.
pub(super) fn keep_around_update(s: &mut Session, before: &Document, r: &mut Value) {
    let author = s.prefs.user_name.clone();
    let Ok(st) = s.doc_mut() else { return };
    let mut h = st.versions.clone();
    let after = st.doc.clone();
    let mut keep = || -> Result<()> {
        if !h.is_latest(before) {
            kept(&mut h, before, new_version(&author, VersionKind::Save, "Before Flow update"))?;
        }
        kept(&mut h, &after, NewVersion { reports: r.get("reports").cloned(), ..new_version(&author, VersionKind::LinkUpdate, "") })?;
        Ok(())
    };
    match keep() {
        Ok(()) => st.versions = h,
        Err(e) => {
            if let Some(w) = r.get_mut("warnings").and_then(Value::as_array_mut) {
                w.push(json!(e.to_string()));
            }
        }
    }
}

fn commit(s: &mut Session, p: &Value) -> Result<Value> {
    let label = str_param(p, "label").unwrap_or_default().trim().to_string();
    let mut r = super::file::save_document(s, p, Some(label))?;
    let id = s.doc()?.versions.versions().last().map(|v| v.id);
    if let (Some(o), Some(id)) = (r.as_object_mut(), id) {
        o.insert("id".into(), json!(id));
    }
    Ok(r)
}

/// Version `id` as a document to open in place of, or beside, `st`'s: its parent links checked
/// as opening its file would, with `st`'s document fonts.
fn version_doc(st: &DocState, id: u32, cmd: &str) -> Result<Document> {
    let mut d = st.versions.document(id).map_err(|e| bad(cmd, e.to_string()))?;
    let _ = d.fix_parent_bases();
    d.font_scope = st.doc.font_scope;
    Ok(d)
}

fn show(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "version.show", "id")?;
    let st = s.doc()?;
    let mut d = version_doc(st, id, "version.show")?;
    d.title = format!("{} (version {id})", if d.title.is_empty() { "Untitled" } else { &d.title });
    let mut shown = DocState::new(d, None);
    shown.fonts = st.fonts.clone();
    Ok(json!({"index": s.add_document(shown)}))
}

fn restore(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "version.restore", "id")?;
    let author = s.prefs.user_name.clone();
    let st = s.doc()?;
    let mut d = version_doc(st, id, "version.restore")?;
    // Ids handed out since stay used: an asset added later never takes the id of another
    // version's file.
    d.next_id = d.next_id.max(st.doc.next_id);
    let mut h = st.versions.clone();
    let restored = kept(&mut h, &d, NewVersion { parent: Some(id), ..new_version(&author, VersionKind::Restore, format!("Restored version {id}")) })?;
    s.edit(|doc, sel| {
        *doc = d;
        *sel = Selection::default();
        Ok(())
    })?;
    s.doc_mut()?.versions = h;
    Ok(json!({"id": restored, "restored": id}))
}

fn prune(s: &mut Session, p: &Value) -> Result<Value> {
    let keep_last = p.get("keepLast").and_then(Value::as_u64).map_or(SAVES_KEPT, |n| usize::try_from(n).unwrap_or(usize::MAX));
    let keep_manual = p.get("keepManual").and_then(Value::as_bool).unwrap_or(true);
    Ok(json!({"removed": s.doc_mut()?.versions.prune(keep_last, keep_manual)}))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "version.export", "id")?;
    let path = super::file::with_document_extension(str_param(p, "path").ok_or_else(|| bad("version.export", "missing `path`"))?);
    crate::file_access::check_write(&path).map_err(EngineError::Other)?;
    let bytes = super::to_bytes(&version_doc(s.doc()?, id, "version.export")?)?;
    #[cfg(not(target_arch = "wasm32"))]
    crate::io::write_file(&path, &bytes).map_err(EngineError::Other)?;
    Ok(json!({"path": path, "bytes": bytes.len()}))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use designcraft_doc::StoryId;
    use serde_json::{Value, json};

    use crate::Session;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dc-version-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A new document with one story saying `text`.
    fn start(text: &str) -> (Session, StoryId) {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 400, 500], "content": "text", "text": text})).unwrap();
        (s, StoryId(r["story"].as_u64().unwrap()))
    }

    fn write(s: &mut Session, sid: StoryId, text: &str) {
        s.edit(|d, _| {
            d.set_story_text(sid, text).unwrap();
            Ok(Value::Null)
        })
        .unwrap();
    }

    fn text(s: &Session, sid: StoryId) -> String {
        s.doc().unwrap().doc.story(sid).unwrap().text.clone()
    }

    fn list(s: &mut Session) -> Vec<Value> {
        s.execute("version.list", &json!({})).unwrap().as_array().unwrap().clone()
    }

    /// Version `id`'s text of story `sid`.
    fn version_text(s: &Session, id: u32, sid: StoryId) -> String {
        s.doc().unwrap().versions.document(id).unwrap().story(sid).unwrap().text.clone()
    }

    /// Saving a changed document keeps a version of it in the file. Restoring one makes the
    /// document that version again, undoably, and keeps a version saying so; the versions before
    /// it stay as they were.
    #[test]
    fn saves_keep_versions_and_a_restore_never_rewrites_history() {
        let dir = folder("restore");
        let path = dir.join("doc.designcraft");
        let (mut s, sid) = start("One.");
        s.execute("file.save", &json!({"path": path})).unwrap();
        write(&mut s, sid, "Two.");
        s.execute("file.save", &json!({})).unwrap();
        // Saved again without a change: no version.
        s.execute("file.save", &json!({})).unwrap();
        let v = list(&mut s);
        assert_eq!(v.iter().map(|v| v["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["save", "save"], "{v:?}");
        assert!(v.iter().all(|v| v["ts"].as_i64().unwrap() > 0));
        // Reopened, the file has them.
        s.execute("file.close", &json!({})).unwrap();
        s.execute("file.open", &json!({"path": path})).unwrap();
        assert_eq!(list(&mut s), v);
        let r = s.execute("version.restore", &json!({"id": 1})).unwrap();
        assert_eq!(text(&s, sid), "One.");
        assert!(s.doc().unwrap().is_dirty());
        let after = list(&mut s);
        assert_eq!(after[..2], v[..], "the versions before are as they were");
        assert_eq!((after[2]["kind"].as_str(), after[2]["parent"].as_u64(), &after[2]["id"]), (Some("restore"), Some(1), &r["id"]));
        // Undone, the document is back, and the history keeps the restore.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(text(&s, sid), "Two.");
        assert_eq!(list(&mut s).len(), 3);
        // Saved and reopened, every version is still what it was.
        s.execute("file.save", &json!({})).unwrap();
        s.execute("file.close", &json!({})).unwrap();
        s.execute("file.open", &json!({"path": path})).unwrap();
        assert_eq!(list(&mut s).len(), 3);
        assert_eq!([1, 2, 3].map(|id| version_text(&s, id, sid)), ["One.", "Two.", "One."]);
        assert!(s.execute("version.restore", &json!({"id": 99})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file kept without history, opened and saved with a change: what it had is kept first.
    #[test]
    fn the_first_save_of_a_file_keeps_what_it_had() {
        let dir = folder("first");
        let path = dir.join("plain.designcraft");
        let (mut s, sid) = start("Original.");
        std::fs::write(&path, designcraft_format::save(&s.doc().unwrap().doc).unwrap()).unwrap();
        s.execute("file.open", &json!({"path": path})).unwrap();
        write(&mut s, sid, "Edited.");
        s.execute("file.save", &json!({})).unwrap();
        let v = list(&mut s);
        assert_eq!(v.iter().map(|v| v["label"].as_str().unwrap()).collect::<Vec<_>>(), ["As opened", ""], "{v:?}");
        assert_eq!([1, 2].map(|id| version_text(&s, id, sid)), ["Original.", "Edited."]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Saving keeps the last 20 saves. Pruning drops the oldest saves past the newest
    /// `keepLast`, keeps manual versions, and leaves every version it keeps as it was.
    #[test]
    fn prune_keeps_manual_versions() {
        let dir = folder("prune");
        let path = dir.join("doc.designcraft");
        let (mut s, sid) = start("v0");
        s.execute("file.save", &json!({"path": path})).unwrap();
        for n in 1..=24 {
            write(&mut s, sid, &format!("v{n}"));
            if n == 5 {
                // File › Save a Version: kept as a manual version, and saved.
                s.execute("version.commit", &json!({"label": "Milestone"})).unwrap();
                assert!(!s.doc().unwrap().is_dirty());
            } else {
                s.execute("file.save", &json!({})).unwrap();
            }
        }
        let v = list(&mut s);
        assert_eq!(v.iter().filter(|v| v["kind"] == "save").count(), 20, "{v:?}");
        let manual: Vec<&Value> = v.iter().filter(|v| v["kind"] == "manual").collect();
        assert_eq!(manual.iter().map(|v| v["label"].as_str().unwrap()).collect::<Vec<_>>(), ["Milestone"]);
        let ids = |v: &[Value]| v.iter().map(|v| v["id"].as_u64().unwrap() as u32).collect::<Vec<_>>();
        let texts = |s: &Session, ids: &[u32]| ids.iter().map(|id| version_text(s, *id, sid)).collect::<Vec<_>>();
        let r = s.execute("version.prune", &json!({"keepLast": 2})).unwrap();
        assert_eq!(r["removed"], 18, "{r}");
        let kept = ids(&list(&mut s));
        assert_eq!(texts(&s, &kept), ["v5", "v23", "v24"]);
        // In the file too.
        s.execute("file.save", &json!({})).unwrap();
        s.execute("file.close", &json!({})).unwrap();
        s.execute("file.open", &json!({"path": path})).unwrap();
        assert_eq!(ids(&list(&mut s)), kept);
        assert_eq!(texts(&s, &kept), ["v5", "v23", "v24"]);
        // keepManual: false prunes manual versions too.
        s.execute("version.prune", &json!({"keepLast": 1, "keepManual": false})).unwrap();
        let last = ids(&list(&mut s));
        assert_eq!(texts(&s, &last), ["v24"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A version shown is a new, unsaved document; exported, a file of it alone; compared, the
    /// redline of one version against another or against the document.
    #[test]
    fn versions_show_export_and_compare() {
        let dir = folder("show");
        let path = dir.join("doc.designcraft");
        let (mut s, sid) = start("Revenue grew. Costs were flat.");
        s.execute("file.save", &json!({"path": path})).unwrap();
        write(&mut s, sid, "Revenue grew. Costs fell.");
        s.execute("file.save", &json!({})).unwrap();
        write(&mut s, sid, "Revenue grew strongly. Costs fell.");
        let doc = s.active_index().unwrap();
        let r = s.execute("version.show", &json!({"id": 1})).unwrap();
        assert_eq!(text(&s, sid), "Revenue grew. Costs were flat.");
        assert!(s.doc().unwrap().path.is_none());
        assert!(s.doc().unwrap().doc.title.contains("version 1"), "{}", s.doc().unwrap().doc.title);
        s.execute("file.close", &json!({"index": r["index"]})).unwrap();
        s.execute("file.activate", &json!({"index": doc})).unwrap();
        let out = dir.join("v1");
        let r = s.execute("version.export", &json!({"id": 1, "path": out})).unwrap();
        let (exported, history) = designcraft_format::load_with_history(&std::fs::read(r["path"].as_str().unwrap()).unwrap()).unwrap();
        assert!(r["path"].as_str().unwrap().ends_with(".designcraft"));
        assert_eq!(exported.story(sid).unwrap().text, "Revenue grew. Costs were flat.");
        assert!(history.unwrap().is_empty(), "a version exported alone");
        // Version 1 against version 2, and against the document: accepting every change gives the
        // later one, rejecting every change the earlier.
        for (params, later) in [(json!({"a": 1, "b": 2}), "Revenue grew. Costs fell."), (json!({"a": 1}), "Revenue grew strongly. Costs fell.")] {
            for (resolve, want) in [("changes.acceptAll", later), ("changes.rejectAll", "Revenue grew. Costs were flat.")] {
                s.execute("file.activate", &json!({"index": doc})).unwrap();
                let r = s.execute("compare.versions", &params).unwrap();
                assert!(r["changes"].as_u64().unwrap() >= 2, "{r}");
                assert_eq!(s.active_index(), r["redline"].as_u64().map(|i| i as usize));
                s.execute(resolve, &json!({})).unwrap();
                assert_eq!(text(&s, sid), want, "{params} {resolve}");
                s.execute("file.close", &json!({})).unwrap();
            }
        }
        s.execute("file.activate", &json!({"index": doc})).unwrap();
        assert!(s.execute("compare.versions", &json!({"a": 7})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn versions_in(file: &[u8]) -> usize {
        designcraft_format::load_with_history(file).unwrap().1.unwrap().versions().count()
    }

    /// Save a Copy keeps the history unless told not to, as does the serialized file; Revert
    /// brings back the file's; a recovered document keeps its own.
    #[test]
    fn copies_revert_and_recovery_keep_the_history() {
        let dir = folder("copies");
        let path = dir.join("doc.designcraft");
        let (mut s, sid) = start("One.");
        s.execute("file.save", &json!({"path": path})).unwrap();
        write(&mut s, sid, "Two.");
        s.execute("file.save", &json!({})).unwrap();
        let (with, without) = (dir.join("with.designcraft"), dir.join("without.designcraft"));
        s.execute("file.saveACopy", &json!({"path": with})).unwrap();
        s.execute("file.saveACopy", &json!({"path": without, "history": false})).unwrap();
        assert_eq!((versions_in(&std::fs::read(&with).unwrap()), versions_in(&std::fs::read(&without).unwrap())), (2, 0));
        let b64 = s.execute("file.serialize", &json!({})).unwrap()["base64"].as_str().unwrap().to_string();
        assert_eq!(versions_in(&crate::cmd::base64_decode(&b64)), 2);
        // Reverted after a restore: the file's history again.
        s.execute("version.restore", &json!({"id": 1})).unwrap();
        assert_eq!(list(&mut s).len(), 3);
        s.execute("file.revert", &json!({})).unwrap();
        assert_eq!(list(&mut s).len(), 2);
        assert_eq!(text(&s, sid), "Two.");
        // Recovered with its history.
        write(&mut s, sid, "Three.");
        let rec = dir.join("recovery");
        s.execute("file.recovery.save", &json!({"dir": rec})).unwrap();
        let mut t = Session::new();
        t.execute("file.recovery.open", &json!({"dir": rec})).unwrap();
        assert_eq!(text(&t, sid), "Three.");
        assert_eq!(list(&mut t).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file whose history can't be read opens, with a warning saying saving leaves it out.
    #[test]
    fn a_file_whose_history_cannot_be_read_opens_with_a_warning() {
        let dir = folder("broken");
        let path = dir.join("doc.designcraft");
        let (mut s, sid) = start("One.");
        s.execute("file.save", &json!({"path": path})).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(std::fs::read(&path).unwrap())).unwrap();
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            out.start_file(f.name(), zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(if f.name() == "history/versions.json" { b"{" } else { &data }).unwrap();
        }
        std::fs::write(&path, out.finish().unwrap().into_inner()).unwrap();
        let r = s.execute("file.open", &json!({"path": path})).unwrap();
        assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("history")), "{r}");
        assert!(list(&mut s).is_empty());
        assert_eq!(text(&s, sid), "One.");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
