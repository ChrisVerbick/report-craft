//! Window › Links: placed graphics with their status, effective resolution and uses; relink,
//! update, embed, go to.
//!
//! Every placed graphic keeps its bytes in the document; a linked graphic also remembers its
//! file. Its status compares that file with the copy in the document: missing, modified (the
//! file's bytes differ) or OK.
//!
//! A linked text file (File › Place with Create Links) remembers its file's size and hash on the
//! story instead; updating it imports the file again into the story.

use std::sync::Arc;

use designcraft_doc::{AssetId, Content, Document, Fingerprint, ItemId, Selection, Story, StoryId, TextFileLink};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc, str_param};
use crate::{EngineError, Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "links.check",
            "Check Links",
            [],
            None,
            "{asset? | story?} — the errors the files of modified linked text stories show, as Update would bring them in: Excel error values, formulas taking values from another workbook or never calculated, Word field errors → [{story, name, at, cell?: {row, col}, shows, formula?, kind: errorValue|otherWorkbook|notCalculated|fieldError, blocks: whether Update stops for it (all but otherWorkbook), message}]",
            has_doc,
            check
        ),
        cmd!(query "links.list", "Links", [], None, "{} → [{asset, name, path, status: ok|modified|missing|embedded, pixels, pdfCrop (placed PDFs: the box shown), uses: [{id, page, ppi}]}], then linked text files: [{story, name, path, status (also tooLarge: unchanged size, too big to check), edited (the story was changed in the layout), merges (Update merges the file's changes in rather than replacing the story), lastUpdate?: {at (seconds since 1970; 0 unknown), report (as links.update returned it, or {story, replaced: true}), errors}, uses: [{id, page}]}]", has_doc, |s, _| {
            // Taken out for the call so the document needn't be cloned on every poll.
            let mut checks = std::mem::take(&mut s.text_link_checks);
            let links = s.doc().map(|d| list(&d.doc, &mut checks));
            s.text_link_checks = checks;
            Ok(Value::Array(links?))
        }),
        cmd!(
            "links.relink",
            "Relink…",
            ["Window", "Links"],
            None,
            "{asset | story, path} — use another file for every placement of the graphic, or import another text file into the linked story",
            has_doc,
            relink
        ),
        cmd!(
            "links.relinkFolder",
            "Relink to Folder…",
            ["Window", "Links"],
            None,
            "{dir?: folder to look in (default: each link's own folder), extension?: e.g. \"tif\" (Relink File Extension), assets?: [ids], missingOnly?: bool (default true with `dir`)} → {relinked, notFound: [names]}",
            has_doc,
            relink_folder
        ),
        cmd!(
            "links.update",
            "Update Link",
            ["Window", "Links"],
            None,
            "{asset? | story?, mode?: merge|replace, allowErrors?: bool, dryRun?: bool, text?: bool} — reload modified links from their files (all when neither; text: false leaves linked text files alone, for the app's Flow review); with dryRun, return what the update would do and change nothing; a file that shows errors (see links.check; values from other workbooks are only reported) is refused unless allowErrors. A linked text file's changes merge into its story with the import options it was placed with: the layout's own edits survive and are reported, and where both changed a sentence the file's text wins and a Flow note keeps the layout's; refused for a story with tracked changes. With mode replace, or for a link placed before merging existed, the file is imported again, replacing edits made in the layout → {updated, warnings, reports: [{story, changes: [{before, after, …}], deviations: [{file, layout, …}], conflicts: [{file, layout, …}], warnings, pages: [{index, name}], fit: {overset, lines}}], errors: [as links.check]}; each entry has cell? {table, row, col}, at {start, end} (bytes in the updated story or cell, for text.select / table.select), was? {start, end} (the same place in the story or cell before the update, for Go To while reviewing), words [{kind: same|deleted|inserted, text}], page, pageName, y (0–1 from the page top) and overset (in text that doesn't fit)",
            has_doc,
            update
        ),
        cmd!(
            "links.clearHighlights",
            "Clear Flow Highlights",
            [],
            None,
            "{story?} — take Flow's highlights of the last update off a linked text story (all when none); errors stay highlighted until they are fixed → {cleared}",
            has_doc,
            clear_highlights
        ),
        cmd!(
            "links.embed",
            "Embed Link",
            ["Window", "Links"],
            None,
            "{asset | story} — keep only the copy in the document (a text file: unlink the story)",
            has_doc,
            |s, p| {
                if let Some(sid) = story_param(p) {
                    return s.edit(|d, _| {
                        d.story_mut(sid).ok_or_else(|| bad("links.embed", "no such story"))?.file_link = None;
                        Ok(Value::Null)
                    });
                }
                let aid = asset_param(p, "links.embed")?;
                s.edit(|d, _| {
                    let a = d.assets.get_mut(&aid).ok_or_else(|| bad("links.embed", "no such graphic"))?;
                    Arc::make_mut(a).link = None;
                    Ok(Value::Null)
                })
            }
        ),
        cmd!(noundo "links.goTo", "Go To Link", ["Window", "Links"], None, "{asset | story} — selects the frames showing the graphic or the linked text", has_doc, |s, p| {
            let st = s.doc_mut()?;
            let ids = match story_param(p) {
                Some(sid) => st.doc.story(sid).map(|s| s.frames.clone()).unwrap_or_default(),
                None => uses(&st.doc, asset_param(p, "links.goTo")?),
            };
            if ids.is_empty() {
                return Err(bad("links.goTo", "the graphic isn't placed"));
            }
            st.selection = Selection::items(ids.clone());
            st.bump_revision();
            Ok(json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
        }),
    ]
}

fn asset_param(p: &Value, cmd: &str) -> Result<AssetId> {
    p.get("asset").and_then(Value::as_u64).map(AssetId).ok_or_else(|| bad(cmd, "missing asset"))
}

fn story_param(p: &Value) -> Option<StoryId> {
    p.get("story").and_then(Value::as_u64).map(StoryId)
}

/// FNV-1a (64-bit) of `bytes`, in hex: a linked text file's fingerprint.
pub(crate) fn fnv_hex(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3));
    format!("{h:016x}")
}

/// Fingerprint of a story's text, formatting and tables' cells (edits made in the layout change it).
/// Tables count only when the story has some, so stories without tables keep the fingerprint
/// earlier builds recorded.
pub(crate) fn story_hash(st: &Story) -> String {
    // Flow's highlight marks are the panel's, not edits: hashed as if they weren't there.
    let chars = unflowed(&st.chars);
    let json = if st.tables.is_empty() {
        serde_json::to_string(&(&st.text, &st.paras, &chars))
    } else {
        // A table counts by its cells' text and formatting, not by revision counters, which
        // change without an edit (a merge, a highlight).
        let tables: Vec<Vec<(&str, &Vec<designcraft_doc::ParaFormat>, Vec<designcraft_doc::CharRun>)>> =
            st.tables.values().map(|t| t.cells.iter().map(|c| (c.text.text.as_str(), &c.text.paras, unflowed(&c.text.chars))).collect()).collect();
        serde_json::to_string(&(&st.text, &st.paras, &chars, &tables))
    };
    fnv_hex(json.unwrap_or_default().as_bytes())
}

/// `runs` without Flow's highlight marks, joined where that leaves neighbours alike.
fn unflowed(runs: &[designcraft_doc::CharRun]) -> Vec<designcraft_doc::CharRun> {
    let mut out: Vec<designcraft_doc::CharRun> = Vec::with_capacity(runs.len());
    for r in runs {
        let mut format = r.format.clone();
        format.over.flow = None;
        match out.last_mut() {
            Some(last) if last.format == format => last.len += r.len,
            _ => out.push(designcraft_doc::CharRun { len: r.len, format }),
        }
    }
    out
}

/// Frames (graphic content) showing an asset, top-level or in groups.
fn uses(d: &Document, aid: AssetId) -> Vec<ItemId> {
    let mut out = Vec::new();
    for sp in d.spreads.iter().chain(&d.parents) {
        for it in &sp.items {
            it.walk(&mut |i| {
                if let Content::Graphic(g) = &i.content
                    && g.asset == aid
                {
                    out.push(i.id);
                }
            });
        }
    }
    out
}

/// Whether an asset's linked file is gone ([`status`] "missing" without reading the file). An
/// asset can hold data while its file is missing: the document's preview of it.
pub fn link_missing(a: &designcraft_doc::Asset) -> bool {
    let Some(path) = &a.link else { return false };
    #[cfg(not(target_arch = "wasm32"))]
    {
        !crate::io::exists(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        false
    }
}

/// Link status of an asset.
pub fn status(a: &designcraft_doc::Asset) -> &'static str {
    let Some(path) = &a.link else { return "embedded" };
    #[cfg(not(target_arch = "wasm32"))]
    {
        match crate::io::metadata(path) {
            Err(_) => "missing",
            Ok(m) if m.len() as usize != a.data.len() => "modified",
            // Same size: compare the bytes (cheap for the sizes involved, and exact).
            Ok(_) => match crate::io::read_file(path) {
                Ok(b) if b == *a.data => "ok",
                Ok(_) => "modified",
                Err(_) => "missing",
            },
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        "ok"
    }
}

/// The largest linked text file read (the Data Merge source limit). Only regular files are read,
/// so a link to a FIFO or a device (e.g. `/dev/zero`) can't block or fill memory.
const MAX_TEXT_LINK_BYTES: u64 = super::datamerge::MAX_DATA_FILE_BYTES;

/// Can a linked text file this size be checked and updated?
pub(crate) fn checkable_size(len: usize) -> bool {
    len as u64 <= MAX_TEXT_LINK_BYTES
}

/// Can a size and time be trusted to change when the file does? Times are whole seconds, so not
/// while the file was changed in the last two: another change in that second keeps the time. A
/// time in the future (clock skew, a file copied from another machine) counts as settled, or such
/// a file would be read on every poll; a later save gives it a time unlike the remembered one.
pub(crate) fn settled(f: &Fingerprint) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        f.mtime > now || now.saturating_sub(f.mtime) >= 2
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = f;
        false
    }
}

/// Checked linked text files: path → (size and time, hash of the contents then).
pub(crate) type TextChecks = std::collections::HashMap<String, (Fingerprint, String)>;

/// Link status of a linked text file: `tooLarge` when it is the recorded size but over
/// [`MAX_TEXT_LINK_BYTES`], so its contents aren't checked. The file is read only when its size
/// is the recorded one but its time isn't, and then once per new time (`checks`): a file saved
/// or synced again without changes stays OK.
pub(crate) fn text_status(l: &TextFileLink, checks: &mut TextChecks) -> &'static str {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !crate::io::is_file(&l.path) {
            return "missing";
        }
        let now = match crate::io::metadata(&l.path) {
            Err(_) => return "missing",
            Ok(m) => super::datamerge::fingerprint_from(&m),
        };
        if now.size != l.size {
            return "modified";
        }
        if l.mtime != 0 && now.mtime == l.mtime {
            return "ok";
        }
        let hash = match checks.get(&l.path) {
            Some((at, hash)) if *at == now => hash.clone(),
            _ if now.size > MAX_TEXT_LINK_BYTES => return "tooLarge",
            _ => match read_text(&l.path) {
                Ok((b, at)) => {
                    let hash = fnv_hex(&b);
                    // The time from before the read: a change during it is seen next time.
                    if settled(&at) {
                        checks.insert(l.path.clone(), (at, hash.clone()));
                    }
                    hash
                }
                Err(_) => return "missing",
            },
        };
        if hash == l.hash { "ok" } else { "modified" }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (l, checks);
        "ok"
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

fn list(d: &Document, checks: &mut TextChecks) -> Vec<Value> {
    let texts = d.stories.values().filter_map(|st| {
        let l = st.file_link.as_ref()?;
        let uses: Vec<Value> = st.frames.iter().map(|id| json!({"id": id.0, "page": d.page_of_item(*id).map(|p| d.page_name(p))})).collect();
        Some(json!({"story": st.id.0, "name": file_name(&l.path), "path": l.path, "status": text_status(l, checks), "edited": story_hash(st) != l.story_hash, "merges": l.base.is_some(), "lastUpdate": l.last_update, "uses": uses}))
    });
    let used = d.used_assets();
    d.assets
        .values()
        .filter(|a| used.contains(&a.id))
        .map(|a| {
            let uses: Vec<Value> = uses(d, a.id)
                .into_iter()
                .map(|id| {
                    // Effective resolution: image pixels over the size it is shown at.
                    let ppi = (|| {
                        let loc = d.find(id)?;
                        let it = d.item_at(&loc)?;
                        let Content::Graphic(g) = &it.content else { return None };
                        // Vector (placed PDF or SVG): no effective resolution.
                        if a.mime == "application/pdf" || a.mime == "image/svg+xml" {
                            return None;
                        }
                        let (pw, _) = a.pixels?;
                        let xf = d.parent_xf(&loc) * it.xf * g.xf;
                        let shown_w = xf.transform_rect_bbox(designcraft_geom::Rect::new(0.0, 0.0, g.size.0, g.size.1)).width();
                        (shown_w > 0.0).then(|| (pw as f64 / (shown_w / 72.0)).round())
                    })();
                    let page = d.page_of_item(id).map(|p| d.page_name(p));
                    json!({"id": id.0, "page": page, "ppi": ppi})
                })
                .collect();
            let pdf_crop = (a.mime == "application/pdf").then(|| a.pdf_crop.name());
            json!({"asset": a.id.0, "name": a.name, "path": a.link, "status": status(a), "pixels": a.pixels, "pdfCrop": pdf_crop, "uses": uses})
        })
        .chain(texts)
        .collect()
}

/// A linked text file's bytes, with its size and time from before they were read.
#[cfg(not(target_arch = "wasm32"))]
fn read_text(path: &str) -> Result<(Vec<u8>, Fingerprint)> {
    let err = |e: std::io::Error| EngineError::Other(format!("{path}: {e}"));
    // The roots' own message when they refuse the path.
    crate::file_access::check_read(path).map_err(EngineError::Other)?;
    let m = crate::io::metadata(path).map_err(err)?;
    if !crate::io::is_file(path) {
        return Err(EngineError::Other(format!("{path}: not a file")));
    }
    let too_large = || EngineError::Other(format!("{path}: larger than {} MiB", MAX_TEXT_LINK_BYTES / (1024 * 1024)));
    if m.len() > MAX_TEXT_LINK_BYTES {
        return Err(too_large());
    }
    // At most one byte over the limit, in case the file grew since.
    let b = crate::io::read_capped(path, MAX_TEXT_LINK_BYTES).map_err(err)?;
    if b.len() as u64 > MAX_TEXT_LINK_BYTES {
        return Err(too_large());
    }
    Ok((b, super::datamerge::fingerprint_from(&m)))
}

#[cfg(target_arch = "wasm32")]
fn read_text(path: &str) -> Result<(Vec<u8>, Fingerprint)> {
    Err(EngineError::Other(format!("{path}: files can't be read on the web")))
}

/// Import a linked text file again; a warning says when edits made in the layout were replaced.
fn reimport(
    d: &mut Document,
    sid: StoryId,
    path: &str,
    bytes: &[u8],
    before: Fingerprint,
    allow_errors: bool,
) -> Result<(Vec<String>, Vec<designcraft_textimport::Problem>)> {
    let edited = d.story(sid).is_some_and(|st| st.file_link.as_ref().is_some_and(|l| story_hash(st) != l.story_hash));
    let (mut warnings, problems) = super::place_text::reimport(d, sid, path, bytes, Some(before), allow_errors)?;
    if edited {
        warnings.insert(0, format!("{}: edits made to story {} in the layout were replaced by the file's text", file_name(path), sid.0));
    }
    Ok((warnings, problems))
}

/// A file's error for the check and the update result: the story, the file, where, what it shows,
/// its formula, what kind of error, and a one-line message.
pub(super) fn problem_json(sid: Option<StoryId>, path: &str, p: &designcraft_textimport::Problem) -> Value {
    use designcraft_textimport::ProblemKind;
    let kind = match p.kind {
        ProblemKind::ErrorValue => "errorValue",
        ProblemKind::OtherWorkbook => "otherWorkbook",
        ProblemKind::NotCalculated => "notCalculated",
        ProblemKind::FieldError => "fieldError",
    };
    let cell = p.cell.map(|(row, col)| json!({"row": row, "col": col}));
    json!({"story": sid.map(|s| s.0), "name": file_name(path), "at": p.at, "cell": cell, "shows": p.shows, "formula": p.formula, "kind": kind, "blocks": p.kind.blocks(), "message": p.message()})
}

/// The errors the files of modified linked text stories show, as an update would bring them in.
fn check(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.doc()?.doc.clone();
    let checks = &mut s.text_link_checks;
    let only = story_param(p);
    let all = only.is_none() && p.get("asset").is_none();
    let mut out = Vec::new();
    for st in d.stories.values().filter(|st| all || only == Some(st.id)) {
        let Some(l) = st.file_link.as_ref().filter(|l| text_status(l, checks) == "modified") else { continue };
        let (bytes, _) = read_text(&l.path)?;
        let name = file_name(&l.path);
        let imp = designcraft_textimport::import(&name, &bytes).map_err(|e| bad("links.check", format!("{name}: {e}")))?;
        out.extend(imp.problems.iter().map(|pr| problem_json(Some(st.id), &l.path, pr)));
    }
    Ok(Value::Array(out))
}

#[cfg(not(target_arch = "wasm32"))]
fn reload(path: &str) -> Result<(Vec<u8>, (u32, u32))> {
    let b = crate::io::read_file(path).map_err(EngineError::Other)?;
    let px = designcraft_render::image_size(&b).ok_or_else(|| bad("links", format!("{path}: unsupported or corrupt image")))?;
    Ok((b, px))
}

#[cfg(target_arch = "wasm32")]
fn reload(path: &str) -> Result<(Vec<u8>, (u32, u32))> {
    Err(EngineError::Other(format!("{path}: files can't be read on the web")))
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad("links.relink", "missing path"))?.to_string();
    if let Some(sid) = story_param(p) {
        let (bytes, before) = read_text(&path)?;
        let name = file_name(&path);
        return s.edit(|d, _| {
            // A relinked file is chosen by hand: its errors are warnings.
            let (mut warnings, problems) = reimport(d, sid, &path, &bytes, before, true)?;
            warnings.extend(problems.iter().map(designcraft_textimport::Problem::message));
            Ok(json!({"name": name, "warnings": warnings}))
        });
    }
    let aid = asset_param(p, "links.relink")?;
    let (bytes, px) = reload(&path)?;
    let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
    s.edit(|d, _| {
        let a = d.assets.get_mut(&aid).ok_or_else(|| bad("links.relink", "no such graphic"))?;
        let a = Arc::make_mut(a);
        a.mime = designcraft_render::image_mime(&bytes).to_string();
        a.data = Arc::new(bytes.clone());
        a.pixels = Some(px);
        a.name = name.clone();
        a.link = Some(path.clone());
        // A placed PDF keeps its crop choice; its box is found on the new file.
        super::file::crop_pdf(a);
        Ok(json!({"name": name}))
    })
}

fn relink_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = str_param(p, "dir").map(std::path::PathBuf::from);
    if let Some(dir) = &dir {
        // Refused outright: candidates outside the automation read root would only look missing.
        crate::file_access::check_read(dir).map_err(EngineError::Other)?;
    }
    let ext = str_param(p, "extension").map(|e| e.trim_start_matches('.').to_string());
    if dir.is_none() && ext.is_none() {
        return Err(bad("links.relinkFolder", "give `dir` and/or `extension`"));
    }
    let missing_only = p.get("missingOnly").and_then(Value::as_bool).unwrap_or(dir.is_some() && ext.is_none());
    let only: Option<Vec<u64>> = p.get("assets").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect());
    let d = &s.doc()?.doc;
    let mut todo: Vec<(AssetId, std::path::PathBuf)> = Vec::new();
    let mut not_found = Vec::new();
    for a in d.assets.values() {
        if only.as_ref().is_some_and(|o| !o.contains(&a.id.0)) || (missing_only && status(a) != "missing") {
            continue;
        }
        let old = std::path::PathBuf::from(a.link.clone().unwrap_or_else(|| a.name.clone()));
        let mut name = old.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| a.name.clone().into());
        if let Some(e) = &ext {
            name = std::path::Path::new(&name).with_extension(e).into_os_string();
        }
        let folder = dir.clone().or_else(|| old.parent().map(|p| p.to_path_buf())).unwrap_or_default();
        let cand = folder.join(&name);
        if crate::io::exists(&cand) {
            todo.push((a.id, cand));
        } else {
            not_found.push(name.to_string_lossy().to_string());
        }
    }
    let mut n = 0;
    for (aid, path) in todo {
        relink(s, &json!({"asset": aid.0, "path": path.to_string_lossy()}))?;
        n += 1;
    }
    Ok(json!({"relinked": n, "notFound": not_found}))
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let merge = match p.get("mode").and_then(Value::as_str) {
        None | Some("merge") => true,
        Some("replace") => false,
        Some(m) => return Err(bad("links.update", format!("unknown mode `{m}` (merge, replace)"))),
    };
    let allow_errors = p.get("allowErrors").and_then(Value::as_bool).unwrap_or(false);
    let dry = p.get("dryRun").and_then(Value::as_bool).unwrap_or(false);
    let with_text = p.get("text").and_then(Value::as_bool).unwrap_or(true);
    let d = s.doc()?.doc.clone();
    let checks = &mut s.text_link_checks;
    let (only_asset, only_story) = (p.get("asset").and_then(Value::as_u64), story_param(p));
    let all = only_asset.is_none() && only_story.is_none();
    let which: Vec<(AssetId, String)> = d
        .assets
        .values()
        .filter(|a| all || only_asset == Some(a.id.0))
        .filter(|a| status(a) == "modified")
        .filter_map(|a| a.link.clone().map(|l| (a.id, l)))
        .collect();
    let texts: Vec<(StoryId, String)> = d
        .stories
        .values()
        .filter(|st| with_text && (all || only_story == Some(st.id)))
        .filter_map(|st| st.file_link.as_ref().filter(|l| text_status(l, checks) == "modified").map(|l| (st.id, l.path.clone())))
        .collect();
    let mut fresh = Vec::new();
    for (aid, path) in which {
        fresh.push((aid, reload(&path)?));
    }
    let mut fresh_texts = Vec::new();
    for (sid, path) in texts {
        let (bytes, before) = read_text(&path)?;
        fresh_texts.push((sid, path, bytes, before));
    }
    let n = fresh.len() + fresh_texts.len();
    if n == 0 {
        return Ok(json!({"updated": 0, "warnings": [], "reports": [], "errors": []}));
    }
    let run = |d: &mut Document| -> Result<Value> {
        for (aid, (bytes, px)) in &fresh {
            if let Some(a) = d.assets.get_mut(aid) {
                let a = Arc::make_mut(a);
                a.data = Arc::new(bytes.clone());
                a.pixels = Some(*px);
                super::file::crop_pdf(a);
            }
        }
        let (mut warnings, mut reports, mut errors) = (Vec::new(), Vec::new(), Vec::new());
        for (sid, path, bytes, before) in &fresh_texts {
            let (report, problems) = if merge && d.story(*sid).and_then(|st| st.file_link.as_ref()).is_some_and(|l| l.base.is_some()) {
                let (w, r, problems) = super::place_text::remerge(d, *sid, path, bytes, Some(*before), allow_errors)?;
                warnings.extend(w);
                reports.push(r.clone());
                (r, problems)
            } else {
                let (w, problems) = reimport(d, *sid, path, bytes, *before, allow_errors)?;
                warnings.extend(w);
                (json!({"story": sid.0, "replaced": true}), problems)
            };
            let found: Vec<Value> = problems.iter().map(|p| problem_json(Some(*sid), path, p)).collect();
            if let Some(l) = d.story_mut(*sid).and_then(|st| st.file_link.as_mut()) {
                l.last_update = Some(designcraft_doc::UpdateRecord { at: now_secs(), report, errors: found.clone() });
            }
            errors.extend(found);
        }
        Ok(json!({"updated": n, "warnings": warnings, "reports": reports, "errors": errors}))
    };
    if dry {
        // The whole update on a copy: the document, its undo history and the links stay as they were.
        return run(&mut (*d).clone());
    }
    s.edit(|d, _| run(d))
}

/// Clear Flow Highlights: the marks of the last update off the linked text stories (or `story`),
/// except errors, which stay until fixed.
fn clear_highlights(s: &mut Session, p: &Value) -> Result<Value> {
    let only = story_param(p);
    s.edit(|d, _| {
        let ids: Vec<StoryId> = d.stories.values().filter(|st| st.file_link.is_some() && only.is_none_or(|o| o == st.id)).map(|st| st.id).collect();
        for sid in &ids {
            if let Some(st) = d.story_mut(*sid) {
                super::place_text::unmark(st, true);
            }
        }
        Ok(json!({"cleared": ids.len()}))
    })
}

/// Seconds since 1970 (0 on the web, which has no clock to ask here).
fn now_secs() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        designcraft_render::Rendered { width: w, height: h, pixels: vec![200u8; (w * h * 4) as usize] }.to_png()
    }

    #[test]
    fn link_status_relink_update_embed() {
        let dir = std::env::temp_dir().join(format!("dc-links-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.png");
        let b = dir.join("b.png");
        std::fs::write(&a, png(10, 10)).unwrap();
        std::fs::write(&b, png(20, 10)).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("file.place", &json!({"path": a.to_string_lossy(), "x": 72, "y": 72, "width": 72})).unwrap();
        let l = s.execute("links.list", &json!({})).unwrap();
        assert_eq!(l[0]["status"], "ok");
        assert_eq!(l[0]["uses"][0]["ppi"], 10.0, "10 px over 1 inch");
        let aid = l[0]["asset"].as_u64().unwrap();
        // The file changes on disk.
        std::fs::write(&a, png(30, 10)).unwrap();
        assert_eq!(s.execute("links.list", &json!({})).unwrap()[0]["status"], "modified");
        s.execute("links.update", &json!({})).unwrap();
        let l = s.execute("links.list", &json!({})).unwrap();
        assert_eq!(l[0]["status"], "ok");
        assert_eq!(l[0]["pixels"], json!([30, 10]));
        // Missing, then relinked.
        std::fs::remove_file(&a).unwrap();
        assert_eq!(s.execute("links.list", &json!({})).unwrap()[0]["status"], "missing");
        s.execute("links.relink", &json!({"asset": aid, "path": b.to_string_lossy()})).unwrap();
        let l = s.execute("links.list", &json!({})).unwrap();
        assert_eq!((l[0]["status"].as_str(), l[0]["name"].as_str()), (Some("ok"), Some("b.png")));
        // Go to selects the frame; embed forgets the file.
        let r = s.execute("links.goTo", &json!({"asset": aid})).unwrap();
        assert_eq!(r["ids"].as_array().unwrap().len(), 1);
        s.execute("links.embed", &json!({"asset": aid})).unwrap();
        assert_eq!(s.execute("links.list", &json!({})).unwrap()[0]["status"], "embedded");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod relink_folder_tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn relink_missing_links_to_a_folder_and_by_extension() {
        let dir = std::env::temp_dir().join(format!("dc-relink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("new")).unwrap();
        let svg =
            |c: &str| format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="{c}"/></svg>"##);
        let orig = dir.join("logo.svg");
        std::fs::write(&orig, svg("red")).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("file.place", &json!({"path": orig.to_string_lossy()})).unwrap();
        // The file moves away: relink from the new folder.
        std::fs::rename(&orig, dir.join("new").join("logo.svg")).unwrap();
        let r = s.execute("links.relinkFolder", &json!({"dir": dir.join("new").to_string_lossy()})).unwrap();
        assert_eq!(r["relinked"], 1);
        let link = s.doc().unwrap().doc.assets.values().next().unwrap().link.clone().unwrap();
        assert!(link.contains("new"), "{link}");
        // Relink File Extension: the same name as .png doesn't exist → reported.
        let r = s.execute("links.relinkFolder", &json!({"extension": "png"})).unwrap();
        assert_eq!(r["relinked"], 0);
        assert_eq!(r["notFound"], json!(["logo.png"]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod shared_asset_tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        designcraft_render::Rendered { width: w, height: h, pixels: vec![200u8; (w * h * 4) as usize] }.to_png()
    }

    #[test]
    fn placements_of_one_file_share_an_asset_and_saves_drop_replaced_ones() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let a = super::super::file::base64_encode(&png(10, 10));
        s.execute("file.place", &json!({"base64": a, "name": "a.png", "x": 72, "y": 72})).unwrap();
        s.execute("edit.deselectAll", &json!({})).unwrap();
        s.execute("file.place", &json!({"base64": a, "name": "a.png", "x": 200, "y": 72})).unwrap();
        let l = s.execute("links.list", &json!({})).unwrap();
        assert_eq!(l.as_array().unwrap().len(), 1);
        assert_eq!(l[0]["uses"].as_array().unwrap().len(), 2);
        assert_eq!(s.doc().unwrap().doc.assets.len(), 1);

        // A new image into a frame: the old asset stays in memory (undo) but not in the file.
        let f = s.execute("frame.create", &json!({"rect": [100, 300, 200, 400]})).unwrap()["id"].as_u64().unwrap();
        let b = super::super::file::base64_encode(&png(20, 10));
        let c = super::super::file::base64_encode(&png(30, 10));
        s.execute("file.place", &json!({"base64": b, "name": "b.png", "frame": f})).unwrap();
        s.execute("file.place", &json!({"base64": c, "name": "c.png", "frame": f})).unwrap();
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.assets.len(), 3);
        assert_eq!(s.execute("links.list", &json!({})).unwrap().as_array().unwrap().len(), 2);
        let back = designcraft_format::load(&designcraft_format::save(&s.doc().unwrap().doc).unwrap()).unwrap();
        let mut names: Vec<&str> = back.assets.values().map(|a| a.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["a.png", "c.png"]);
    }

    #[test]
    fn files_with_identical_bytes_at_different_paths_keep_their_own_links() {
        let dir = std::env::temp_dir().join(format!("dc-same-bytes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.png"), dir.join("b.png"));
        std::fs::write(&a, png(10, 10)).unwrap();
        std::fs::write(&b, png(10, 10)).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        for (path, x) in [(&a, 72), (&b, 200)] {
            s.execute("edit.deselectAll", &json!({})).unwrap();
            s.execute("file.place", &json!({"path": path.to_string_lossy(), "x": x, "y": 72})).unwrap();
        }
        assert_eq!(s.doc().unwrap().doc.assets.len(), 2);
        let rows = |s: &mut Session| -> Vec<(String, Value)> {
            let l = s.execute("links.list", &json!({})).unwrap();
            l.as_array().unwrap().iter().map(|r| (r["path"].as_str().unwrap().to_string(), r["pixels"].clone())).collect()
        };
        let paths = |r: &[(String, Value)]| r.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();
        let before = rows(&mut s);
        assert_eq!(paths(&before), [a.to_string_lossy(), b.to_string_lossy()]);

        // b.png changes on disk: updating it leaves a.png's placement alone.
        std::fs::write(&b, png(20, 10)).unwrap();
        assert_eq!(s.execute("links.update", &json!({})).unwrap()["updated"], 1);
        let after = rows(&mut s);
        assert_eq!(paths(&after), paths(&before));
        assert_eq!((after[0].1.clone(), after[1].1.clone()), (json!([10, 10]), json!([20, 10])));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pasted_graphics_reuse_the_identical_asset_of_the_target() {
        use designcraft_doc::build::NewDocument;
        use designcraft_doc::{Asset, Graphic, Item, ItemId, Shape, SpreadRef};
        let asset = |id: u64, data: Vec<u8>| Asset {
            id: AssetId(id),
            name: "p.png".into(),
            mime: "image/png".into(),
            data: Arc::new(data),
            ..Default::default()
        };
        let mut src = Document::new(&NewDocument::default());
        let mut dst = Document::new(&NewDocument::default());
        let (pa, pb) = (png(4, 4), png(8, 4));
        // The source's asset id is taken in the target by a different file.
        let sid = AssetId(src.alloc());
        src.assets.insert(sid, Arc::new(asset(sid.0, pa.clone())));
        dst.assets.insert(sid, Arc::new(asset(sid.0, pb)));
        dst.alloc();
        let same = AssetId(dst.alloc());
        assert_ne!(same, sid);
        dst.assets.insert(same, Arc::new(asset(same.0, pa)));
        let iid = ItemId(src.alloc());
        let mut it = Item::new(
            iid,
            src.default_layer(),
            Shape::Rectangle,
            designcraft_geom::shapes::rectangle(designcraft_geom::Rect::new(0.0, 0.0, 9.0, 9.0)),
        );
        it.content = Content::Graphic(Graphic {
            asset: sid,
            size: (4.0, 4.0),
            xf: designcraft_geom::Affine::IDENTITY,
            clip: None,
            auto_fit: Default::default(),
            fit_align: 4,
            crop: [1.0; 4],
            wrap: Default::default(),
            fill: Default::default(),
        });
        src.insert_item(SpreadRef::Doc(0), it, None).unwrap();
        let ids = super::super::object::duplicate_from(&mut dst, &src, &[iid], SpreadRef::Doc(0), designcraft_geom::Vec2::ZERO).unwrap();
        let g = dst.item(ids[0]).and_then(|i| i.graphic()).unwrap();
        assert_eq!((g.asset, g.crop), (same, [1.0; 4]));
        assert_eq!(dst.assets.len(), 2);
    }

    #[test]
    fn pasted_missing_graphic_does_not_show_an_unrelated_target_asset() {
        use designcraft_doc::build::NewDocument;
        use designcraft_doc::{Asset, Graphic, Item, ItemId, Shape, SpreadRef};
        let mut src = Document::new(&NewDocument::default());
        let mut dst = Document::new(&NewDocument::default());
        // The source graphic points at an asset the source does not have; the target uses that id for another image.
        let missing = AssetId(src.alloc());
        dst.assets.insert(
            missing,
            Arc::new(Asset {
                id: missing,
                name: "p.png".into(),
                mime: "image/png".into(),
                link: None,
                data: Arc::new(png(8, 4)),
                pixels: None,
                ..Default::default()
            }),
        );
        let iid = ItemId(src.alloc());
        let mut it = Item::new(
            iid,
            src.default_layer(),
            Shape::Rectangle,
            designcraft_geom::shapes::rectangle(designcraft_geom::Rect::new(0.0, 0.0, 9.0, 9.0)),
        );
        it.content = Content::Graphic(Graphic {
            asset: missing,
            size: (4.0, 4.0),
            xf: designcraft_geom::Affine::IDENTITY,
            clip: None,
            auto_fit: Default::default(),
            fit_align: 4,
            crop: [1.0; 4],
            wrap: Default::default(),
            fill: Default::default(),
        });
        src.insert_item(SpreadRef::Doc(0), it, None).unwrap();
        let ids = super::super::object::duplicate_from(&mut dst, &src, &[iid], SpreadRef::Doc(0), designcraft_geom::Vec2::ZERO).unwrap();
        let g = dst.item(ids[0]).and_then(|i| i.graphic()).unwrap();
        assert_ne!(g.asset, missing);
        assert!(!dst.assets.contains_key(&g.asset));
    }

    #[test]
    fn pasted_missing_graphic_never_takes_an_asset_placed_later() {
        use designcraft_doc::build::NewDocument;
        use designcraft_doc::{Graphic, Item, ItemId, Shape, SpreadRef};
        let mut src = Document::new(&NewDocument::default());
        let mut dst = Document::new(&NewDocument::default());
        // The source's missing id is ahead of the target's counter: the target would allocate it to its next image.
        for _ in 0..50 {
            src.alloc();
        }
        let missing = AssetId(src.alloc());
        let iid = ItemId(src.alloc());
        let mut it = Item::new(
            iid,
            src.default_layer(),
            Shape::Rectangle,
            designcraft_geom::shapes::rectangle(designcraft_geom::Rect::new(0.0, 0.0, 9.0, 9.0)),
        );
        it.content = Content::Graphic(Graphic {
            asset: missing,
            size: (4.0, 4.0),
            xf: designcraft_geom::Affine::IDENTITY,
            clip: None,
            auto_fit: Default::default(),
            fit_align: 4,
            crop: [1.0; 4],
            wrap: Default::default(),
            fill: Default::default(),
        });
        src.insert_item(SpreadRef::Doc(0), it, None).unwrap();
        let ids = super::super::object::duplicate_from(&mut dst, &src, &[iid], SpreadRef::Doc(0), designcraft_geom::Vec2::ZERO).unwrap();
        let g = dst.item(ids[0]).and_then(|i| i.graphic()).unwrap().asset;
        assert!((0..100).all(|_| AssetId(dst.alloc()) != g), "a later image in the target takes the pasted graphic's id");
    }
}

#[cfg(test)]
mod text_link_tests {
    use serde_json::{Value, json};

    use crate::Session;
    use designcraft_doc::{ItemId, StoryId};

    /// A small Word file: a Title paragraph, then Body paragraphs (`body` is WordprocessingML).
    fn docx(body: &str) -> Vec<u8> {
        use std::io::Write;
        const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let styles = format!(
            r#"<w:styles {W}><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:rPr><w:sz w:val="56"/></w:rPr></w:style>
            <w:style w:type="paragraph" w:styleId="Body"><w:name w:val="Body"/><w:rPr><w:sz w:val="20"/></w:rPr></w:style></w:styles>"#
        );
        let doc = format!(
            r#"<w:document {W}><w:body><w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Head</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="Body"/></w:pPr>{body}</w:p></w:body></w:document>"#
        );
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        for (n, c) in [("word/document.xml", doc), ("word/styles.xml", styles)] {
            w.start_file(n, o).unwrap();
            w.write_all(c.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn text_rows(s: &mut Session) -> Vec<Value> {
        let l = s.execute("links.list", &json!({})).unwrap();
        l.as_array().unwrap().iter().filter(|r| r.get("story").is_some()).cloned().collect()
    }

    fn story(s: &Session, sid: StoryId) -> designcraft_doc::Story {
        s.doc().unwrap().doc.story(sid).unwrap().clone()
    }

    fn styles(st: &designcraft_doc::Story) -> Vec<String> {
        st.paras.iter().map(|p| p.style.clone()).collect()
    }

    #[test]
    fn linked_word_file_status_update_relink_and_unlink() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("moved")).unwrap();
        let path = dir.join("chapter.docx");
        std::fs::write(&path, docx("<w:r><w:t>First draft.</w:t></w:r>")).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 2})).unwrap();
        s.execute("style.paragraph.create", &json!({"name": "Body", "chars": {"size": 9}})).unwrap();
        s.execute("style.paragraph.create", &json!({"name": "Headline", "chars": {"size": 30}})).unwrap();
        // Two threaded frames.
        let a = s.execute("frame.create", &json!({"rect": [72, 72, 300, 200], "content": "text"})).unwrap()["id"].as_u64().unwrap();
        let b = s.execute("frame.create", &json!({"rect": [72, 72, 300, 200], "spread": 1, "content": "text"})).unwrap()["id"].as_u64().unwrap();
        s.edit(|d, _| Ok(d.thread(ItemId(a), ItemId(b))?)).unwrap();
        s.execute("edit.deselectAll", &json!({})).unwrap();

        // Place and link, with a style map and Auto Rename.
        let r = s
            .execute(
                "file.place",
                &json!({"path": path.to_string_lossy(), "frame": a, "link": true, "styleMap": {"Title": "Headline"}, "styleConflicts": "autoRename"}),
            )
            .unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        let placed = story(&s, sid);
        assert_eq!(placed.text, "Head\nFirst draft.");
        assert_eq!(styles(&placed), ["Headline", "Body_wrd_1"]);
        let link = placed.file_link.clone().unwrap();
        assert_eq!(link.path, path.to_string_lossy());
        assert_eq!(link.style_conflicts, "autoRename");
        assert_eq!(link.style_map.get("Title").map(String::as_str), Some("Headline"));
        assert_eq!(link.style_map.get("Body").map(String::as_str), Some("Body_wrd_1"), "Auto Rename's choice is kept");
        let rows = text_rows(&mut s);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0]["story"].as_u64(), rows[0]["name"].as_str()), (Some(sid.0), Some("chapter.docx")));
        assert_eq!((rows[0]["status"].as_str(), rows[0]["edited"].as_bool()), (Some("ok"), Some(false)));
        assert_eq!(rows[0]["uses"].as_array().unwrap().len(), 2, "both threaded frames");

        // The file changes (with tracked changes): Modified; Update imports it as if accepted, with
        // the saved style map, into the same frames, without new renamed styles.
        std::fs::write(
            &path,
            docx(r#"<w:r><w:t xml:space="preserve">Second </w:t></w:r><w:ins w:id="1" w:author="A"><w:r><w:t>revised </w:t></w:r></w:ins><w:del w:id="2" w:author="A"><w:r><w:delText>old </w:delText></w:r></w:del><w:r><w:t>draft.</w:t></w:r>"#),
        )
        .unwrap();
        assert_eq!(text_rows(&mut s)[0]["status"], "modified");
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        assert_eq!(r["updated"], 1);
        assert_eq!(r["warnings"], json!([]), "no edits in the layout to replace");
        let updated = story(&s, sid);
        assert_eq!(updated.text, "Head\nSecond revised draft.");
        assert_eq!(styles(&updated), ["Headline", "Body_wrd_1"]);
        assert_eq!(updated.frames, [ItemId(a), ItemId(b)], "frames and thread kept");
        assert!(s.doc().unwrap().doc.styles.para("Body_wrd_2").is_none());
        assert!(s.doc().unwrap().doc.styles.para("Title").is_none(), "mapped styles aren't imported");
        assert_eq!(text_rows(&mut s)[0]["status"], "ok");

        // An edit in the layout, then the file changes again: Update in replace mode replaces the
        // edit and warns (merging, the default, keeps it: see the merge tests below).
        s.execute("story.setText", &json!({"story": sid.0, "text": "Fixed in layout"})).unwrap();
        assert_eq!(text_rows(&mut s)[0]["edited"], true);
        std::fs::write(&path, docx("<w:r><w:t>Third draft.</w:t></w:r>")).unwrap();
        let r = s.execute("links.update", &json!({"mode": "replace"})).unwrap();
        assert_eq!(r["updated"], 1);
        let w = r["warnings"][0].as_str().unwrap();
        assert!(w.contains("chapter.docx") && w.contains("replaced"), "{w}");
        assert_eq!(story(&s, sid).text, "Head\nThird draft.");
        assert_eq!(text_rows(&mut s)[0]["edited"], false);
        // Undo brings the edit back.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(story(&s, sid).text, "Fixed in layout");
        s.execute("edit.redo", &json!({})).unwrap();

        // Saved and opened again: the link and its options are kept.
        let back = designcraft_format::load(&designcraft_format::save(&s.doc().unwrap().doc).unwrap()).unwrap();
        assert_eq!(back.story(sid).unwrap().file_link, story(&s, sid).file_link);
        assert_eq!(super::text_status(back.story(sid).unwrap().file_link.as_ref().unwrap(), &mut Default::default()), "ok");

        // The file moves: Missing; Relink to it imports it into the story.
        let moved = dir.join("moved").join("chapter.docx");
        std::fs::write(&moved, docx("<w:r><w:t>Moved draft.</w:t></w:r>")).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(text_rows(&mut s)[0]["status"], "missing");
        s.execute("links.relink", &json!({"story": sid.0, "path": moved.to_string_lossy()})).unwrap();
        let rows = text_rows(&mut s);
        assert_eq!((rows[0]["status"].as_str(), rows[0]["path"].as_str()), (Some("ok"), Some(moved.to_string_lossy().as_ref())));
        assert_eq!(story(&s, sid).text, "Head\nMoved draft.");

        // Go To selects the story's frames; Unlink keeps the text and drops the file.
        let r = s.execute("links.goTo", &json!({"story": sid.0})).unwrap();
        assert_eq!(r["ids"], json!([a, b]));
        s.execute("links.embed", &json!({"story": sid.0})).unwrap();
        assert!(text_rows(&mut s).is_empty());
        assert_eq!(story(&s, sid).text, "Head\nMoved draft.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn unchanged_linked_text_files_are_not_read_again() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("dc-text-link-poll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.txt");
        std::fs::write(&path, "Hello").unwrap();
        // Unreadable: a status that reads the file would say missing.
        let readable = |on: bool| std::fs::set_permissions(&path, std::fs::Permissions::from_mode(if on { 0o644 } else { 0o000 })).unwrap();
        // A time `secs` ago (a file changed in the last two seconds is always read).
        let touch = |secs: u64| {
            let f = std::fs::File::options().write(true).open(&path).unwrap();
            f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(secs)).unwrap();
        };
        touch(60);
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("file.place", &json!({"path": path.to_string_lossy(), "link": true})).unwrap();
        let status = |s: &mut Session| text_rows(s)[0]["status"].as_str().unwrap().to_string();

        // Size and time as placed: not read.
        readable(false);
        if std::fs::read(&path).is_ok() {
            // Running as root: permissions don't stop reads, so this can't be observed.
            return;
        }
        assert_eq!(status(&mut s), "ok");
        // Saved again unchanged (a new time): read once, then not again.
        readable(true);
        touch(30);
        assert_eq!(status(&mut s), "ok");
        readable(false);
        assert_eq!(status(&mut s), "ok", "the second poll doesn't read it");
        // Changed, same size, new time: modified.
        readable(true);
        std::fs::write(&path, "Jello").unwrap();
        touch(20);
        assert_eq!(status(&mut s), "modified");
        // Changed again in the same second, same size: still seen (just-changed files are read).
        std::fs::write(&path, "Hello").unwrap();
        assert_eq!(status(&mut s), "ok");
        std::fs::write(&path, "Jello").unwrap();
        assert_eq!(status(&mut s), "modified");
        touch(10);
        // Updated: the new size and time are kept, so the next poll doesn't read it.
        s.execute("links.update", &json!({})).unwrap();
        readable(false);
        assert_eq!(status(&mut s), "ok");
        // A time in the future (clock skew): read once, then not on every poll.
        readable(true);
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600)).unwrap();
        assert_eq!(status(&mut s), "ok");
        readable(false);
        assert_eq!(status(&mut s), "ok", "a future time is remembered");
        readable(true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_links_when_placing_text_files_preference() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-pref-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notes.txt");
        std::fs::write(&path, "Plain text").unwrap();
        let place = |s: &mut Session, extra: Value| {
            let mut p = json!({"path": path.to_string_lossy(), "rect": [72, 72, 300, 200]});
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            s.execute("edit.deselectAll", &json!({})).unwrap();
            let r = s.execute("file.place", &p).unwrap();
            (s.doc().unwrap().doc.story(StoryId(r["story"].as_u64().unwrap())).unwrap().file_link.is_some(), r)
        };
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        assert!(!place(&mut s, json!({})).0, "off by default");
        assert!(place(&mut s, json!({"link": true})).0);
        s.execute("prefs.set", &json!({"linkTextFiles": true})).unwrap();
        assert!(place(&mut s, json!({})).0, "the preference links");
        assert!(!place(&mut s, json!({"link": false})).0, "`link` overrides it");
        // Placed from bytes: no file to link.
        let b64 = crate::cmd::base64_encode(b"Plain text");
        let r = s.execute("file.place", &json!({"base64": b64, "name": "notes.txt", "rect": [72, 300, 300, 400]})).unwrap();
        assert!(s.doc().unwrap().doc.story(StoryId(r["story"].as_u64().unwrap())).unwrap().file_link.is_none());
        // Into an insertion point: not linked, with a warning.
        let fr = s.execute("frame.create", &json!({"rect": [72, 500, 300, 600], "content": "text", "text": "Start  end"})).unwrap();
        let sid = fr["story"].as_u64().unwrap();
        s.execute("text.select", &json!({"story": sid, "anchor": 6, "focus": 6})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy()})).unwrap();
        assert!(r["warnings"][0].as_str().unwrap().contains("not linked"), "{r}");
        assert!(s.doc().unwrap().doc.story(StoryId(sid)).unwrap().file_link.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The text of what a linked story's last import brought in (its link's base), if any.
    fn base(s: &Session, sid: StoryId) -> Option<String> {
        Some(s.doc().unwrap().doc.story(sid)?.file_link.as_ref()?.base.as_ref()?.text.clone())
    }

    /// A linked story keeps what its last import brought in (what a merge compares against): set
    /// by place, update and relink, saved with the document, left alone by layout edits, dropped
    /// with the link. Only imported content: a Word file's tracked deletions and authors stay out.
    #[test]
    fn linked_text_keeps_what_its_last_import_brought_in() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-base-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("other")).unwrap();
        let (a, b) = (dir.join("a.docx"), dir.join("b.docx"));
        let tracked = r#"<w:r><w:t xml:space="preserve">First </w:t></w:r><w:del w:id="1" w:author="Reviewer A"><w:r><w:delText>secret </w:delText></w:r></w:del><w:r><w:t>draft.</w:t></w:r>"#;
        std::fs::write(&a, docx(tracked)).unwrap();
        std::fs::write(&b, docx("<w:r><w:t>Other.</w:t></w:r>")).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let place = |s: &mut Session, p: &std::path::Path, y: f64| {
            s.execute("edit.deselectAll", &json!({})).unwrap();
            let r = s.execute("file.place", &json!({"path": p.to_string_lossy(), "rect": [72, y, 300, y + 100.0], "link": true})).unwrap();
            StoryId(r["story"].as_u64().unwrap())
        };
        let (sa, sb) = (place(&mut s, &a, 72.0), place(&mut s, &b, 300.0));
        assert_eq!(base(&s, sa).as_deref(), Some("Head\nFirst draft."));
        let kept = story(&s, sa).file_link.unwrap().base.unwrap();
        assert!(kept.frames.is_empty() && kept.file_link.is_none(), "detached");
        let json = serde_json::to_string(&s.doc().unwrap().doc).unwrap();
        assert!(!json.contains("secret") && !json.contains("Reviewer A"), "nothing of the Word file beyond the import");
        assert!(s.doc().unwrap().doc.assets.is_empty(), "the Word file itself isn't stored");

        // Saved with the document; Update and Relink keep what they bring in.
        let saved = designcraft_format::load(&designcraft_format::save(&s.doc().unwrap().doc).unwrap()).unwrap();
        assert_eq!(saved.story(sa).unwrap().file_link, story(&s, sa).file_link);
        std::fs::write(&a, docx("<w:r><w:t>Second, longer.</w:t></w:r>")).unwrap();
        s.execute("links.update", &json!({"story": sa.0})).unwrap();
        assert_eq!(base(&s, sa).as_deref(), Some("Head\nSecond, longer."));
        assert_eq!(base(&s, sb).as_deref(), Some("Head\nOther."), "the other link keeps its own");
        let c = dir.join("other").join("c.docx");
        std::fs::write(&c, docx("<w:r><w:t>Third.</w:t></w:r>")).unwrap();
        s.execute("links.relink", &json!({"story": sa.0, "path": c.to_string_lossy()})).unwrap();
        assert_eq!(base(&s, sa).as_deref(), Some("Head\nThird."));

        // A layout edit leaves it as imported; unlinking drops it.
        s.execute("story.setText", &json!({"story": sa.0, "text": "Edited in layout"})).unwrap();
        assert_eq!(base(&s, sa).as_deref(), Some("Head\nThird."));
        s.execute("links.embed", &json!({"story": sa.0})).unwrap();
        assert_eq!(base(&s, sa), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Links saved before bases existed load without one.
    #[test]
    fn text_links_saved_without_a_base_load_without_one() {
        let l: designcraft_doc::TextFileLink = serde_json::from_str(r#"{"path": "a.docx", "size": 1, "hash": "00"}"#).unwrap();
        assert_eq!(l.base, None);
    }

    /// A temporary folder with a linked Word file of `body`, placed into a new document.
    fn placed(name: &str, body: &str) -> (std::path::PathBuf, std::path::PathBuf, Session, StoryId) {
        let dir = std::env::temp_dir().join(format!("dc-text-link-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.docx");
        std::fs::write(&path, docx(&format!("<w:r><w:t>{body}</w:t></w:r>"))).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [72, 72, 400, 300], "link": true})).unwrap();
        (dir, path, s, StoryId(r["story"].as_u64().unwrap()))
    }

    /// An edit made in the layout: `from` replaced by `to` in the story.
    fn layout_edit(s: &mut Session, sid: StoryId, from: &str, to: &str) {
        s.edit(|d, _| {
            let st = d.story_mut(sid).unwrap();
            let at = st.text.find(from).unwrap();
            st.replace(at..at + from.len(), to);
            Ok(())
        })
        .unwrap();
    }

    fn revise(path: &std::path::Path, body: &str) {
        std::fs::write(path, docx(&format!("<w:r><w:t>{body}</w:t></w:r>"))).unwrap();
    }

    /// With a base, Update merges: the file's changes come in, the layout's own edits survive and
    /// are reported, and undo brings back the story as it was.
    #[test]
    fn update_merges_the_files_changes_with_the_layouts_edits() {
        let (dir, path, mut s, sid) = placed("merge", "First sentence. Second sentence.");
        assert_eq!(text_rows(&mut s)[0]["merges"], true);
        layout_edit(&mut s, sid, "Second sentence.", "Layout sentence.");
        revise(&path, "First sentence, revised. Second sentence.");
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        assert_eq!(story(&s, sid).text, "Head\nFirst sentence, revised. Layout sentence.");
        assert!(r["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap().contains("replaced")), "{r}");
        let report = &r["reports"][0];
        assert_eq!(report["story"], sid.0);
        assert!(!report["deviations"].as_array().unwrap().is_empty() && report["conflicts"] == json!([]), "{report}");
        assert_eq!(
            story(&s, sid).file_link.unwrap().base.unwrap().text,
            "Head\nFirst sentence, revised. Second sentence.",
            "the base is the file as applied"
        );
        assert_eq!(text_rows(&mut s)[0]["edited"], true, "the kept layout sentence still differs from the file");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(story(&s, sid).text, "Head\nFirst sentence. Layout sentence.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Where both rewrote a sentence, the file's text wins and the layout's goes in a Flow note.
    #[test]
    fn update_notes_the_layouts_version_where_both_changed_a_sentence() {
        let (dir, path, mut s, sid) = placed("conflict", "First sentence. Second sentence.");
        layout_edit(&mut s, sid, "Second sentence.", "Layout version.");
        revise(&path, "First sentence. File version.");
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        let st = story(&s, sid);
        assert_eq!(st.text.replace(designcraft_doc::NOTE_MARK, ""), "Head\nFirst sentence. File version.");
        assert_eq!(st.editorial.len(), 1);
        assert_eq!(st.editorial[0].author, "Flow");
        assert!(st.editorial[0].text.starts_with("Layout had: "), "{:?}", st.editorial[0]);
        assert_eq!(r["reports"][0]["conflicts"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `mode: "replace"` imports the file again, as do links placed before merging existed.
    #[test]
    fn replace_mode_and_links_without_a_base_replace_the_layouts_edits() {
        let (dir, path, mut s, sid) = placed("replace", "One. Two.");
        layout_edit(&mut s, sid, "Two.", "Layout.");
        revise(&path, "One, revised. Two.");
        let r = s.execute("links.update", &json!({"story": sid.0, "mode": "replace"})).unwrap();
        assert_eq!(story(&s, sid).text, "Head\nOne, revised. Two.");
        assert!(r["warnings"][0].as_str().unwrap().contains("replaced"), "{r}");
        layout_edit(&mut s, sid, "Two.", "Layout again.");
        s.edit(|d, _| {
            d.story_mut(sid).unwrap().file_link.as_mut().unwrap().base = None;
            Ok(())
        })
        .unwrap();
        assert_eq!(text_rows(&mut s)[0]["merges"], false);
        revise(&path, "One, revised twice. Two.");
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        assert_eq!(story(&s, sid).text, "Head\nOne, revised twice. Two.");
        assert!(r["warnings"][0].as_str().unwrap().contains("replaced"), "{r}");
        assert!(s.execute("links.update", &json!({"mode": "guess"})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Text with tracked changes isn't one text yet: the update is refused and nothing changes.
    #[test]
    fn a_story_with_tracked_changes_is_not_merged() {
        let (dir, path, mut s, sid) = placed("tracked", "One. Two.");
        s.edit(|d, _| {
            d.story_mut(sid).unwrap().format_chars(5..8, |f| f.over.change = Some(designcraft_doc::ChangeMark::Inserted));
            Ok(())
        })
        .unwrap();
        revise(&path, "One, revised. Two.");
        let e = s.execute("links.update", &json!({"story": sid.0})).unwrap_err().to_string();
        assert!(e.contains("tracked changes"), "{e}");
        assert_eq!(story(&s, sid).text, "Head\nOne. Two.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The demo of 2026-10-16 as a test: the report reads in whole sentences, the conflict mark sits
    /// at the start of its sentence with the layout's sentence in the Flow note, and the story still
    /// counts as edited while it differs from the file.
    #[test]
    fn update_reports_whole_sentences_and_notes_the_layouts_sentence() {
        let (dir, path, mut s, sid) = placed("report", "Revenue grew in every region. The board approved the new plan. Costs were flat.");
        layout_edit(&mut s, sid, "The board approved the new plan.", "The board backed the plan.");
        layout_edit(&mut s, sid, "Costs were flat.", "Costs were held steady.");
        revise(&path, "Revenue grew in every region except the north. The board approved the new plan. Costs fell by two percent.");
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        let report = &r["reports"][0];
        assert_eq!(
            only(&report["deviations"], &["file", "layout"]),
            json!([{"file": "The board approved the new plan.", "layout": "The board backed the plan."}])
        );
        assert_eq!(
            only(&report["conflicts"], &["file", "layout"]),
            json!([{"file": "Costs fell by two percent.", "layout": "Costs were held steady."}])
        );
        let changes = report["changes"].as_array().unwrap();
        assert!(
            !changes.iter().any(|c| c["after"].as_str().unwrap().contains("Costs") || c["after"] == "[note]"),
            "a conflict is listed once, as a conflict: {report}"
        );
        assert!(
            changes.iter().any(|c| c["before"] == "Revenue grew in every region." && c["after"] == "Revenue grew in every region except the north."),
            "{report}"
        );
        let st = story(&s, sid);
        assert!(st.text.ends_with(&format!("{}Costs fell by two percent.", designcraft_doc::NOTE_MARK)), "{:?}", st.text);
        assert_eq!(st.editorial[0].text, "Layout had: Costs were held steady.");
        assert_eq!(text_rows(&mut s)[0]["edited"], true, "the story still differs from the file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A workbook whose first sheet holds `rows` as inline strings.
    fn workbook(rows: &[&[&str]]) -> Vec<u8> {
        use std::io::Write;
        let rows: String = rows
            .iter()
            .map(|row| format!("<row>{}</row>", row.iter().map(|v| format!(r#"<c t="inlineStr"><is><t>{v}</t></is></c>"#)).collect::<String>()))
            .collect();
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("xl/worksheets/sheet1.xml", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(format!("<worksheet><sheetData>{rows}</sheetData></worksheet>").as_bytes()).unwrap();
        w.finish().unwrap().into_inner()
    }

    /// A linked spreadsheet that kept its shape merges cell by cell: the file's new values come in,
    /// a cell edited in the layout keeps its text and is reported with its address, and the
    /// layout's cell formatting stays.
    #[test]
    fn update_merges_a_linked_spreadsheets_cells_and_keeps_its_formatting() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-cells-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prices.xlsx");
        std::fs::write(&path, workbook(&[&["Item", "Price"], &["Tea", "3.50"]])).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [72, 72, 300, 200], "link": true})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        let table = *story(&s, sid).tables.keys().next().unwrap();
        s.execute("table.select", &json!({"story": sid.0, "table": table, "rows": [0, 0], "cols": [0, 0]})).unwrap();
        s.execute("table.setCell", &json!({"fill": "[Black]"})).unwrap();
        s.execute("table.select", &json!({"story": sid.0, "table": table, "rows": [1, 1], "cols": [0, 0]})).unwrap();
        s.execute("table.setCell", &json!({"text": "Green tea"})).unwrap();

        std::fs::write(&path, workbook(&[&["Item", "Price"], &["Tea", "3.75"]])).unwrap();
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        let st = story(&s, sid);
        let t = st.tables.values().next().unwrap();
        let text = |r: usize, c: usize| t.cell(r, c).unwrap().text.text.clone();
        assert_eq!((text(1, 0), text(1, 1)), ("Green tea".to_string(), "3.75".to_string()));
        assert_eq!(t.cell(0, 0).unwrap().fill, "[Black]");
        let report = &r["reports"][0];
        assert_eq!(
            only(&report["deviations"], &["file", "layout", "cell"]),
            json!([{"file": "Tea", "layout": "Green tea", "cell": {"table": table, "row": 1, "col": 0}}]),
            "{report}"
        );
        assert!(report["changes"].as_array().unwrap().iter().any(|c| c["after"] == "3.75" && c["cell"]["row"] == 1), "{report}");
        assert_eq!(report["warnings"], json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A workbook whose first sheet is `sheet_data` (the cells of `<sheetData>`).
    fn workbook_xml(sheet_data: &str) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("xl/worksheets/sheet1.xml", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(format!("<worksheet><sheetData>{sheet_data}</sheetData></worksheet>").as_bytes()).unwrap();
        w.finish().unwrap().into_inner()
    }

    const DIV0: &str = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="B1" t="inlineStr"><is><t>Price</t></is></c></row>
        <row r="2"><c r="A2" t="inlineStr"><is><t>Tea</t></is></c><c r="B2" t="e"><f>A9/0</f><v>#DIV/0!</v></c></row>"#;

    /// A file that shows errors is not applied silently: the check lists them, Update refuses
    /// without `allowErrors`, and with it applies the file and reports them.
    #[test]
    fn update_asks_before_applying_a_file_that_shows_errors() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-errors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prices.xlsx");
        std::fs::write(&path, workbook(&[&["Item", "Price"], &["Tea", "3.50"]])).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [72, 72, 300, 200], "link": true})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        let price = |s: &Session| story(s, sid).tables.values().next().unwrap().cell(1, 1).unwrap().text.text.clone();

        std::fs::write(&path, workbook_xml(DIV0)).unwrap();
        let found = s.execute("links.check", &json!({"story": sid.0})).unwrap();
        assert_eq!(found.as_array().unwrap().len(), 1, "{found}");
        let e = &found[0];
        assert_eq!((e["story"].as_u64(), e["name"].as_str(), e["at"].as_str()), (Some(sid.0), Some("prices.xlsx"), Some("B2")));
        assert_eq!((e["shows"].as_str(), e["formula"].as_str(), e["kind"].as_str()), (Some("#DIV/0!"), Some("A9/0"), Some("errorValue")));
        assert_eq!(e["cell"], json!({"row": 1, "col": 1}));
        assert_eq!(e["blocks"], true);

        let refused = s.execute("links.update", &json!({"story": sid.0})).unwrap_err().to_string();
        assert!(refused.contains("#DIV/0!") && refused.contains("allowErrors"), "{refused}");
        assert_eq!(price(&s), "3.50", "nothing changed");
        let r = s.execute("links.update", &json!({"story": sid.0, "allowErrors": true})).unwrap();
        assert_eq!(price(&s), "#DIV/0!");
        assert_eq!(r["errors"].as_array().unwrap().len(), 1, "{r}");
        assert_eq!(r["errors"][0]["message"], "B2: #DIV/0! (formula =A9/0)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A value taken from another workbook may be out of date but is not an error: it is reported,
    /// and the update goes ahead without `allowErrors`.
    #[test]
    fn values_from_other_workbooks_are_reported_without_stopping_the_update() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-other-book-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prices.xlsx");
        std::fs::write(&path, workbook(&[&["Item", "Price"], &["Tea", "3.50"]])).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [72, 72, 300, 200], "link": true})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());

        std::fs::write(
            &path,
            workbook_xml(
                r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="B1" t="inlineStr"><is><t>Price</t></is></c></row>
                <row r="2"><c r="A2" t="inlineStr"><is><t>Tea</t></is></c><c r="B2"><f>[1]Prices!B4</f><v>3.75</v></c></row>"#,
            ),
        )
        .unwrap();
        let found = s.execute("links.check", &json!({"story": sid.0})).unwrap();
        assert_eq!((found[0]["kind"].as_str(), found[0]["blocks"].as_bool()), (Some("otherWorkbook"), Some(false)), "{found}");
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        assert_eq!(story(&s, sid).tables.values().next().unwrap().cell(1, 1).unwrap().text.text, "3.75");
        assert_eq!((r["errors"][0]["at"].as_str(), r["errors"][0]["blocks"].as_bool()), (Some("B2"), Some(false)), "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Placing a file that shows errors places it, with the errors as warnings.
    #[test]
    fn placing_a_file_that_shows_errors_warns() {
        let b64 = crate::cmd::base64_encode(&workbook_xml(DIV0));
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"base64": b64, "name": "prices.xlsx", "rect": [72, 72, 300, 200]})).unwrap();
        assert!(r["warnings"].as_array().unwrap().iter().any(|w| w == "B2: #DIV/0! (formula =A9/0)"), "{r}");
    }

    /// Report entries with only the given fields.
    fn only(entries: &Value, keys: &[&str]) -> Value {
        let keep = |e: &Value| keys.iter().filter_map(|k| e.get(*k).map(|v| (k.to_string(), v.clone()))).collect::<serde_json::Map<_, _>>();
        Value::Array(entries.as_array().unwrap().iter().map(|e| Value::Object(keep(e))).collect())
    }

    /// The text an entry's `at` covers in `text`, as the report shows it.
    fn covered(text: &str, e: &Value) -> String {
        covered_by(text, e, "at")
    }

    /// The text an entry's range `key` (`at` or `was`) covers in `text`, as the report shows it.
    fn covered_by(text: &str, e: &Value, key: &str) -> String {
        let (a, b) = (e[key]["start"].as_u64().unwrap() as usize, e[key]["end"].as_u64().unwrap() as usize);
        designcraft_flow::shown(text.get(a..b).unwrap()).trim().to_string()
    }

    /// What an entry's `words` rebuild: the unchanged runs and those of `kind`.
    fn rebuilt(e: &Value, kind: &str) -> String {
        e["words"].as_array().unwrap().iter().filter(|w| w["kind"] == "same" || w["kind"] == kind).map(|w| w["text"].as_str().unwrap()).collect()
    }

    /// Each reported difference says where it is in the updated story (for Go To), word by word
    /// how it changed, and where it lands: its page and its height on the page. The report lists
    /// the story's pages and says whether it still fits.
    #[test]
    fn report_entries_say_where_they_are() {
        let (dir, path, mut s, sid) = placed("where", "Revenue grew. Costs were flat. Prices held. Staff held.");
        layout_edit(&mut s, sid, "grew", "grew strongly");
        layout_edit(&mut s, sid, "were flat", "were steady");
        // A kept edit, a conflict, and a change from the file away from it.
        revise(&path, "Revenue grew. Costs fell. Prices held. Staff held firm.");
        let was = story(&s, sid).text;
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        let rep = &r["reports"][0];
        let text = story(&s, sid).text;
        let (changes, kept, conflicts) =
            (rep["changes"].as_array().unwrap(), rep["deviations"].as_array().unwrap(), rep["conflicts"].as_array().unwrap());
        assert!(!changes.is_empty() && !kept.is_empty() && !conflicts.is_empty(), "{rep}");
        // `was`: the same place in the story before the update, for Go To while reviewing.
        for e in changes {
            assert_eq!(covered_by(&was, e, "was"), e["before"], "{e}");
        }
        for e in kept.iter().chain(conflicts) {
            assert_eq!(covered_by(&was, e, "was"), e["layout"], "{e}");
        }
        for e in changes {
            assert_eq!(covered(&text, e), e["after"], "{e}");
            assert_eq!((rebuilt(e, "deleted"), rebuilt(e, "inserted")), (e["before"].as_str().unwrap().into(), e["after"].as_str().unwrap().into()));
        }
        for e in kept {
            assert_eq!(covered(&text, e), e["layout"], "{e}");
            assert_eq!(rebuilt(e, "inserted"), e["layout"].as_str().unwrap());
        }
        for e in conflicts {
            assert_eq!(covered(&text, e), e["file"], "{e}");
            assert_eq!(rebuilt(e, "inserted"), e["file"].as_str().unwrap());
        }
        for e in changes.iter().chain(kept).chain(conflicts) {
            assert_eq!((e["page"].as_u64(), e["pageName"].as_str(), e.get("overset")), (Some(0), Some("1"), None), "{e}");
            assert!((0.0..=1.0).contains(&e["y"].as_f64().unwrap()), "{e}");
        }
        assert_eq!(rep["pages"], json!([{"index": 0, "name": "1"}]));
        assert_eq!(rep["fit"], json!({"overset": false, "lines": 0}));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// When the updated story doesn't fit its frames, the report says how many lines don't, and a
    /// difference in the overset text says so: it is on no page.
    #[test]
    fn the_report_says_when_the_story_does_not_fit() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-fit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.docx");
        let long = (0..40).map(|i| format!("Sentence {i} fills the frame.")).collect::<Vec<_>>().join(" ");
        std::fs::write(&path, docx(&format!("<w:r><w:t>{long} End one.</w:t></w:r>"))).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [72, 72, 300, 160], "link": true})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        std::fs::write(&path, docx(&format!("<w:r><w:t>{long} End two.</w:t></w:r>"))).unwrap();
        let r = s.execute("links.update", &json!({"story": sid.0, "dryRun": true})).unwrap();
        let rep = &r["reports"][0];
        assert_eq!(rep["fit"]["overset"], true, "{rep}");
        assert!(rep["fit"]["lines"].as_u64().unwrap() > 5, "{rep}");
        let e = &rep["changes"][0];
        assert_eq!((e["after"].as_str(), e["overset"].as_bool(), e["page"].as_u64()), (Some("End two."), Some(true), Some(0)), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A changed spreadsheet cell says where it is in the cell (`at`) and on the page.
    #[test]
    fn cell_entries_say_where_they_are() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-cell-where-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prices.xlsx");
        std::fs::write(&path, workbook(&[&["Item", "Price"], &["Tea", "3.50"]])).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [72, 72, 300, 200], "link": true})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        std::fs::write(&path, workbook(&[&["Item", "Price"], &["Tea", "3.75"]])).unwrap();
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        let e = &r["reports"][0]["changes"][0];
        assert_eq!((e["cell"]["row"].as_u64(), e["cell"]["col"].as_u64()), (Some(1), Some(1)), "{e}");
        let cell = story(&s, sid).tables.values().next().unwrap().cell(1, 1).unwrap().text.text.clone();
        assert_eq!(covered(&cell, e), "3.75");
        assert_eq!((e["page"].as_u64(), e["pageName"].as_str()), (Some(0), Some("1")), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The stretches of `st` (or a cell's text) a Flow mark covers, joined where they touch.
    fn marked(st: &designcraft_doc::Story, mark: designcraft_doc::FlowMark) -> Vec<String> {
        let mut spans: Vec<std::ops::Range<usize>> = Vec::new();
        for (r, f) in st.runs() {
            if f.over.flow != Some(mark) {
                continue;
            }
            match spans.last_mut() {
                Some(last) if last.end == r.start => last.end = r.end,
                _ => spans.push(r),
            }
        }
        // Its text, without marker characters (note anchors and the like).
        let words = |r: std::ops::Range<usize>| st.text[r].chars().filter(|c| !('\u{E000}'..='\u{E1FF}').contains(c)).collect::<String>();
        spans.into_iter().map(|r| words(r).trim().to_string()).filter(|t| !t.is_empty()).collect()
    }

    /// An update marks what it brought in on the text, for the page's highlights: the file's
    /// changes, the layout's edits it kept, and the file's text that won a conflict.
    #[test]
    fn an_update_marks_what_it_brought_in_on_the_text() {
        use designcraft_doc::FlowMark;
        let (dir, path, mut s, sid) = placed("marks", "Revenue grew. Costs were flat. Prices held. Staff held.");
        layout_edit(&mut s, sid, "grew", "grew strongly");
        layout_edit(&mut s, sid, "were flat", "were steady");
        revise(&path, "Revenue grew. Costs fell. Prices held. Staff held firm.");
        s.execute("links.update", &json!({"story": sid.0})).unwrap();
        let st = story(&s, sid);
        assert_eq!(marked(&st, FlowMark::File), ["Staff held firm."]);
        assert_eq!(marked(&st, FlowMark::Kept), ["Revenue grew strongly."]);
        assert_eq!(marked(&st, FlowMark::Conflict), ["Costs fell."]);
        // The next update's marks replace these.
        revise(&path, "Revenue grew. Costs fell sharply. Prices held. Staff held firm.");
        s.execute("links.update", &json!({"story": sid.0})).unwrap();
        let st = story(&s, sid);
        assert_eq!(marked(&st, FlowMark::File), ["Costs fell sharply."], "{:?}", st.text);
        assert_eq!(marked(&st, FlowMark::Conflict), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The marks are the panel's, not the layout's: they don't count as edits made in the layout,
    /// and text typed or pasted among them isn't marked.
    #[test]
    fn marks_are_not_layout_edits_and_new_text_is_unmarked() {
        use designcraft_doc::FlowMark;
        let (dir, path, mut s, sid) = placed("marks-edits", "Revenue grew.");
        revise(&path, "Revenue grew in every region.");
        s.execute("links.update", &json!({"story": sid.0})).unwrap();
        assert_eq!(marked(&story(&s, sid), FlowMark::File), ["Revenue grew in every region."]);
        assert_eq!(text_rows(&mut s)[0]["edited"], false, "marks are not edits");
        let at = story(&s, sid).text.find("every").unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": at, "focus": at})).unwrap();
        s.execute("text.insert", &json!({"text": "X"})).unwrap();
        assert_eq!(story(&s, sid).format_after(at).over.flow, None, "typed text isn't marked");
        let word = story(&s, sid).text.find("region").unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": word, "focus": word + "region".len()})).unwrap();
        s.execute("edit.copy", &json!({})).unwrap();
        let end = story(&s, sid).text.len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
        s.execute("edit.paste", &json!({})).unwrap();
        let st = story(&s, sid);
        assert!(st.text.ends_with("region"), "{:?}", st.text);
        assert_eq!(st.format_after(end).over.flow, None, "pasted text isn't marked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Clear Highlights takes the marks off, but errors stay marked until they are fixed.
    #[test]
    fn clearing_highlights_keeps_errors_marked() {
        use designcraft_doc::FlowMark;
        let dir = std::env::temp_dir().join(format!("dc-text-link-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prices.xlsx");
        std::fs::write(&path, workbook(&[&["Item", "Price"], &["Tea", "3.50"], &["Cake", "2.00"]])).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [72, 72, 300, 200], "link": true})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        std::fs::write(
            &path,
            workbook_xml(
                r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="B1" t="inlineStr"><is><t>Price</t></is></c></row>
                <row r="2"><c r="A2" t="inlineStr"><is><t>Tea</t></is></c><c r="B2" t="e"><f>A9/0</f><v>#DIV/0!</v></c></row>
                <row r="3"><c r="A3" t="inlineStr"><is><t>Cake</t></is></c><c r="B3" t="inlineStr"><is><t>2.50</t></is></c></row>"#,
            ),
        )
        .unwrap();
        s.execute("links.update", &json!({"story": sid.0, "allowErrors": true})).unwrap();
        let cell = |s: &Session, r: usize, c: usize| story(s, sid).tables.values().next().unwrap().cell(r, c).unwrap().text.clone();
        assert_eq!(marked(&cell(&s, 1, 1), FlowMark::Error), ["#DIV/0!"]);
        assert_eq!(marked(&cell(&s, 2, 1), FlowMark::File), ["2.50"]);
        s.execute("links.clearHighlights", &json!({"story": sid.0})).unwrap();
        assert_eq!(marked(&cell(&s, 2, 1), FlowMark::File), Vec::<String>::new());
        assert_eq!(marked(&cell(&s, 1, 1), FlowMark::Error), ["#DIV/0!"], "errors stay marked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Before a file is placed, place.check says what it would bring in: the errors it shows and a
    /// preview (the first rows of a workbook, the first paragraphs of a text).
    #[test]
    fn place_check_shows_what_a_file_would_bring_in() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let b64 = crate::cmd::base64_encode(&workbook_xml(DIV0));
        let r = s.execute("place.check", &json!({"base64": b64, "name": "prices.xlsx"})).unwrap();
        assert_eq!(r["problems"].as_array().unwrap().len(), 1, "{r}");
        assert_eq!((r["problems"][0]["at"].as_str(), r["problems"][0]["blocks"].as_bool()), (Some("B2"), Some(true)));
        assert_eq!(r["preview"]["rows"], json!([["Item", "Price"], ["Tea", "#DIV/0!"]]));
        let b64 = crate::cmd::base64_encode(b"First paragraph.\nSecond one.");
        let r = s.execute("place.check", &json!({"base64": b64, "name": "notes.txt"})).unwrap();
        assert_eq!(r["preview"]["paragraphs"], json!(["First paragraph.", "Second one."]));
        assert_eq!(r["problems"], json!([]));
        assert_eq!(s.doc().unwrap().doc.stories.len(), 0, "nothing placed");
    }

    /// Flow's marks in a linked spreadsheet's cells don't count as edits made in the layout either.
    #[test]
    fn marks_in_cells_are_not_layout_edits() {
        use designcraft_doc::FlowMark;
        let dir = std::env::temp_dir().join(format!("dc-text-link-cell-marks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prices.xlsx");
        std::fs::write(&path, workbook(&[&["Item", "Price"], &["Tea", "3.50"]])).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [72, 72, 300, 200], "link": true})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        std::fs::write(&path, workbook(&[&["Item", "Price"], &["Tea", "3.75"]])).unwrap();
        s.execute("links.update", &json!({"story": sid.0})).unwrap();
        let cell = story(&s, sid).tables.values().next().unwrap().cell(1, 1).unwrap().text.clone();
        assert_eq!(marked(&cell, FlowMark::File), ["3.75"]);
        assert_eq!(text_rows(&mut s)[0]["edited"], false, "the cell's mark is not an edit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `text: false` updates graphics only: the app reviews linked text files in Flow first.
    #[test]
    fn update_can_leave_linked_text_for_review() {
        let (dir, path, mut s, sid) = placed("graphics-only", "Revenue grew.");
        revise(&path, "Revenue grew in every region.");
        let r = s.execute("links.update", &json!({"text": false})).unwrap();
        assert_eq!(r["updated"], 0);
        assert_eq!(story(&s, sid).text, "Head\nRevenue grew.");
        assert_eq!(text_rows(&mut s)[0]["status"], "modified");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A dry run returns what the update would do, with nothing changed: the story, its link, the
    /// undo history and the file's status stay as they were.
    #[test]
    fn a_dry_run_reports_the_update_and_changes_nothing() {
        let (dir, path, mut s, sid) = placed("dry-run", "Revenue grew. Costs were flat.");
        layout_edit(&mut s, sid, "flat", "steady");
        revise(&path, "Revenue grew in every region. Costs were flat.");
        let doc = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let dry = s.execute("links.update", &json!({"story": sid.0, "dryRun": true})).unwrap();
        assert!(std::sync::Arc::ptr_eq(&doc, &s.doc().unwrap().doc), "the document is untouched");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "nothing to undo");
        assert_eq!(text_rows(&mut s)[0]["status"], "modified");
        assert_eq!(dry["reports"][0]["changes"][0]["after"], "Revenue grew in every region.", "{dry}");
        let real = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        assert_eq!(dry, real, "the dry run shows exactly what the update does");
        assert_eq!(story(&s, sid).text, "Head\nRevenue grew in every region. Costs were steady.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Each update is recorded on its link (when, its report, the errors the file showed) and the
    /// record is saved with the document; a dry run records nothing.
    #[test]
    fn an_update_is_recorded_on_its_link_and_saved() {
        let (dir, path, mut s, sid) = placed("record", "Revenue grew. Costs were flat.");
        revise(&path, "Revenue grew in every region. Costs were flat.");
        s.execute("links.update", &json!({"story": sid.0, "dryRun": true})).unwrap();
        assert_eq!(text_rows(&mut s)[0]["lastUpdate"], Value::Null);
        let r = s.execute("links.update", &json!({"story": sid.0})).unwrap();
        let rec = text_rows(&mut s)[0]["lastUpdate"].clone();
        assert!(rec["at"].as_i64().unwrap() > 0, "{rec}");
        assert_eq!(rec["report"], r["reports"][0]);
        assert_eq!(rec["errors"], json!([]));
        let saved = dir.join("doc.designcraft");
        s.execute("file.save", &json!({"path": saved.to_string_lossy()})).unwrap();
        let mut t = Session::new();
        t.execute("file.open", &json!({"path": saved.to_string_lossy()})).unwrap();
        assert_eq!(text_rows(&mut t)[0]["lastUpdate"], rec);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The 0% gate: a merged story whose text differs from the file's by anything unreported is
    /// refused.
    #[test]
    fn the_exactness_gate_refuses_an_unreported_difference() {
        let story = |t: &str| designcraft_doc::Story::with_text(StoryId(1), t, designcraft_doc::ParaFormat::default());
        let (base, mine, theirs) = (story("One. Two."), story("One. Layout."), story("One, revised. Two."));
        let mut m = designcraft_flow::merge_story(&base, &mine, &theirs).unwrap();
        assert!(super::super::place_text::verified(&theirs, &mine, &m));
        m.story.text.push('!');
        assert!(!super::super::place_text::verified(&theirs, &mine, &m));
    }

    /// A workbook whose first sheet holds `rows` as inline strings.
    fn xlsx(rows: &[&[&str]]) -> Vec<u8> {
        use std::io::Write;
        let rows: String = rows
            .iter()
            .map(|row| format!("<row>{}</row>", row.iter().map(|v| format!(r#"<c t="inlineStr"><is><t>{v}</t></is></c>"#)).collect::<String>()))
            .collect();
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("xl/worksheets/sheet1.xml", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(format!("<worksheet><sheetData>{rows}</sheetData></worksheet>").as_bytes()).unwrap();
        w.finish().unwrap().into_inner()
    }

    /// A cell edited in a linked spreadsheet is an edit made in the layout: the Links panel shows
    /// it, and Update warns that it replaces it (it used to be replaced silently).
    #[test]
    fn edits_in_linked_table_cells_are_flagged_and_the_update_warns() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-xlsx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prices.xlsx");
        std::fs::write(&path, xlsx(&[&["Item", "Price"], &["Tea", "3.50"]])).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [72, 72, 300, 200], "link": true})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        let table = *story(&s, sid).tables.keys().next().expect("the sheet is placed as a table");
        assert_eq!(text_rows(&mut s)[0]["edited"], false);

        s.execute("table.select", &json!({"story": sid.0, "table": table, "rows": [1, 1], "cols": [1, 1]})).unwrap();
        s.execute("table.setCell", &json!({"text": "3.75"})).unwrap();
        assert_eq!(text_rows(&mut s)[0]["edited"], true, "a cell edited in the layout");

        std::fs::write(&path, xlsx(&[&["Item", "Price"], &["Tea", "3.50"], &["Cake", "12.00"]])).unwrap();
        // Replacing warns that the layout's edits go (merging, Flow's default, keeps them).
        let r = s.execute("links.update", &json!({"story": sid.0, "mode": "replace"})).unwrap();
        assert_eq!(r["updated"], 1);
        let w = r["warnings"][0].as_str().unwrap_or_default();
        assert!(w.contains("prices.xlsx") && w.contains("replaced"), "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Linked stories saved by earlier builds keep their recorded hash: a story without tables
    /// must hash as it always did, or every one of them would show as edited.
    #[test]
    fn story_hash_of_a_story_without_tables_is_unchanged() {
        let st = designcraft_doc::Story::with_text(StoryId(1), "Head\nBody", designcraft_doc::ParaFormat::default());
        assert_eq!(super::story_hash(&st), "5ffd958c8ee493e4");
    }
}

#[cfg(unix)]
#[cfg(test)]
mod text_link_read_tests {
    use std::time::Duration;

    use designcraft_doc::TextFileLink;

    use super::{MAX_TEXT_LINK_BYTES, read_text, text_status};

    /// `f` on another thread: a call that blocks fails the test instead of hanging it.
    fn promptly<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        // A timeout means the call blocked.
        rx.recv_timeout(Duration::from_secs(10)).unwrap()
    }

    fn status_of(path: &std::path::Path, size: u64) -> &'static str {
        let l = TextFileLink { path: path.to_string_lossy().to_string(), size, hash: "0".into(), ..Default::default() };
        promptly(move || text_status(&l, &mut Default::default()))
    }

    fn read(path: &std::path::Path) -> Result<usize, String> {
        let p = path.to_string_lossy().to_string();
        promptly(move || read_text(&p).map(|(b, _)| b.len()).map_err(|e| e.to_string()))
    }

    #[test]
    fn links_to_fifos_devices_folders_and_huge_files_are_not_read() {
        let dir = std::env::temp_dir().join(format!("dc-text-link-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A FIFO with no writer: reading it would block for ever.
        let fifo = dir.join("pipe.docx");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        assert_eq!(status_of(&fifo, 0), "missing");
        assert!(read(&fifo).is_err());
        // A device that never ends, recorded with size 0 (as its metadata reports).
        let zero = std::path::Path::new("/dev/zero");
        assert_eq!(status_of(zero, 0), "missing");
        assert!(read(zero).is_err());
        // A folder.
        assert_eq!(status_of(&dir, 0), "missing");
        assert!(read(&dir).is_err());
        // Over the limit (sparse, so cheap): not read; the same size is reported as too large
        // to check rather than OK, another size as modified.
        let big = dir.join("big.txt");
        std::fs::File::create(&big).unwrap().set_len(MAX_TEXT_LINK_BYTES + 1).unwrap();
        assert_eq!(status_of(&big, MAX_TEXT_LINK_BYTES + 1), "tooLarge");
        assert_eq!(status_of(&big, 10), "modified");
        assert!(read(&big).unwrap_err().contains("larger than"));
        assert!(!super::checkable_size(MAX_TEXT_LINK_BYTES as usize + 1), "placing it doesn't link it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
