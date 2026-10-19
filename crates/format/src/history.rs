//! A document's history, kept in its `.designcraft` file: earlier versions of the document,
//! each kept whole or as the changes from the version it was made from.
//!
//! ```text
//! meta.json                 "history": true when the file has versions
//! history/versions.json     [{id, parent, author, ts, label, kind, full, size, format, assets}]
//! history/<id>.json         the version's document.json (full) or its changes from `parent`
//! history/assets/<n>.<ext>  bytes a version's assets had that the current document's don't
//!                           (removed since, or a link updated since)
//! ```
//!
//! Changes are an ordered list of `{op: set | insert | remove, path, value}` on the JSON of the
//! document, each path a JSON Pointer into the JSON as the changes before it left it. Arrays
//! whose elements all have distinct `id`s are matched by id; others by position, past the
//! elements both start and end with. One version in ten along a line of versions is kept whole,
//! so rebuilding one applies at most nine lists of changes. Older builds open the file and leave
//! its history out when they save.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Seek, Write};
use std::sync::Arc;

use designcraft_doc::{AssetId, Document};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use crate::FormatError;

/// One version in this many along a line of versions is kept whole.
const FULL_EVERY: usize = 10;

/// Why a version was kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VersionKind {
    /// Saving a document with changes.
    #[default]
    Save,
    /// File › Save a Version.
    Manual,
    /// Kept before a Flow update (`links.update`).
    LinkUpdate,
    /// An earlier version restored.
    Restore,
    /// A kind from a newer build.
    #[serde(other)]
    Other,
}

/// What is known about a version without rebuilding it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub id: u32,
    /// The version it was made from, which its changes are from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    #[serde(default)]
    pub author: String,
    /// Unix seconds, UTC.
    #[serde(default)]
    pub ts: i64,
    #[serde(default)]
    pub label: String,
    pub kind: VersionKind,
    /// Kept whole rather than as changes from `parent`.
    pub full: bool,
    /// Bytes of its `history/<id>.json`.
    pub size: u64,
    /// The format version it was kept in (rebuilding it applies the upgrades since).
    pub format: u32,
    /// A Flow update's reports (`LinkUpdate`): what it changed in each linked story.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reports: Option<Value>,
}

/// A version to add.
#[derive(Clone, Debug, Default)]
pub struct NewVersion {
    pub author: String,
    /// Unix seconds, UTC.
    pub ts: i64,
    pub label: String,
    pub kind: VersionKind,
    /// The version the document was made from; `None`: the latest.
    pub parent: Option<u32>,
    pub reports: Option<Value>,
}

#[derive(Clone, Debug)]
struct Stored {
    info: VersionInfo,
    /// Its `history/<id>.json`.
    body: Arc<Vec<u8>>,
    /// The bytes of its assets, with the file name extension of their type.
    assets: BTreeMap<AssetId, (Arc<Vec<u8>>, &'static str)>,
}

/// The versions of a document, oldest first (by id).
#[derive(Clone, Debug, Default)]
pub struct History {
    versions: Vec<Stored>,
}

impl History {
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    /// The versions, oldest first.
    pub fn versions(&self) -> impl Iterator<Item = &VersionInfo> {
        self.versions.iter().map(|s| &s.info)
    }

    fn get(&self, id: u32) -> Result<&Stored, FormatError> {
        let at = self.versions.binary_search_by_key(&id, |s| s.info.id).map_err(|_| FormatError::History(format!("no version {id}")))?;
        self.versions.get(at).ok_or_else(|| FormatError::History(format!("no version {id}")))
    }

    /// Add `doc` as a version, as saving it would write it; returns its id. Kept as its changes
    /// from the version it was made from, or whole: every tenth along a line of versions, and
    /// when that version can't be rebuilt.
    pub fn commit(&mut self, doc: &Document, v: NewVersion) -> Result<u32, FormatError> {
        if let Some(p) = v.parent {
            self.get(p)?;
        }
        let compact = crate::saveable(doc)?;
        let value = serde_json::to_value(&compact).map_err(|e| FormatError::Io(e.to_string()))?;
        let parent = v.parent.or_else(|| self.versions.last().map(|s| s.info.id));
        let ops = parent.filter(|p| self.depth(*p).is_some_and(|d| d + 1 < FULL_EVERY)).and_then(|p| self.value(p).ok()).map(|base| {
            let mut ops = Vec::new();
            diff(&base, &value, "", &mut ops);
            ops
        });
        let body = match &ops {
            Some(ops) => serde_json::to_vec(ops),
            None => serde_json::to_vec(&value),
        }
        .map_err(|e| FormatError::Io(e.to_string()))?;
        let id = match self.versions.last() {
            Some(s) => s.info.id.checked_add(1).ok_or_else(|| FormatError::History("no version ids are left".into()))?,
            None => 1,
        };
        let info = VersionInfo {
            id,
            parent,
            author: v.author,
            ts: v.ts,
            label: v.label,
            kind: v.kind,
            full: ops.is_none(),
            size: body.len() as u64,
            format: crate::VERSION,
            reports: v.reports,
        };
        let assets = compact.assets.values().filter(|a| !a.data.is_empty()).map(|a| (a.id, (a.data.clone(), crate::ext_for(&a.mime)))).collect();
        self.versions.push(Stored { info, body: Arc::new(body), assets });
        Ok(id)
    }

    /// Whether `doc` is what the latest version holds, as a file would hold it.
    pub fn is_latest(&self, doc: &Document) -> bool {
        let Some(last) = self.versions.last() else { return false };
        let (Ok(compact), Ok(v)) = (crate::saveable(doc), self.value(last.info.id)) else { return false };
        let same_bytes = compact.assets.values().filter(|a| !a.data.is_empty()).all(|a| last.assets.get(&a.id).is_some_and(|(d, _)| *d == a.data));
        same_bytes && serde_json::to_value(&compact).is_ok_and(|c| c == v)
    }

    /// Drop the oldest versions kept on save (`Save`; every kind unless `keep_others`) past the
    /// newest `keep_last` of them; returns how many went. The versions made from a dropped one
    /// are kept as they were: as changes from its parent, or whole when it was whole (so no line
    /// of changes grows past nine).
    pub fn prune(&mut self, keep_last: usize, keep_others: bool) -> usize {
        let candidates: Vec<u32> = self.versions.iter().filter(|s| !keep_others || s.info.kind == VersionKind::Save).map(|s| s.info.id).collect();
        let n = candidates.len().saturating_sub(keep_last);
        for id in candidates.into_iter().take(n) {
            self.drop_version(id);
        }
        n
    }

    fn drop_version(&mut self, id: u32) {
        let Ok(gone) = self.get(id).cloned() else { return };
        // The versions made from it, rebuilt while it is still there (whole ones need nothing).
        let children: Vec<(u32, Option<Value>)> = self
            .versions
            .iter()
            .filter(|s| s.info.parent == Some(id))
            .map(|s| (s.info.id, if s.info.full { None } else { self.value(s.info.id).ok() }))
            .collect();
        self.versions.retain(|s| s.info.id != id);
        for (child, value) in children {
            let body = value.map(|v| {
                let base = if gone.info.full { None } else { gone.info.parent.and_then(|p| self.value(p).ok()) };
                match base {
                    Some(base) => {
                        let mut ops = Vec::new();
                        diff(&base, &v, "", &mut ops);
                        (serde_json::to_vec(&ops), false)
                    }
                    None => (serde_json::to_vec(&v), true),
                }
            });
            let Ok(at) = self.versions.binary_search_by_key(&child, |s| s.info.id) else { continue };
            let Some(s) = self.versions.get_mut(at) else { continue };
            s.info.parent = gone.info.parent;
            if let Some((Ok(body), full)) = body {
                s.info.full = full;
                s.info.size = body.len() as u64;
                s.body = Arc::new(body);
            }
        }
    }

    /// How many lists of changes rebuilding `id` applies; `None` when it can't be rebuilt.
    fn depth(&self, id: u32) -> Option<usize> {
        let mut at = self.get(id).ok()?;
        for n in 0..=self.versions.len() {
            if at.info.full {
                return Some(n);
            }
            at = self.get(at.info.parent?).ok()?;
        }
        None
    }

    /// The JSON of version `id`: the nearest version kept whole that it was made from, with the
    /// changes since applied.
    fn value(&self, id: u32) -> Result<Value, FormatError> {
        let bad = |id: u32, e: String| FormatError::History(format!("version {id}: {e}"));
        let mut at = self.get(id)?;
        let mut line = vec![at];
        while !at.info.full {
            if line.len() > self.versions.len() {
                return Err(bad(id, "its versions are made from each other".into()));
            }
            at = self.get(at.info.parent.ok_or_else(|| bad(at.info.id, "changes from no version".into()))?)?;
            line.push(at);
        }
        let mut v = Value::Null;
        for s in line.iter().rev() {
            if s.info.full {
                v = serde_json::from_slice(&s.body).map_err(|e| bad(s.info.id, e.to_string()))?;
                continue;
            }
            let ops: Vec<Op> = serde_json::from_slice(&s.body).map_err(|e| bad(s.info.id, e.to_string()))?;
            for op in ops {
                apply(&mut v, op).map_err(|e| bad(s.info.id, e))?;
            }
        }
        Ok(v)
    }

    /// Version `id`, as opening the file saved then would give it.
    pub fn document(&self, id: u32) -> Result<Document, FormatError> {
        let s = self.get(id)?;
        let mut v = self.value(id)?;
        crate::upgrade_json(&mut v);
        let mut doc: Document = serde_json::from_value(v).map_err(|e| FormatError::History(format!("version {id}: {e}")))?;
        for (aid, a) in &mut doc.assets {
            if let Some((data, _)) = s.assets.get(aid) {
                Arc::make_mut(a).data = data.clone();
            }
        }
        crate::finish(&mut doc, s.info.format).map_err(|e| FormatError::History(format!("version {id}: {e}")))?;
        Ok(doc)
    }

    /// Write `history/`; `doc` is the document as written (its `assets/` entries).
    pub(crate) fn write<W: Write + Seek>(&self, z: &mut zip::ZipWriter<W>, doc: &Document, large_entry: usize) -> Result<(), FormatError> {
        let io = |e: zip::result::ZipError| FormatError::Io(e.to_string());
        let write = |e: std::io::Error| FormatError::Io(e.to_string());
        let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        // Each buffer is written once, however many versions have it.
        let mut written: HashMap<*const Vec<u8>, String> = HashMap::new();
        let mut records = Vec::with_capacity(self.versions.len());
        for s in &self.versions {
            z.start_file(format!("history/{}.json", s.info.id), deflate).map_err(io)?;
            z.write_all(&s.body).map_err(write)?;
            let mut assets = BTreeMap::new();
            for (id, (data, ext)) in &s.assets {
                let name = match (doc.assets.get(id), written.get(&Arc::as_ptr(data))) {
                    (Some(a), _) if a.data == *data => format!("assets/{}.{}", id.0, crate::ext_for(&a.mime)),
                    (_, Some(name)) => name.clone(),
                    _ => {
                        let name = format!("history/assets/{}.{ext}", written.len());
                        let method =
                            if matches!(*ext, "png" | "jpg" | "gif" | "webp") { CompressionMethod::Stored } else { CompressionMethod::Deflated };
                        z.start_file(name.as_str(), SimpleFileOptions::default().compression_method(method).large_file(data.len() >= large_entry))
                            .map_err(io)?;
                        z.write_all(data).map_err(write)?;
                        written.insert(Arc::as_ptr(data), name.clone());
                        name
                    }
                };
                assets.insert(id.0, name);
            }
            records.push(Record { info: s.info.clone(), assets });
        }
        z.start_file("history/versions.json", deflate).map_err(io)?;
        z.write_all(&serde_json::to_vec(&records).map_err(|e| FormatError::Io(e.to_string()))?).map_err(write)?;
        Ok(())
    }

    /// Read `history/`; `doc` holds the bytes of the document's own `assets/` entries.
    pub(crate) fn read(z: &mut zip::ZipArchive<Cursor<&[u8]>>, doc: &Document, left: &mut u64) -> Result<History, FormatError> {
        let bad = FormatError::History;
        let list = crate::read_entry(z, "history/versions.json", crate::MAX_JSON_BYTES, left)?
            .ok_or_else(|| bad("history/versions.json is missing".into()))?;
        let records: Vec<Record> = serde_json::from_slice(&list).map_err(|e| bad(format!("history/versions.json: {e}")))?;
        let mut read: HashMap<String, Arc<Vec<u8>>> = doc
            .assets
            .values()
            .filter(|a| !a.data.is_empty())
            .map(|a| (format!("assets/{}.{}", a.id.0, crate::ext_for(&a.mime)), a.data.clone()))
            .collect();
        let mut versions: Vec<Stored> = Vec::new();
        for r in records {
            let id = r.info.id;
            let body = crate::read_entry(z, &format!("history/{id}.json"), crate::MAX_JSON_BYTES, left)?
                .ok_or_else(|| bad(format!("history/{id}.json is missing")))?;
            let mut assets = BTreeMap::new();
            for (aid, name) in r.assets {
                if !(name.starts_with("assets/") || name.starts_with("history/assets/")) {
                    return Err(bad(format!("version {id}: {name} isn't an asset")));
                }
                let data = match read.get(&name) {
                    Some(d) => d.clone(),
                    None => {
                        let d =
                            Arc::new(crate::read_entry(z, &name, crate::MAX_ASSET_BYTES, left)?.ok_or_else(|| bad(format!("{name} is missing")))?);
                        read.insert(name.clone(), d.clone());
                        d
                    }
                };
                let ext = name
                    .rsplit('.')
                    .next()
                    .and_then(|e| ["png", "jpg", "gif", "webp", "tif", "pdf", "svg"].into_iter().find(|k| *k == e))
                    .unwrap_or("bin");
                assets.insert(AssetId(aid), (data, ext));
            }
            versions.push(Stored { info: VersionInfo { size: body.len() as u64, ..r.info }, body: Arc::new(body), assets });
        }
        versions.sort_by_key(|s| s.info.id);
        if let Some(w) = versions.windows(2).find(|w| w.first().map(|s| s.info.id) == w.get(1).map(|s| s.info.id)) {
            return Err(bad(format!("version {} is listed twice", w.first().map_or(0, |s| s.info.id))));
        }
        Ok(History { versions })
    }
}

/// A version's line in `history/versions.json`.
#[derive(Serialize, Deserialize)]
struct Record {
    #[serde(flatten)]
    info: VersionInfo,
    /// Asset id → the entry holding the bytes it had.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    assets: BTreeMap<u64, String>,
}

/// One change to the JSON of a document.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Op {
    /// Set an object's member, or replace an array's element.
    Set { path: String, value: Value },
    /// Insert an array element before the one at `path` (or after the last).
    Insert { path: String, value: Value },
    /// Remove an object's member or an array's element.
    Remove { path: String },
}

/// The changes that turn `a` into `b`, appended to `out`; `path` is where `a` sits.
fn diff(a: &Value, b: &Value, path: &str, out: &mut Vec<Op>) {
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, va) in x {
                match y.get(k) {
                    Some(vb) => diff(va, vb, &child(path, k), out),
                    None => out.push(Op::Remove { path: child(path, k) }),
                }
            }
            for (k, vb) in y.iter().filter(|(k, _)| !x.contains_key(*k)) {
                out.push(Op::Set { path: child(path, k), value: vb.clone() });
            }
        }
        (Value::Array(x), Value::Array(y)) => diff_array(x, y, path, out),
        _ => out.push(Op::Set { path: path.into(), value: b.clone() }),
    }
}

fn diff_array(x: &[Value], y: &[Value], path: &str, out: &mut Vec<Op>) {
    let at = |i: usize| child(path, &i.to_string());
    if let (Some(kx), Some(ky)) = (ids(x), ids(y)) {
        // Elements with the same id stay (and change in place); the others are removed or
        // inserted. `k` is where the next element of `x` sits after the changes so far.
        let (mut i, mut j, mut k) = (0, 0, 0);
        for (mi, mj) in same_order(&kx, &ky).into_iter().chain([(x.len(), y.len())]) {
            for _ in i..mi {
                out.push(Op::Remove { path: at(k) });
            }
            for v in y.get(j..mj).unwrap_or_default() {
                out.push(Op::Insert { path: at(k), value: v.clone() });
                k += 1;
            }
            if let (Some(a), Some(b)) = (x.get(mi), y.get(mj)) {
                diff(a, b, &at(k), out);
                k += 1;
            }
            (i, j) = (mi + 1, mj + 1);
        }
        return;
    }
    // By position, past the elements both start and end with.
    let start = x.iter().zip(y).take_while(|(a, b)| a == b).count();
    let (x, y) = (x.get(start..).unwrap_or_default(), y.get(start..).unwrap_or_default());
    let end = x.iter().rev().zip(y.iter().rev()).take_while(|(a, b)| a == b).count();
    let (x, y) = (x.get(..x.len() - end).unwrap_or_default(), y.get(..y.len() - end).unwrap_or_default());
    for (n, (a, b)) in x.iter().zip(y).enumerate() {
        diff(a, b, &at(start + n), out);
    }
    let both = x.len().min(y.len());
    for _ in both..x.len() {
        out.push(Op::Remove { path: at(start + both) });
    }
    for (n, v) in y.iter().enumerate().skip(both) {
        out.push(Op::Insert { path: at(start + n), value: v.clone() });
    }
}

/// Each element's `id`, when every element is an object with one and no two share it.
fn ids(v: &[Value]) -> Option<Vec<String>> {
    let mut seen = HashSet::new();
    v.iter().map(|e| e.get("id").map(Value::to_string).filter(|k| seen.insert(k.clone()))).collect()
}

/// The most elements of `x` and `y` (whose keys are distinct in each) found in both in the same
/// order, as index pairs: the longest increasing run of `x`'s indices in `y`'s order.
fn same_order(x: &[String], y: &[String]) -> Vec<(usize, usize)> {
    let at: HashMap<&str, usize> = x.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    let pairs: Vec<(usize, usize)> = y.iter().enumerate().filter_map(|(j, k)| Some((*at.get(k.as_str())?, j))).collect();
    // tails[l]: the pair ending the best run of length l + 1 found so far (the smallest end).
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; pairs.len()];
    for (n, &(i, _)) in pairs.iter().enumerate() {
        let len = tails.partition_point(|&t| pairs.get(t).is_some_and(|p| p.0 < i));
        if let Some(p) = prev.get_mut(n) {
            *p = len.checked_sub(1).and_then(|l| tails.get(l).copied());
        }
        match tails.get_mut(len) {
            Some(t) => *t = n,
            None => tails.push(n),
        }
    }
    let mut out = Vec::new();
    let mut cur = tails.last().copied();
    while let Some(n) = cur {
        out.extend(pairs.get(n).copied());
        cur = prev.get(n).copied().flatten();
    }
    out.reverse();
    out
}

/// The JSON Pointer of member `key` of the value at `path`.
fn child(path: &str, key: &str) -> String {
    let mut p = String::with_capacity(path.len() + key.len() + 1);
    p.push_str(path);
    p.push('/');
    for c in key.chars() {
        match c {
            '~' => p.push_str("~0"),
            '/' => p.push_str("~1"),
            c => p.push(c),
        }
    }
    p
}

fn apply(doc: &mut Value, op: Op) -> Result<(), String> {
    let path = match &op {
        Op::Set { path, .. } | Op::Insert { path, .. } | Op::Remove { path } => path.clone(),
    };
    if path.is_empty() {
        return match op {
            Op::Set { value, .. } => {
                *doc = value;
                Ok(())
            }
            _ => Err("a change to the whole document".into()),
        };
    }
    let missing = || format!("{path} isn't there");
    let cut = path.rfind('/').ok_or_else(|| format!("{path} isn't a path"))?;
    let (up, last) = path.split_at(cut);
    let key = last.get(1..).unwrap_or_default().replace("~1", "/").replace("~0", "~");
    let index = || key.parse::<usize>().map_err(|_| format!("{path}: {key} isn't an index"));
    match (op, doc.pointer_mut(up).ok_or_else(missing)?) {
        (Op::Set { value, .. }, Value::Array(a)) => *a.get_mut(index()?).ok_or_else(missing)? = value,
        (Op::Insert { value, .. }, Value::Array(a)) => {
            let i = index()?;
            if i > a.len() {
                return Err(missing());
            }
            a.insert(i, value);
        }
        (Op::Remove { .. }, Value::Array(a)) => {
            let i = index()?;
            if i >= a.len() {
                return Err(missing());
            }
            a.remove(i);
        }
        (Op::Set { value, .. }, Value::Object(m)) => {
            m.insert(key, value);
        }
        (Op::Remove { .. }, Value::Object(m)) => {
            m.remove(&key).ok_or_else(missing)?;
        }
        _ => return Err(format!("{path} can't change that way")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{load, load_with_history, save, save_with_history};
    use designcraft_doc::build::NewDocument;
    use designcraft_doc::geom::{Affine, Rect};
    use designcraft_doc::{Asset, Content, ParaFormat, SpreadRef, StoryId};
    use proptest::prelude::*;
    use serde_json::{Value, json};
    use std::io::{Cursor, Read, Write};

    fn png(d: &mut Document, bytes: &[u8]) -> AssetId {
        let id = AssetId(d.alloc());
        d.assets.insert(
            id,
            Arc::new(Asset {
                id,
                name: "x.png".into(),
                mime: "image/png".into(),
                data: Arc::new(bytes.to_vec()),
                pixels: Some((1, 1)),
                ..Default::default()
            }),
        );
        id
    }

    fn place(d: &mut Document, asset: AssetId) -> designcraft_doc::ItemId {
        use designcraft_doc::geom::shapes;
        use designcraft_doc::{Graphic, Item, ItemId, Shape};
        let id = ItemId(d.alloc());
        let mut it = Item::new(id, d.default_layer(), Shape::Rectangle, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)));
        it.content = Content::Graphic(Graphic {
            asset,
            size: (1.0, 1.0),
            xf: Affine::IDENTITY,
            clip: None,
            auto_fit: Default::default(),
            fit_align: 4,
            crop: [0.0; 4],
            wrap: Default::default(),
            fill: Default::default(),
        });
        d.insert_item(SpreadRef::Doc(0), it, None).unwrap();
        id
    }

    fn story(d: &Document) -> StoryId {
        *d.stories.keys().next().unwrap()
    }

    /// Version `n`'s edit: a word typed every time, and now and then a frame added, moved,
    /// brought to the front or deleted, an image placed and its link updated twice (new bytes,
    /// same asset), another placed and deleted, a word added to the dictionary and the title
    /// changed.
    fn edit(d: &mut Document, n: u32, images: &mut Vec<(AssetId, designcraft_doc::ItemId)>) {
        let sid = story(d);
        let st = Arc::make_mut(d.stories.get_mut(&sid).unwrap());
        st.insert(st.text.len(), &format!(" word{n}"));
        let lid = d.default_layer();
        if n.is_multiple_of(3) {
            d.add_text_frame(SpreadRef::Doc(0), Rect::new(f64::from(n), 300.0, 100.0, 40.0), lid, &format!("frame {n}"), ParaFormat::default())
                .unwrap();
        }
        let items = |d: &Document| d.spreads[0].items.iter().map(|it| it.id).collect::<Vec<_>>();
        if n.is_multiple_of(4) {
            let it = d.item_mut(items(d)[0]).unwrap();
            it.xf = Affine::translate((5.0, 0.0)) * it.xf;
        }
        if n == 10 {
            let sp = Arc::make_mut(&mut d.spreads[0]);
            let first = sp.items.remove(0);
            sp.items.push(first);
        }
        if n == 8 {
            d.remove_item(*items(d).last().unwrap()).unwrap();
        }
        match n {
            5 | 16 => {
                let a = png(d, if n == 5 { b"one" } else { b"bee" });
                let it = place(d, a);
                images.push((a, it));
            }
            9 | 13 => {
                Arc::make_mut(d.assets.get_mut(&images[0].0).unwrap()).data = Arc::new(if n == 9 { b"two".to_vec() } else { b"three".to_vec() })
            }
            18 => {
                d.remove_item(images[1].1).unwrap();
            }
            15 => d.user_words.push("Craftwork".into()),
            20 => d.title = "Renamed".into(),
            _ => {}
        }
    }

    fn start() -> Document {
        let mut d = Document::new(&NewDocument { pages: 2, ..Default::default() });
        let lid = d.default_layer();
        d.add_text_frame(SpreadRef::Doc(0), Rect::new(36.0, 36.0, 300.0, 200.0), lid, "Start.", ParaFormat::default()).unwrap();
        d
    }

    /// 25 versions of an edited document, and each version's plain save.
    fn versions() -> (Document, History, Vec<Vec<u8>>) {
        let mut d = start();
        let mut h = History::default();
        let mut saved = Vec::new();
        let mut images = Vec::new();
        for n in 1..=25 {
            edit(&mut d, n, &mut images);
            let id = h.commit(&d, NewVersion { label: format!("v{n}"), ts: i64::from(n), author: "Chris".into(), ..Default::default() }).unwrap();
            assert_eq!(id, n);
            saved.push(save(&d).unwrap());
        }
        (d, h, saved)
    }

    fn image_bytes(d: &Document) -> Vec<Vec<u8>> {
        d.assets.values().map(|a| a.data.to_vec()).collect()
    }

    #[test]
    fn twenty_five_versions_reopen_and_restore_as_they_were_saved() {
        let (d, h, saved) = versions();
        let file = save_with_history(&d, &h, None).unwrap();
        let (doc, back) = load_with_history(&file).unwrap();
        let back = back.unwrap();
        assert_eq!(doc, load(&save(&d).unwrap()).unwrap());
        assert_eq!(back.versions().map(|v| v.id).collect::<Vec<_>>(), (1..=25).collect::<Vec<_>>());
        assert_eq!(back.versions().map(|v| v.label.as_str()).nth(6), Some("v7"));
        // Version 7, the plan's check: the document as saved then.
        assert_eq!(back.document(7).unwrap(), load(&saved[6]).unwrap());
        for (n, bytes) in saved.iter().enumerate() {
            assert_eq!(back.document(n as u32 + 1).unwrap(), load(bytes).unwrap(), "version {}", n + 1);
        }
        // One version in ten whole.
        assert_eq!(back.versions().filter(|v| v.full).map(|v| v.id).collect::<Vec<_>>(), [1, 11, 21]);
        // The images' bytes as each version had them: one updated since, one deleted since.
        assert_eq!(image_bytes(&doc), [b"three".to_vec()]);
        assert_eq!(image_bytes(&back.document(7).unwrap()), [b"one".to_vec()]);
        assert_eq!(image_bytes(&back.document(10).unwrap()), [b"two".to_vec()]);
        assert_eq!(image_bytes(&back.document(17).unwrap()), [b"three".to_vec(), b"bee".to_vec()]);
        // Each distinct file once: the document's in `assets/`, the others in `history/assets/`.
        let names: Vec<String> = entries(&file).into_iter().map(|(n, _)| n).filter(|n| n.contains("assets/")).collect();
        assert_eq!(names.len(), 4, "{names:?}");
        assert_eq!(names.iter().filter(|n| n.starts_with("history/assets/")).count(), 3, "{names:?}");
        // Saved again, it's the same history.
        let again = save_with_history(&doc, &back, None).unwrap();
        let (_, back2) = load_with_history(&again).unwrap();
        assert_eq!(back2.unwrap().document(7).unwrap(), load(&saved[6]).unwrap());
    }

    fn entries(file: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut z = zip::ZipArchive::new(Cursor::new(file)).unwrap();
        (0..z.len())
            .map(|i| {
                let mut f = z.by_index(i).unwrap();
                let mut v = Vec::new();
                f.read_to_end(&mut v).unwrap();
                (f.name().to_string(), v)
            })
            .collect()
    }

    fn rezip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, data) in entries {
            z.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        buf.into_inner()
    }

    fn with_entry(file: &[u8], name: &str, data: &[u8]) -> Vec<u8> {
        let mut e = entries(file);
        e.iter_mut().find(|(n, _)| n == name).unwrap().1 = data.to_vec();
        rezip(&e)
    }

    #[test]
    fn a_corrupt_version_is_an_error_not_a_panic() {
        let (d, h, saved) = versions();
        let file = save_with_history(&d, &h, None).unwrap();
        let deep = format!("[{{\"op\":\"set\",\"path\":\"{}\",\"value\":1}}]", "/a".repeat(10_000));
        for bad in [
            &b"not json"[..],
            br#"[{"op":"set","path":"/spreads/99/x","value":1}]"#,
            br#"[{"op":"remove","path":"/title/0"}]"#,
            br#"[{"op":"insert","path":"/spreads/99999999999999999999999","value":1}]"#,
            br#"[{"op":"insert","path":"/title","value":1}]"#,
            br#"[{"op":"remove","path":""}]"#,
            br#"[{"op":"set","path":"title","value":1}]"#,
            br#"[{"op":"bogus","path":"/title"}]"#,
            br#"[{"op":"set","path":"/title","value":{"not":"a title"}}]"#,
            deep.as_bytes(),
        ] {
            let broken = with_entry(&file, "history/3.json", bad);
            let (doc, back) = load_with_history(&broken).unwrap();
            assert_eq!(doc, load(&save(&d).unwrap()).unwrap(), "the document opens");
            let back = back.unwrap();
            assert!(matches!(back.document(3), Err(FormatError::History(_))), "{}", String::from_utf8_lossy(bad));
            assert!(back.document(5).is_err(), "made from it");
            assert_eq!(back.document(2).unwrap(), load(&saved[1]).unwrap());
            assert_eq!(back.document(12).unwrap(), load(&saved[11]).unwrap(), "past the next whole version");
        }
        // A list of versions that doesn't read, or versions made from each other: the document
        // still opens.
        let looped = br#"[{"id":1,"parent":2,"kind":"save","full":false,"size":0,"format":2},{"id":2,"parent":1,"kind":"save","full":false,"size":0,"format":2}]"#;
        let e: Vec<_> = entries(&file)
            .into_iter()
            .filter(|(n, _)| !n.starts_with("history/") || n == "history/versions.json" || n == "history/1.json" || n == "history/2.json")
            .collect();
        let broken = with_entry(&rezip(&e), "history/versions.json", looped);
        let back = load_with_history(&broken).unwrap().1.unwrap();
        assert!(back.document(1).is_err());
        let twice = br#"[{"id":1,"kind":"save","full":true,"size":0,"format":2},{"id":1,"kind":"save","full":true,"size":0,"format":2}]"#;
        for bad in [&b"{"[..], br#"[{"id":1}]"#, br#"[{"id":99,"kind":"save","full":true,"size":0,"format":2}]"#, twice] {
            let (doc, back) = load_with_history(&with_entry(&file, "history/versions.json", bad)).unwrap();
            assert_eq!(doc, load(&save(&d).unwrap()).unwrap());
            assert!(back.is_err());
        }
        // A new version on top of one that doesn't rebuild is kept whole.
        let mut back = load_with_history(&with_entry(&file, "history/25.json", b"[]x")).unwrap().1.unwrap();
        let id = back.commit(&d, NewVersion::default()).unwrap();
        assert!(back.versions().last().unwrap().full);
        assert_eq!(back.document(id).unwrap(), load(&save(&d).unwrap()).unwrap());
    }

    fn meta(file: &[u8]) -> Value {
        serde_json::from_slice(&entries(file).into_iter().find(|(n, _)| n == "meta.json").unwrap().1).unwrap()
    }

    #[test]
    fn files_without_history_stay_valid() {
        let (d, h, _) = versions();
        let plain = save(&d).unwrap();
        let (doc, back) = load_with_history(&plain).unwrap();
        assert_eq!(doc, load(&plain).unwrap());
        assert!(back.unwrap().is_empty());
        assert_eq!(meta(&plain).get("history"), None);
        // An empty history writes nothing.
        assert_eq!(entries(&save_with_history(&d, &History::default(), None).unwrap()).len(), entries(&plain).len());
        // A file with history opens as the document without it.
        let file = save_with_history(&d, &h, None).unwrap();
        assert_eq!(meta(&file).get("history"), Some(&json!(true)));
        assert_eq!(meta(&file).get("version"), meta(&plain).get("version"), "the format version is upstream's");
        assert_eq!(load(&file).unwrap(), load(&plain).unwrap());
        // `assets/` holds only the document's files, as before.
        let names = |f: &[u8]| entries(f).into_iter().map(|(n, _)| n).filter(|n| n.starts_with("assets/")).collect::<Vec<_>>();
        assert_eq!(names(&file), names(&plain));
    }

    /// Pruning drops the oldest saves past the newest `keep_last` (with manual versions too when
    /// told), and every version it keeps rebuilds as it was, in the file too: the versions made
    /// from a dropped one are kept as changes from its parent, or whole when it was whole.
    #[test]
    fn pruning_keeps_every_kept_version_as_it_was() {
        let mut d = start();
        let mut h = History::default();
        let mut saved = Vec::new();
        let mut images = Vec::new();
        for n in 1..=25 {
            edit(&mut d, n, &mut images);
            let kind = if n == 5 || n == 11 { VersionKind::Manual } else { VersionKind::Save };
            h.commit(&d, NewVersion { kind, ..Default::default() }).unwrap();
            saved.push(save(&d).unwrap());
        }
        assert!(h.is_latest(&d));
        assert_eq!(h.prune(4, true), 19);
        let kept: Vec<u32> = h.versions().map(|v| v.id).collect();
        assert_eq!(kept, [5, 11, 22, 23, 24, 25]);
        assert_eq!(h.versions().filter(|v| v.full).map(|v| v.id).collect::<Vec<_>>(), [5, 11, 22]);
        assert_eq!(h.versions().map(|v| v.parent).collect::<Vec<_>>(), [None, Some(5), Some(11), Some(22), Some(23), Some(24)]);
        let back = load_with_history(&save_with_history(&d, &h, None).unwrap()).unwrap().1.unwrap();
        for id in kept {
            assert_eq!(back.document(id).unwrap(), load(&saved[id as usize - 1]).unwrap(), "version {id}");
        }
        assert!(h.is_latest(&d));
        // New bytes for the image, the same JSON: not the latest any more.
        let a = images[0].0;
        Arc::make_mut(d.assets.get_mut(&a).unwrap()).data = Arc::new(b"four".to_vec());
        assert!(!h.is_latest(&d));
        assert_eq!(h.prune(0, false), 6);
        assert!(h.is_empty() && !h.is_latest(&d));
    }

    fn changes(a: &Value, b: &Value) -> Value {
        let mut ops = Vec::new();
        diff(a, b, "", &mut ops);
        serde_json::to_value(ops).unwrap()
    }

    #[test]
    fn changes_name_only_what_changed() {
        let items: Vec<Value> = (0..30).map(|i| json!({"id": i, "xf": [1, 0, 0, 1, i, 0]})).collect();
        // Brought to the front: out of its place, in at the end.
        let mut front = items.clone();
        let it = front.remove(3);
        front.push(it);
        assert_eq!(
            changes(&json!({"items": items}), &json!({"items": front})),
            json!([{"op": "remove", "path": "/items/3"}, {"op": "insert", "path": "/items/29", "value": {"id": 3, "xf": [1, 0, 0, 1, 3, 0]}}])
        );
        // Moved: the one number.
        let mut moved = items.clone();
        moved[5]["xf"][4] = json!(50);
        assert_eq!(changes(&json!({"items": items}), &json!({"items": moved})), json!([{"op": "set", "path": "/items/5/xf/4", "value": 50}]));
        // A paragraph added among paragraphs without ids, and a member with `/` and `~` in its name.
        let paras: Vec<Value> = (0..20).map(|i| json!({"style": i % 3})).collect();
        let mut more = paras.clone();
        more.insert(4, json!({"style": 7}));
        assert_eq!(changes(&json!(paras), &json!(more)), json!([{"op": "insert", "path": "/4", "value": {"style": 7}}]));
        assert_eq!(changes(&json!({}), &json!({"a/b~": 1})), json!([{"op": "set", "path": "/a~1b~0", "value": 1}]));
    }

    /// JSON with arrays of objects that have distinct ids, and keys that need escaping.
    fn json_value() -> impl Strategy<Value = Value> {
        let leaf =
            prop_oneof![Just(Value::Null), any::<bool>().prop_map(Value::from), (0i64..4).prop_map(Value::from), "[ab~/]{0,2}".prop_map(Value::from)];
        leaf.prop_recursive(4, 40, 5, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
                prop::collection::btree_map("[ab~/]{1,2}", inner.clone(), 0..4).prop_map(|m| Value::Object(m.into_iter().collect())),
                (prop::sample::subsequence((0..6).collect::<Vec<i32>>(), 0..=6).prop_shuffle(), prop::collection::vec(inner, 6))
                    .prop_map(|(ids, v)| Value::Array(ids.into_iter().map(|i| json!({"id": i, "v": v[i as usize]})).collect())),
            ]
        })
    }

    proptest! {
        #[test]
        fn changes_rebuild_the_new_json(a in json_value(), b in json_value()) {
            let mut ops = Vec::new();
            diff(&a, &b, "", &mut ops);
            let ops: Vec<Op> = serde_json::from_slice(&serde_json::to_vec(&ops).unwrap()).unwrap();
            let mut v = a.clone();
            for op in ops {
                apply(&mut v, op).unwrap();
            }
            prop_assert_eq!(v, b);
        }
    }
}
