//! File › Review: the review bundle a reviewer's browser reads (`designcraft-review`), and the
//! document's review identity.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc, str_param};
use crate::{Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "review.export", "Export for Review…", ["File", "Review"], None,
            "{dir, scale?: 2 (pixels per point, 0.5–8)} — a review bundle in `dir`: manifest.json, pages/NNN.webp, text.json and document.pdf (docs/review-bundle.md). Gives the document its review id the first time, and keeps a version `Review vN` in its history, written by the next Save (the document's file itself isn't touched) → {dir, documentId, versionId?, pages, warnings}",
            has_doc, export),
        cmd!(noundo "review.detach", "Detach from Review", ["File", "Review"], None,
            "{} — clears the document's review id, so a file reused for another client starts a review of its own → {detached}",
            has_doc, detach),
    ]
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.export";
    let dir = std::path::PathBuf::from(str_param(p, "dir").ok_or_else(|| bad(ID, "missing `dir`"))?);
    let scale = match p.get("scale") {
        None | Some(Value::Null) => 2.0,
        Some(v) => v.as_f64().filter(|s| (0.5..=8.0).contains(s)).ok_or_else(|| bad(ID, "`scale`: 0.5 to 8 pixels per point"))?,
    };
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, dir, scale);
        Err(bad(ID, "exporting a review bundle needs a file system"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    export_to(s, dir, scale)
}

#[cfg(not(target_arch = "wasm32"))]
fn export_to(s: &mut Session, dir: std::path::PathBuf, scale: f64) -> Result<Value> {
    use crate::EngineError;
    let root = super::package::OutputRoot::new(dir.clone())?;
    let mut warnings = Vec::new();
    // The document's review identity, the first time.
    let document_id = match s.doc()?.doc.id.clone() {
        Some(id) => id,
        None => {
            let id = designcraft_doc::Document::fresh_id();
            let st = s.doc_mut()?;
            Arc::make_mut(&mut st.doc).id = Some(id.clone());
            st.bump_revision();
            id
        }
    };
    let version_id = keep_version(s, &mut warnings)?;
    let doc = s.doc()?.doc.clone();
    let cache = s.cache.clone();
    let pages = designcraft_review::page_infos(&doc);
    let json = |v: &Value, pretty: bool| {
        if pretty { serde_json::to_vec_pretty(v) } else { serde_json::to_vec(v) }.map_err(|e| EngineError::Other(e.to_string()))
    };
    let mut r = designcraft_render::Renderer::new();
    let opts = designcraft_render::RenderOptions { printing_only: true, ..Default::default() };
    for page in &pages {
        match r.render_page(&doc, &cache, page.index, scale, false, &opts) {
            Some(img) => root.write(&dir.join(&page.image), &img.to_webp())?,
            None => warnings.push(format!("page {} couldn't be rendered at {scale} pixels per point", page.label)),
        }
    }
    let (text, mut layer_warnings) = designcraft_review::text_layer(&doc, &cache);
    warnings.append(&mut layer_warnings);
    root.write(&dir.join("text.json"), &json(&serde_json::to_value(&text).map_err(|e| EngineError::Other(e.to_string()))?, false)?)?;
    let pdf = s.execute("file.exportPdf", &json!({"path": dir.join("document.pdf").to_string_lossy()}))?;
    warnings.extend(pdf["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
    let manifest = json!({
        "format": "designcraft-review", "version": 1, "generator": concat!("DesignCraft ", env!("CARGO_PKG_VERSION")),
        "documentId": document_id, "versionId": version_id, "title": doc.title, "scale": scale,
        "pages": pages, "pdf": "document.pdf", "text": "text.json", "warnings": warnings,
    });
    root.write(&dir.join("manifest.json"), &json(&manifest, true)?)?;
    Ok(json!({"dir": dir, "documentId": document_id, "versionId": version_id, "pages": pages.len(), "warnings": warnings}))
}

/// Keeps a `Review vN` version of the document in its history, for the next Save to write (as a
/// Flow update does), or says in `warnings` why none was kept.
#[cfg(not(target_arch = "wasm32"))]
fn keep_version(s: &mut Session, warnings: &mut Vec<String>) -> Result<Option<u32>> {
    use designcraft_format::{NewVersion, VersionKind};
    if !super::version::keeps(s.prefs.keep_versions, s.doc()?) {
        warnings.push(format!("{}: no version of what reviewers see was kept", super::version::VERSIONS_OFF));
        return Ok(None);
    }
    let author = s.prefs.user_name.clone();
    let st = s.doc_mut()?;
    let n = st.versions.versions().filter(|v| v.label.starts_with("Review v")).count().saturating_add(1);
    let v = NewVersion { author, ts: designcraft_doc::vars::now(), label: format!("Review v{n}"), kind: VersionKind::Manual, ..Default::default() };
    let doc = st.doc.clone();
    let id = st.versions.commit(&doc, v).map_err(|e| crate::EngineError::Other(format!("could not keep a version of the document: {e}")))?;
    st.bump_revision();
    Ok(Some(id))
}

fn detach(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc_mut()?;
    let had = st.doc.id.is_some();
    if had {
        Arc::make_mut(&mut st.doc).id = None;
        st.bump_revision();
    }
    Ok(json!({"detached": had}))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use designcraft_doc::StoryId;
    use serde_json::{Value, json};

    use crate::Session;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dc-review-{name}-{}", std::process::id()));
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

    fn export(s: &mut Session, dir: &Path) -> Value {
        s.execute("review.export", &json!({"dir": dir.to_string_lossy()})).unwrap()
    }

    fn read_json(p: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
    }

    fn id_of(s: &Session) -> Option<String> {
        s.doc().unwrap().doc.id.clone()
    }

    fn file_id(path: &Path) -> Option<String> {
        designcraft_format::load(&std::fs::read(path).unwrap()).unwrap().id
    }

    /// The labels of the open document's versions, oldest first.
    fn labels(s: &mut Session) -> Vec<String> {
        s.execute("version.list", &json!({})).unwrap().as_array().unwrap().iter().map(|v| v["label"].as_str().unwrap().to_string()).collect()
    }

    fn last_version_id(s: &mut Session) -> Option<u64> {
        s.execute("version.list", &json!({})).unwrap().as_array().unwrap().last().and_then(|v| v["id"].as_u64())
    }

    fn rect_of(line: &Value) -> Vec<f64> {
        line["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
    }

    #[test]
    fn export_writes_pages_the_text_layer_the_pdf_and_a_manifest() {
        let (mut s, sid) = start("Hello review world");
        let dir = folder("bundle");
        let r = export(&mut s, &dir);
        assert_eq!(r["pages"], 1, "{r}");
        let m = read_json(&dir.join("manifest.json"));
        assert_eq!(m["documentId"], r["documentId"]);
        assert_eq!(m["documentId"].as_str().unwrap().len(), 36);
        assert_eq!(m["pages"][0]["image"], "pages/001.webp");
        assert_eq!((m["pages"][0]["width"].as_f64().unwrap(), m["pages"][0]["height"].as_f64().unwrap()), (612.0, 792.0));
        assert_eq!((m["pdf"].as_str(), m["text"].as_str()), (Some("document.pdf"), Some("text.json")));
        let webp = std::fs::read(dir.join("pages/001.webp")).unwrap();
        assert!(webp.starts_with(b"RIFF") && &webp[8..12] == b"WEBP", "not a WebP");
        assert!(std::fs::read(dir.join("document.pdf")).unwrap().starts_with(b"%PDF"));
        let t = read_json(&dir.join("text.json"));
        let line = &t["pages"][0]["lines"][0];
        assert_eq!(line["text"], "Hello review world");
        assert_eq!(line["story"], sid.0);
        let rect = rect_of(line);
        assert!(rect[0] >= 72.0 && rect[1] >= 72.0 && rect[2] <= 400.0 && rect[3] <= 500.0, "{rect:?}");
    }

    #[test]
    fn export_gives_the_document_an_id_that_save_as_keeps_and_a_copy_drops() {
        let (mut s, _) = start("id");
        assert_eq!(id_of(&s), None);
        let dir = folder("ids");
        export(&mut s, &dir.join("bundle"));
        let id = id_of(&s).expect("an id after export");
        let (a, b, c) = (dir.join("a.designcraft"), dir.join("b.designcraft"), dir.join("copy.designcraft"));
        s.execute("file.save", &json!({"path": a.to_string_lossy()})).unwrap();
        s.execute("file.saveAs", &json!({"path": b.to_string_lossy()})).unwrap();
        s.execute("file.saveACopy", &json!({"path": c.to_string_lossy()})).unwrap();
        assert_eq!((file_id(&a), file_id(&b), file_id(&c)), (Some(id.clone()), Some(id.clone()), None));
        assert_eq!(id_of(&s), Some(id.clone()), "the open document keeps its id after Save a Copy");
        s.execute("review.detach", &json!({})).unwrap();
        assert_eq!(id_of(&s), None);
        export(&mut s, &dir.join("bundle2"));
        assert_ne!(id_of(&s), Some(id), "a detached document starts over");
    }

    #[test]
    fn export_keeps_a_review_version_that_the_next_save_writes_and_never_saves_itself() {
        let (mut s, _) = start("versions");
        let dir = folder("versions");
        let path = dir.join("doc.designcraft");
        s.execute("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
        let before = std::fs::read(&path).unwrap();
        let r = export(&mut s, &dir.join("b1"));
        assert_eq!(labels(&mut s).last().map(String::as_str), Some("Review v1"));
        assert_eq!(r["versionId"].as_u64(), last_version_id(&mut s));
        assert_eq!(std::fs::read(&path).unwrap(), before, "export doesn't write the document file");
        export(&mut s, &dir.join("b2"));
        let l = labels(&mut s);
        assert_eq!(&l[l.len() - 2..], ["Review v1", "Review v2"]);
        assert!(s.doc().unwrap().is_dirty(), "a kept version is something to save");
        s.execute("file.save", &json!({})).unwrap();
        let (_, h) = designcraft_format::load_with_history(&std::fs::read(&path).unwrap()).unwrap();
        let saved: Vec<String> = h.unwrap().versions().map(|v| v.label.clone()).collect();
        assert!(saved.contains(&"Review v1".to_string()) && saved.contains(&"Review v2".to_string()), "{saved:?}");
    }

    #[test]
    fn an_unsaved_document_exports_and_keeps_its_version_in_memory() {
        let (mut s, _) = start("never saved");
        let r = export(&mut s, &folder("unsaved"));
        assert_eq!(labels(&mut s), vec!["Review v1"]);
        assert_eq!(r["versionId"].as_u64(), last_version_id(&mut s));
    }

    #[test]
    fn with_versions_off_export_warns_and_keeps_none() {
        let (mut s, _) = start("off");
        s.prefs.keep_versions = false;
        let r = export(&mut s, &folder("off"));
        assert!(labels(&mut s).is_empty());
        assert!(r["versionId"].is_null());
        assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("Keep Earlier Versions")), "{r}");
    }

    #[test]
    fn export_refuses_a_missing_dir_and_a_scale_out_of_range() {
        let (mut s, _) = start("x");
        assert!(s.execute("review.export", &json!({})).is_err());
        assert!(s.execute("review.export", &json!({"dir": folder("scale").to_string_lossy(), "scale": 50})).is_err());
    }

    /// The sample magazine has a table and a footnote: their text is in the layer, addressed as
    /// `text.select` addresses it, and every line's centre lies on its page.
    #[test]
    fn the_sample_s_cells_and_footnotes_are_in_the_text_layer_on_their_pages() {
        let mut s = Session::new();
        s.execute("file.newSample", &json!({})).unwrap();
        let dir = folder("sample");
        let r = s.execute("review.export", &json!({"dir": dir.to_string_lossy(), "scale": 0.5})).unwrap();
        let m = read_json(&dir.join("manifest.json"));
        let t = read_json(&dir.join("text.json"));
        let pages = t["pages"].as_array().unwrap();
        assert_eq!(pages.len(), m["pages"].as_array().unwrap().len());
        assert_eq!(r["pages"].as_u64().unwrap() as usize, pages.len());
        let (mut cells, mut notes, mut lines) = (0, 0, 0);
        for (i, p) in pages.iter().enumerate() {
            let (w, h) = (m["pages"][i]["width"].as_f64().unwrap(), m["pages"][i]["height"].as_f64().unwrap());
            assert!(dir.join(m["pages"][i]["image"].as_str().unwrap()).is_file());
            for l in p["lines"].as_array().unwrap() {
                lines += 1;
                cells += usize::from(l.get("cell").is_some());
                notes += usize::from(l.get("footnote").is_some());
                let r = rect_of(l);
                let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
                assert!((0.0..=w).contains(&cx) && (0.0..=h).contains(&cy), "page {i}: {r:?} off the {w}×{h} page");
                assert_eq!(l["bytes"].as_array().unwrap().len(), l["text"].as_str().unwrap().chars().count());
            }
        }
        assert!(lines > 20 && cells > 0 && notes > 0, "lines {lines}, cells {cells}, notes {notes}");
    }
}
