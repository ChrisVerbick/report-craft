//! File › Review: the review bundle a reviewer's browser reads (`designcraft-review`), the
//! document's review identity, and the round trip with the review service: publish a version,
//! sync the comments, take them into the layout as notes and tracked changes.

use std::sync::Arc;

use designcraft_doc::{Document, StoryId};
use designcraft_review::{Anchored, Comment};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc, str_param};
use crate::review::{Client, ReviewState, Transport};
use crate::{EngineError, Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "review.export", "Export for Review…", ["File", "Review"], None,
            "{dir, scale?: 2 (pixels per point, 0.5–8)} — a review bundle in `dir`: manifest.json, pages/NNN.webp, text.json and document.pdf (docs/review-bundle.md). Gives the document its review id the first time, and keeps a version `Review vN` in its history, written by the next Save (the document's file itself isn't touched) → {dir, documentId, versionId?, pages, warnings}",
            has_doc, export),
        cmd!(noundo "review.detach", "Detach from Review", ["File", "Review"], None,
            "{} — clears the document's review id, so a file reused for another client starts a review of its own → {detached}",
            has_doc, detach),
        cmd!(query "review.projects", "Review Projects", [], None,
            "{server?} — the review service's projects → [{id, name, client}]; the service is Preferences' reviewServer and reviewToken (prefs.set)",
            super::always, projects),
        cmd!(noundo "review.createProject", "New Review Project", [], None,
            "{name, client?, server?} → {id, name, client}", super::always, create_project),
        cmd!(noundo "review.publish", "Publish for Review…", ["File", "Review"], None,
            "{project?, scale?, server?} — exports the review bundle and uploads it as a new version of this document; `project` the first time (a document stays bound to the project it was first published to; the service refuses another and names it) → {project, document, version: {id, n, label}, pages, warnings}",
            has_doc, publish),
        cmd!(noundo "review.sync", "Sync Review Comments", ["File", "Review"], None,
            "{server?} — fetches the comments of this document's review; text that is gone from the document is reported to the service as orphaned → {project, document, comments, open, orphaned}",
            has_doc, sync),
        cmd!(query "review.comments", "Review Comments", [], None,
            "{} — the comments last synced, each with `anchored`: where its text is now ({status: found|moved, start, end} or {status: orphaned}; null for pins and notes) → [comment]",
            has_doc, comments),
        cmd!(noundo "review.goTo", "Go to Review Comment", [], None,
            "{id} — selects the comment's text where it is now → {page, anchored}", has_doc, go_to),
        cmd!(
            "review.addNote",
            "Add Review Comment as Note",
            [],
            None,
            "{id} — the comment as an editorial note at its text (at the story's start when the text is gone), by `<reviewer> (Review)`; the service marks it applied → {story, id, at}",
            has_doc,
            add_note
        ),
        cmd!(
            "review.apply",
            "Apply Review Suggestion",
            [],
            None,
            "{id} — a replace or strike suggestion as a tracked change at its text (Track Changes accepts or rejects it); refused when the text is gone; the service marks it applied → {status, story, start, end}",
            has_doc,
            apply
        ),
        cmd!(noundo "review.resolve", "Resolve Review Comment", [], None,
            "{id, on?: true} — resolve (or reopen) a comment on the service → {status}", has_doc, resolve),
        cmd!(noundo "review.reply", "Reply to Review Comment", [], None,
            "{id, body} — a reply on the service, signed with Preferences' userName → the reply", has_doc, reply),
        cmd!(noundo "review.link", "Review Share Link…", ["File", "Review"], None,
            "{project?, mode?: invite|public, password?, expiresDays?, server?} — a share link for the document's project (shown once: the service keeps only its hash) → {id, token, url, mode, expires}",
            has_doc, link),
    ]
}

fn scale_param(p: &Value, id: &str) -> Result<f64> {
    match p.get("scale") {
        None | Some(Value::Null) => Ok(2.0),
        Some(v) => v.as_f64().filter(|s| (0.5..=8.0).contains(s)).ok_or_else(|| bad(id, "`scale`: 0.5 to 8 pixels per point")),
    }
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.export";
    let dir = std::path::PathBuf::from(str_param(p, "dir").ok_or_else(|| bad(ID, "missing `dir`"))?);
    let scale = scale_param(p, ID)?;
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
    let root = super::package::OutputRoot::new(dir.clone())?;
    let mut warnings = Vec::new();
    // The document's review identity, the first time.
    let document_id = match s.doc()?.doc.id.clone() {
        Some(id) => id,
        None => {
            let id = Document::fresh_id();
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
    let id = st.versions.commit(&doc, v).map_err(|e| EngineError::Other(format!("could not keep a version of the document: {e}")))?;
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
    st.review = None;
    Ok(json!({"detached": had}))
}

// ---- the round trip with the service ----

const SETTINGS_HELP: &str = "set the review service first: prefs.set {reviewServer: \"https://…\", reviewToken: \"…\"} (Preferences › Review)";

/// The service as reached from this session: the app's transport, the server from the params or
/// the preferences, the owner token from the preferences.
struct Service {
    server: String,
    token: String,
    transport: Arc<dyn Transport>,
}

impl Service {
    fn client(&self) -> Client<'_> {
        Client { server: &self.server, token: &self.token, transport: self.transport.as_ref() }
    }
}

fn service(s: &Session, p: &Value, id: &str) -> Result<Service> {
    let transport = s.transport.clone().ok_or_else(|| bad(id, "this build has no connection to a review service"))?;
    // A parameter, else the preference, else what the host set (its edition's own service).
    let server = str_param(p, "server")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| Some(s.prefs.review_server.trim().to_string()).filter(|v| !v.is_empty()))
        .or_else(|| s.review_server_default.clone())
        .unwrap_or_default();
    let token = s.prefs.review_token.clone();
    if server.is_empty() || token.is_empty() {
        return Err(bad(id, SETTINGS_HELP));
    }
    Ok(Service { server, token, transport })
}

fn projects(s: &mut Session, p: &Value) -> Result<Value> {
    service(s, p, "review.projects")?.client().get("/api/projects")
}

fn create_project(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.createProject";
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(ID, "missing `name`"))?;
    service(s, p, ID)?.client().post("/api/projects", &json!({"name": name, "client": str_param(p, "client").unwrap_or("")}))
}

/// Where the document is published, as the service has it: `{project, document, versions}`.
fn binding(svc: &Service, s: &Session) -> Result<Option<Value>> {
    match s.doc()?.doc.id.clone() {
        Some(id) => svc.client().get_opt(&format!("/api/documents/{id}")),
        None => Ok(None),
    }
}

fn publish(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.publish";
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, p);
        return Err(bad(ID, "publishing for review needs a file system"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let svc = service(s, p, ID)?;
        let scale = scale_param(p, ID)?;
        let bound = binding(&svc, s)?;
        let project_id = match (str_param(p, "project").map(str::trim).filter(|v| !v.is_empty()), &bound) {
            (Some(given), _) => given.to_string(),
            (None, Some(b)) => b["project"]["id"].as_str().unwrap_or_default().to_string(),
            (None, None) => {
                return Err(bad(
                    ID,
                    "not published yet: give the `project` to publish to (review.projects lists them; review.createProject makes one)",
                ));
            }
        };
        let dir = tempfile::tempdir().map_err(|e| EngineError::Other(format!("temporary folder: {e}")))?;
        let exported = export_to(s, dir.path().to_path_buf(), scale)?;
        let zip = zip_bundle(dir.path())?;
        let r = svc.client().post_zip(&format!("/api/projects/{project_id}/versions"), zip)?;
        let b = binding(&svc, s)?.unwrap_or(Value::Null);
        let project = b["project"].clone();
        let document = r["document"]["id"].as_str().unwrap_or_default().to_string();
        let st = s.doc_mut()?;
        let comments = st.review.take().filter(|r| r.document == document).map(|r| r.comments).unwrap_or_default();
        st.review = Some(ReviewState {
            server: svc.server.clone(),
            project: project.clone(),
            document: document.clone(),
            version: Some(r["version"].clone()),
            comments,
            synced: 0,
        });
        Ok(
            json!({"project": project, "document": r["document"], "version": r["version"], "pages": exported["pages"], "warnings": exported["warnings"]}),
        )
    }
}

/// The exported bundle folder as one zip: the manifest, the text layer, the PDF and the pages
/// it names. Reads go through the checked reader, like every file a command reads.
#[cfg(not(target_arch = "wasm32"))]
fn zip_bundle(dir: &std::path::Path) -> Result<Vec<u8>> {
    use std::io::Write;
    let io = |e: std::io::Error| EngineError::Other(format!("review bundle: {e}"));
    let zipped = |e: zip::result::ZipError| EngineError::Other(format!("review bundle: {e}"));
    let manifest_bytes = crate::io::read(dir.join("manifest.json")).map_err(io)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|e| EngineError::Other(format!("review bundle: manifest.json: {e}")))?;
    let mut names = vec!["text.json".to_string(), manifest["pdf"].as_str().unwrap_or("document.pdf").to_string()];
    names.extend(manifest["pages"].as_array().into_iter().flatten().filter_map(|p| p["image"].as_str()).map(str::to_string));
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = |name: &str| {
        // Images and the PDF are compressed already.
        let method = if name.ends_with(".json") { zip::CompressionMethod::Deflated } else { zip::CompressionMethod::Stored };
        zip::write::SimpleFileOptions::default().compression_method(method)
    };
    z.start_file("manifest.json", options("manifest.json")).map_err(zipped)?;
    z.write_all(&manifest_bytes).map_err(io)?;
    for name in names {
        let bytes = crate::io::read(dir.join(&name)).map_err(io)?;
        z.start_file(&name, options(&name)).map_err(zipped)?;
        z.write_all(&bytes).map_err(io)?;
    }
    Ok(z.finish().map_err(zipped)?.into_inner())
}

/// Where a comment's text is in the document now; `None` for comments without text (pins,
/// notes, replies).
fn anchored_in(doc: &Document, c: &Comment) -> Option<Anchored> {
    let sid = StoryId(c.story?);
    match doc.text_story(sid, c.address()) {
        Some(story) => c.anchored_in(&story.text),
        None if c.start.is_some() => Some(Anchored::Orphaned),
        None => None,
    }
}

fn sync(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.sync";
    let svc = service(s, p, ID)?;
    let b = binding(&svc, s)?.ok_or_else(|| bad(ID, "not published yet (review.publish)"))?;
    let (project, document) = (b["project"].clone(), b["document"]["id"].as_str().unwrap_or_default().to_string());
    let project_id = project["id"].as_str().unwrap_or_default().to_string();
    let all = svc.client().get(&format!("/api/projects/{project_id}/comments.json"))?;
    let mut comments: Vec<Comment> = all["comments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["document"] == document.as_str())
        .filter_map(|c| serde_json::from_value(c.clone()).ok())
        .collect();
    // Text that is gone is reported back, once, so reviewers see it.
    let doc = s.doc()?.doc.clone();
    let mut orphaned = 0;
    for c in &mut comments {
        if c.status == "open" && anchored_in(&doc, c) == Some(Anchored::Orphaned) {
            svc.client().patch(&format!("/api/comments/{}", c.id), &json!({"status": "orphaned"}))?;
            c.status = "orphaned".into();
            orphaned += 1;
        }
    }
    let open = comments.iter().filter(|c| c.status == "open" && c.in_reply_to.is_none()).count();
    let n = comments.len();
    let st = s.doc_mut()?;
    st.review = Some(ReviewState {
        server: svc.server.clone(),
        project: project.clone(),
        document: document.clone(),
        version: b["versions"].as_array().and_then(|v| v.last()).cloned(),
        comments,
        synced: designcraft_doc::vars::now(),
    });
    Ok(json!({"project": project, "document": document, "comments": n, "open": open, "orphaned": orphaned}))
}

fn synced<'a>(s: &'a Session, id: &str) -> Result<&'a ReviewState> {
    s.doc()?.review.as_ref().ok_or_else(|| bad(id, "not synced yet (review.sync)"))
}

fn comment_of<'a>(st: &'a ReviewState, p: &Value, id: &str) -> Result<&'a Comment> {
    let wanted = str_param(p, "id").ok_or_else(|| bad(id, "missing `id`"))?;
    st.comments.iter().find(|c| c.id == wanted).ok_or_else(|| bad(id, format!("no comment {wanted} (review.sync lists them)")))
}

fn comments(s: &mut Session, _: &Value) -> Result<Value> {
    let st = synced(s, "review.comments")?;
    let doc = &s.doc()?.doc;
    Ok(Value::Array(
        st.comments
            .iter()
            .map(|c| {
                let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
                v["anchored"] = anchored_in(doc, c).and_then(|a| serde_json::to_value(a).ok()).unwrap_or(Value::Null);
                v
            })
            .collect(),
    ))
}

/// `text.select` parameters for a comment's text at `range`.
fn selection(c: &Comment, range: (usize, usize)) -> Value {
    let mut v = json!({"story": c.story, "anchor": range.0, "focus": range.1});
    if let Some(n) = c.footnote {
        v["footnote"] = json!(n);
    } else if let Some(cell) = c.cell {
        v["cell"] = json!(cell);
    }
    v
}

fn go_to(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.goTo";
    let (c, page) = {
        let c = comment_of(synced(s, ID)?, p, ID)?;
        (c.clone(), c.page)
    };
    let anchored = anchored_in(&s.doc()?.doc, &c);
    if let Some(range) = anchored.as_ref().and_then(Anchored::range) {
        s.execute("text.select", &selection(&c, range))?;
    }
    Ok(json!({"page": page, "anchored": anchored}))
}

fn mark(s: &mut Session, p: &Value, svc: &Service, id: &str, status: &str) -> Result<()> {
    svc.client().patch(&format!("/api/comments/{id}"), &json!({"status": status}))?;
    if let Some(c) = s.doc_mut()?.review.as_mut().and_then(|r| r.comments.iter_mut().find(|c| c.id == id)) {
        c.status = status.into();
    }
    let _ = p;
    Ok(())
}

fn add_note(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.addNote";
    let svc = service(s, p, ID)?;
    let c = comment_of(synced(s, ID)?, p, ID)?.clone();
    let Some(story) = c.story else { return Err(bad(ID, "a pin has no text to attach a note to")) };
    let anchored = anchored_in(&s.doc()?.doc, &c);
    // At the end of the comment's text, in story text (notes don't go in cells or footnotes);
    // at the story's start when the text is gone.
    let at = match (&anchored, c.address()) {
        (Some(a), None) => a.range().map_or(0, |r| r.1),
        _ => 0,
    };
    let mut text = String::new();
    if anchored == Some(Anchored::Orphaned) {
        text.push_str("(text not found) ");
    }
    if let Some(q) = c.quote.exact.as_deref().filter(|q| !q.is_empty()) {
        text.push_str(&format!("“{q}”"));
        if !c.body.is_empty() || c.replacement.is_some() {
            text.push_str(": ");
        }
    }
    text.push_str(&c.body);
    if let Some(r) = &c.replacement {
        text.push_str(&format!(" → {r}"));
    }
    s.execute("text.select", &json!({"story": story, "anchor": at, "focus": at}))?;
    let author = format!("{} (Review)", if c.author.name.is_empty() { "Reviewer" } else { &c.author.name });
    let note = s.execute("note.new", &json!({"text": text.trim(), "author": author}))?;
    mark(s, p, &svc, &c.id, "applied")?;
    Ok(json!({"story": story, "id": note["id"], "at": at}))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.apply";
    let svc = service(s, p, ID)?;
    let c = comment_of(synced(s, ID)?, p, ID)?.clone();
    if c.kind != "replace" && c.kind != "strike" {
        return Err(bad(ID, "only replace and strike suggestions are applied; add the others as notes (review.addNote)"));
    }
    if c.status == "applied" || c.status == "orphaned" {
        return Err(bad(ID, format!("the suggestion is {}", c.status)));
    }
    let range = anchored_in(&s.doc()?.doc, &c)
        .and_then(|a| a.range())
        .ok_or_else(|| bad(ID, "the suggestion's text was not found in the document (orphaned)"))?;
    s.execute("changes.track", &json!({"on": true}))?;
    s.execute("text.select", &selection(&c, range))?;
    if c.kind == "replace" {
        s.execute("text.insert", &json!({"text": c.replacement.clone().unwrap_or_default(), "raw": true}))?;
    } else {
        s.execute("text.delete", &json!({}))?;
    }
    mark(s, p, &svc, &c.id, "applied")?;
    Ok(json!({"status": "applied", "story": c.story, "start": range.0, "end": range.1}))
}

fn resolve(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.resolve";
    let svc = service(s, p, ID)?;
    let id = comment_of(synced(s, ID)?, p, ID)?.id.clone();
    let status = if p.get("on").and_then(Value::as_bool).unwrap_or(true) { "resolved" } else { "open" };
    mark(s, p, &svc, &id, status)?;
    Ok(json!({"status": status}))
}

fn reply(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.reply";
    let svc = service(s, p, ID)?;
    let id = comment_of(synced(s, ID)?, p, ID)?.id.clone();
    let body = super::text_param(p, "body");
    if body.trim().is_empty() {
        return Err(bad(ID, "missing `body`"));
    }
    let name = if s.prefs.user_name.trim().is_empty() { None } else { Some(s.prefs.user_name.trim().to_string()) };
    let r = svc.client().post(&format!("/api/comments/{id}/replies"), &json!({"body": body, "name": name}))?;
    if let (Some(st), Ok(c)) = (s.doc_mut()?.review.as_mut(), serde_json::from_value::<Comment>(r.clone())) {
        st.comments.push(c);
    }
    Ok(r)
}

fn link(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "review.link";
    let svc = service(s, p, ID)?;
    let project = match str_param(p, "project").map(str::trim).filter(|v| !v.is_empty()) {
        Some(given) => given.to_string(),
        None => binding(&svc, s)?
            .and_then(|b| b["project"]["id"].as_str().map(str::to_string))
            .ok_or_else(|| bad(ID, "not published yet: give the `project` (review.projects)"))?,
    };
    let mode = str_param(p, "mode").unwrap_or("invite");
    if mode != "invite" && mode != "public" {
        return Err(bad(ID, "`mode`: invite or public"));
    }
    svc.client().post(
        &format!("/api/projects/{project}/share-links"),
        &json!({"mode": mode, "password": p.get("password"), "expiresDays": p.get("expiresDays")}),
    )
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

    use std::sync::Arc;

    use crate::review::FakeService;

    /// A document with one story, and a fake review service with one project, reachable.
    fn with_service(text: &str) -> (Session, StoryId, Arc<FakeService>) {
        let (mut s, sid) = start(text);
        let fake = FakeService::with_project("Acme annual report");
        s.transport = Some(fake.clone());
        s.execute("prefs.set", &json!({"reviewServer": "https://review.test", "reviewToken": "t"})).unwrap();
        (s, sid, fake)
    }

    fn publish(s: &mut Session) -> Value {
        let p = s.execute("review.projects", &json!({})).unwrap()[0]["id"].as_str().unwrap().to_string();
        s.execute("review.publish", &json!({"project": p})).unwrap()
    }

    fn write(s: &mut Session, sid: StoryId, text: &str) {
        s.edit(|d, _| {
            d.set_story_text(sid, text).unwrap();
            Ok(Value::Null)
        })
        .unwrap();
    }

    fn story_text(s: &Session, sid: StoryId) -> String {
        s.doc().unwrap().doc.story(sid).unwrap().text.clone()
    }

    #[test]
    fn publish_uploads_the_bundle_to_a_project_and_the_document_stays_bound_to_it() {
        let (mut s, _, fake) = with_service("Hello review world");
        let e = s.execute("review.publish", &json!({})).unwrap_err().to_string();
        assert!(e.contains("project"), "no project yet: {e}");
        let r = publish(&mut s);
        assert_eq!((r["version"]["n"].as_i64(), r["project"]["name"].as_str()), (Some(1), Some("Acme annual report")));
        assert_eq!(fake.uploads(), 1);
        let id = id_of(&s).unwrap();
        assert_eq!(fake.binding(&id).unwrap()["project"]["name"], "Acme annual report");
        // The second publish needs no project: the service knows the binding.
        assert_eq!(s.execute("review.publish", &json!({})).unwrap()["version"]["n"], 2);
        let other = s.execute("review.createProject", &json!({"name": "Other client"})).unwrap();
        let e = s.execute("review.publish", &json!({"project": other["id"]})).unwrap_err().to_string();
        assert!(e.contains("Acme annual report"), "the bound project is named: {e}");
        let st = s.doc().unwrap().review.as_ref().unwrap();
        assert_eq!((st.project["name"].as_str(), st.version.as_ref().map(|v| v["n"].as_i64())), (Some("Acme annual report"), Some(Some(2))));
    }

    #[test]
    fn sync_lists_comments_with_where_their_text_is_now() {
        let (mut s, sid, fake) = with_service("Hello review world");
        publish(&mut s);
        fake.add_comment(json!({"kind": "replace", "page": 0, "story": sid.0, "start": 0, "end": 5, "quote": {"exact": "Hello", "prefix": "", "suffix": " review world"}, "body": "Say hi", "replacement": "Hi", "author": {"name": "Ann"}}));
        fake.add_comment(json!({"kind": "pin", "page": 0, "x": 10.0, "y": 20.0, "body": "Logo?", "author": {"name": "Bob"}}));
        let r = s.execute("review.sync", &json!({})).unwrap();
        assert_eq!((r["comments"].as_u64(), r["project"]["name"].as_str()), (Some(2), Some("Acme annual report")));
        let list = s.execute("review.comments", &json!({})).unwrap();
        assert_eq!(list[0]["anchored"]["status"], "found", "{list}");
        assert!(list[1]["anchored"].is_null(), "a pin has no text to find");
        // The text moves: the quote finds it again.
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("text.insert", &json!({"text": "Oh, ", "raw": true})).unwrap();
        let list = s.execute("review.comments", &json!({})).unwrap();
        assert_eq!((list[0]["anchored"]["status"].as_str(), list[0]["anchored"]["start"].as_u64()), (Some("moved"), Some(4)));
        s.execute("review.goTo", &json!({"id": list[0]["id"]})).unwrap();
        let sel = s.doc().unwrap().selection.text.unwrap();
        assert_eq!((sel.anchor, sel.focus), (4, 9), "Go To selects the text where it is now");
        // The text goes: orphaned, here and on the service.
        write(&mut s, sid, "Nothing left");
        assert_eq!(s.execute("review.comments", &json!({})).unwrap()[0]["anchored"]["status"], "orphaned");
        s.execute("review.sync", &json!({})).unwrap();
        assert_eq!(fake.comments()[0]["status"], "orphaned");
    }

    #[test]
    fn apply_makes_a_tracked_change_and_tells_the_service() {
        let (mut s, sid, fake) = with_service("Hello review world");
        publish(&mut s);
        fake.add_comment(json!({"id": "c1", "kind": "replace", "page": 0, "story": sid.0, "start": 0, "end": 5, "quote": {"exact": "Hello"}, "replacement": "Hi", "author": {"name": "Ann"}}));
        fake.add_comment(json!({"id": "c2", "kind": "strike", "page": 0, "story": sid.0, "start": 6, "end": 13, "quote": {"exact": "review "}, "author": {"name": "Ann"}}));
        s.execute("review.sync", &json!({})).unwrap();
        let r = s.execute("review.apply", &json!({"id": "c1"})).unwrap();
        assert_eq!(r["status"], "applied", "{r}");
        let changes = s.execute("changes.list", &json!({})).unwrap();
        let kinds: Vec<(String, String)> =
            changes.as_array().unwrap().iter().map(|c| (c["kind"].as_str().unwrap().into(), c["text"].as_str().unwrap().into())).collect();
        assert!(kinds.contains(&("deleted".into(), "Hello".into())) && kinds.contains(&("inserted".into(), "Hi".into())), "{kinds:?}");
        assert_eq!(fake.comment("c1")["status"], "applied");
        // The strike finds its text past the tracked change and deletes it (tracked too).
        s.execute("review.apply", &json!({"id": "c2"})).unwrap();
        s.execute("changes.acceptAll", &json!({})).unwrap();
        assert_eq!(story_text(&s, sid), "Hi world");
        assert!(s.execute("review.apply", &json!({"id": "c1"})).is_err(), "applied once");
        // An orphaned comment is never applied.
        fake.add_comment(json!({"id": "c3", "kind": "replace", "page": 0, "story": sid.0, "start": 0, "end": 4, "quote": {"exact": "Gone"}, "replacement": "x", "author": {"name": "Ann"}}));
        s.execute("review.sync", &json!({})).unwrap();
        let e = s.execute("review.apply", &json!({"id": "c3"})).unwrap_err().to_string();
        assert!(e.contains("not found") || e.contains("orphan"), "{e}");
        assert_eq!(story_text(&s, sid), "Hi world");
    }

    #[test]
    fn add_as_note_puts_the_comment_in_the_text() {
        let (mut s, sid, fake) = with_service("Hello review world");
        publish(&mut s);
        fake.add_comment(json!({"id": "c1", "kind": "highlight", "page": 0, "story": sid.0, "start": 6, "end": 12, "quote": {"exact": "review"}, "body": "Nice word", "author": {"name": "Ann"}}));
        s.execute("review.sync", &json!({})).unwrap();
        let r = s.execute("review.addNote", &json!({"id": "c1"})).unwrap();
        assert_eq!(r["story"], sid.0);
        let notes = s.execute("note.list", &json!({})).unwrap();
        let n = &notes[0];
        assert_eq!(n["author"], "Ann (Review)");
        assert!(n["text"].as_str().unwrap().contains("Nice word") && n["text"].as_str().unwrap().contains("review"), "{n}");
        assert_eq!(n["at"], 12, "at the end of the quoted text");
        assert_eq!(fake.comment("c1")["status"], "applied");
    }

    #[test]
    fn reply_resolve_and_share_links_go_through_the_service() {
        let (mut s, sid, fake) = with_service("Hello review world");
        publish(&mut s);
        fake.add_comment(json!({"id": "c1", "kind": "note", "page": 0, "story": sid.0, "body": "?", "author": {"name": "Ann"}}));
        s.execute("review.sync", &json!({})).unwrap();
        s.execute("review.resolve", &json!({"id": "c1"})).unwrap();
        assert_eq!(fake.comment("c1")["status"], "resolved");
        s.execute("review.resolve", &json!({"id": "c1", "on": false})).unwrap();
        assert_eq!(fake.comment("c1")["status"], "open");
        s.execute("prefs.set", &json!({"userName": "Chris"})).unwrap();
        s.execute("review.reply", &json!({"id": "c1", "body": "Done"})).unwrap();
        let reply = fake.comments().into_iter().find(|c| c["inReplyTo"] == "c1").unwrap();
        assert_eq!((reply["body"].as_str(), reply["author"]["name"].as_str()), (Some("Done"), Some("Chris")));
        let l = s.execute("review.link", &json!({})).unwrap();
        assert!(l["url"].as_str().unwrap().starts_with("https://review.test/r/"), "{l}");
        assert_eq!(s.execute("review.comments", &json!({})).unwrap().as_array().unwrap().len(), 2, "the reply is synced too");
    }

    #[test]
    fn without_a_transport_or_settings_the_commands_say_what_is_missing() {
        let (mut s, _) = start("x");
        let e = s.execute("review.projects", &json!({})).unwrap_err().to_string();
        assert!(e.contains("review service"), "{e}");
        s.transport = Some(FakeService::with_project("P"));
        let e = s.execute("review.projects", &json!({})).unwrap_err().to_string();
        assert!(e.contains("reviewServer"), "{e}");
        assert!(s.execute("review.sync", &json!({})).is_err());
        // The host's default (an edition's own service) stands in for the preference.
        s.review_server_default = Some("https://review.test".into());
        s.execute("prefs.set", &json!({"reviewToken": "t"})).unwrap();
        assert_eq!(s.execute("review.projects", &json!({})).unwrap()[0]["name"], "P");
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
