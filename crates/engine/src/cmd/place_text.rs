//! File › Place of text files (plain text, Word, RTF): into the text insertion point, the
//! selected frame, or a new frame on the page; with autoflow, pages and threaded frames are added
//! until the whole story fits (InDesign's Shift-click with a loaded text cursor).

use std::collections::{BTreeMap, HashMap, HashSet};

use designcraft_doc::{
    CharAttrs, CharacterStyle, Content, Document, ImportPreset, ParaAttrs, ParaFormat, ParagraphStyle, SpreadRef, Story, StoryId, StyleRule,
    StyleRuleAttrs, StyleRuleKind, TextDirection, TextFileLink, TextSel, WorkbookPart,
};
use designcraft_geom::Rect;
use designcraft_textimport::{Imported, ImportedStyle};
use serde_json::{Value, json};

use super::bad;
use crate::{Result, Session};

/// Add the imported styles the document doesn't have yet (existing styles win, as in InDesign's
/// default style-conflict option).
/// How imported styles that share a name with the document's are handled (Word Import Options).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Conflict {
    /// Use the document's definition.
    UseExisting,
    /// The imported definition replaces the document's.
    Redefine,
    /// Import under a new name (`Name_wrd_1`).
    AutoRename,
}

impl Conflict {
    fn parse(c: &str, cmd: &str) -> Result<Self> {
        match c {
            "useExisting" | "" => Ok(Conflict::UseExisting),
            "redefine" => Ok(Conflict::Redefine),
            "autoRename" => Ok(Conflict::AutoRename),
            c => Err(bad(cmd, format!("unknown styleConflicts `{c}` (useExisting, redefine, autoRename)"))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Conflict::UseExisting => "useExisting",
            Conflict::Redefine => "redefine",
            Conflict::AutoRename => "autoRename",
        }
    }
}

/// Map imported style names (`styleMap`) and resolve name clashes in the import before it lands.
/// Returns `map` with the names Auto Rename chose.
fn map_styles(
    d: &Document,
    imp: &mut designcraft_textimport::Imported,
    map: &std::collections::HashMap<String, String>,
    conflict: Conflict,
) -> std::collections::HashMap<String, String> {
    let mut rename: std::collections::HashMap<String, String> = map.clone();
    if conflict == Conflict::AutoRename {
        let st = &d.styles;
        for s in imp.para_styles.iter().filter(|s| !map.contains_key(&s.name) && st.para(&s.name).is_some()) {
            if let Some(n) = (1..=st.paragraph.len() + 1).map(|i| format!("{}_wrd_{i}", s.name)).find(|n| st.para(n).is_none()) {
                rename.insert(s.name.clone(), n);
            }
        }
        for s in imp.char_styles.iter().filter(|s| !map.contains_key(&s.name) && st.char_style(&s.name).is_some()) {
            if let Some(n) = (1..=st.character.len() + 1).map(|i| format!("{}_wrd_{i}", s.name)).find(|n| st.char_style(n).is_none()) {
                rename.insert(s.name.clone(), n);
            }
        }
    }
    if rename.is_empty() {
        return rename;
    }
    // Mapped styles aren't imported; renamed ones come in under their new name.
    imp.para_styles.retain(|s| !map.contains_key(&s.name));
    imp.char_styles.retain(|s| !map.contains_key(&s.name));
    for s in imp.para_styles.iter_mut().chain(imp.char_styles.iter_mut()) {
        if let Some(n) = rename.get(&s.name) {
            s.name = n.clone();
        }
        if let Some(b) = s.based_on.as_ref().and_then(|b| rename.get(b)) {
            s.based_on = Some(b.clone());
        }
    }
    let n = imp.story.len();
    imp.story.format_paras(0..n, |p| {
        if let Some(m) = rename.get(&p.style) {
            p.style = m.clone();
        }
    });
    imp.story.format_chars(0..n, |f| {
        if let Some(m) = rename.get(&f.style) {
            f.style = m.clone();
        }
    });
    rename
}

/// `styleConflicts` checked: its name as `file.place` takes it.
pub(super) fn conflicts_name(c: &str, cmd: &str) -> Result<&'static str> {
    Conflict::parse(c, cmd).map(Conflict::name)
}

/// Does `name` match `pattern`: `*` stands for any run of characters, `?` for any one, and case
/// doesn't count.
fn glob(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.to_lowercase().chars().collect(), name.to_lowercase().chars().collect());
    // The last `*` and where in `name` its run started: a mismatch makes that run one longer.
    let (mut pi, mut ni, mut star) = (0, 0, None);
    while ni < n.len() {
        match (p.get(pi), n.get(ni)) {
            (Some('*'), _) => {
                star = Some((pi, ni));
                pi += 1;
            }
            (Some(c), Some(x)) if *c == '?' || c == x => {
                pi += 1;
                ni += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    star = Some((sp, sn + 1));
                    (pi, ni) = (sp + 1, sn + 1);
                }
                None => return false,
            },
        }
    }
    p.get(pi..).is_some_and(|rest| rest.iter().all(|c| *c == '*'))
}

/// At most this many rules in one `styleRules`.
const MAX_RULES: usize = 500;

/// The `styleRules` of `p`, checked (`None`: not given).
pub(super) fn rules_param(p: &Value, cmd: &str) -> Result<Option<Vec<StyleRule>>> {
    let Some(v) = p.get("styleRules").filter(|v| !v.is_null()) else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| bad(cmd, "`styleRules` must be a list of rules"))?;
    if list.len() > MAX_RULES {
        return Err(bad(cmd, format!("`styleRules` has at most {MAX_RULES} rules")));
    }
    let mut rules = Vec::with_capacity(list.len());
    for (i, r) in list.iter().enumerate() {
        let why = |e: String| bad(cmd, format!("`styleRules` rule {}: {e}", i + 1));
        let keys = r.as_object().ok_or_else(|| why("must be an object".into()))?;
        let known = |k: &String, of: &[&str]| of.contains(&k.as_str());
        if let Some(k) = keys.keys().find(|k| !known(k, &["match", "kind", "attrs", "to", "keepOverrides"])) {
            return Err(why(format!("unknown key `{k}`")));
        }
        if let Some(k) = keys.get("attrs").and_then(Value::as_object).and_then(|a| a.keys().find(|k| !known(k, &["para", "chars"])).cloned()) {
            return Err(why(format!("`attrs` takes para and chars, not `{k}`")));
        }
        let rule: StyleRule = serde_json::from_value(r.clone()).map_err(|e| why(e.to_string()))?;
        if rule.to.trim().is_empty() {
            return Err(why("give `to`, the document style it maps to".into()));
        }
        rules.push(rule);
    }
    Ok(Some(rules))
}

/// An imported style's attributes with those of the styles it is based on, in the import.
fn imported_attrs(styles: &[ImportedStyle], name: &str) -> (ParaAttrs, CharAttrs) {
    let mut chain: Vec<&ImportedStyle> = Vec::new();
    let mut cur = styles.iter().find(|s| s.name == name);
    while let Some(s) = cur {
        if chain.iter().any(|c| c.name == s.name) {
            break;
        }
        chain.push(s);
        cur = s.based_on.as_deref().and_then(|b| styles.iter().find(|x| x.name == b));
    }
    let (mut para, mut chars) = (ParaAttrs::default(), CharAttrs::default());
    for s in chain.into_iter().rev() {
        para.merge(&s.para);
        chars.merge(&s.chars);
    }
    (para, chars)
}

/// Does a style with the attributes `own` have every attribute `want` gives, with its value?
fn has_attrs(want: &StyleRuleAttrs, own: &(ParaAttrs, CharAttrs)) -> bool {
    let (mut para, mut chars) = own.clone();
    para.merge(&want.para);
    chars.merge(&want.chars);
    (para, chars) == *own
}

/// What `rules` make of the imported styles `map` doesn't name: for each, the first rule that
/// matches it (kind, name and attributes) and names a style of its kind in the document. Also
/// the styles (`true`: a paragraph style) whose rule clears their text's own formatting.
fn resolve_rules(
    d: &Document,
    imp: &Imported,
    map: &HashMap<String, String>,
    rules: &[StyleRule],
) -> (HashMap<String, String>, HashSet<(bool, String)>) {
    let (mut mapped, mut clear) = (HashMap::new(), HashSet::new());
    if rules.is_empty() {
        return (mapped, clear);
    }
    for (para, styles) in [(true, &imp.para_styles), (false, &imp.char_styles)] {
        for s in styles.iter().filter(|s| !map.contains_key(&s.name)) {
            let own = imported_attrs(styles, &s.name);
            let rule = rules.iter().find(|r| {
                let kind = match r.kind {
                    StyleRuleKind::Any => true,
                    StyleRuleKind::Paragraph => para,
                    StyleRuleKind::Character => !para,
                };
                let target = if para { d.styles.para(&r.to).is_some() } else { d.styles.char_style(&r.to).is_some() };
                kind && target && r.pattern.as_deref().is_none_or(|p| glob(p, &s.name)) && has_attrs(&r.attrs, &own)
            });
            if let Some(r) = rule {
                mapped.insert(s.name.clone(), r.to.clone());
                if !r.keep_overrides {
                    clear.insert((para, s.name.clone()));
                }
            }
        }
    }
    (mapped, clear)
}

/// Text whose style a rule with `keepOverrides: false` maps (`true`: a paragraph style) loses its
/// own formatting, as with Clear Overrides. Run while the text still has the import's style names.
fn clear_overrides(story: &mut Story, clear: &HashSet<(bool, String)>) {
    if clear.is_empty() {
        return;
    }
    let n = story.len();
    let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, range) in story.para_ranges().into_iter().enumerate() {
        let Some(p) = story.paras.get_mut(i) else { continue };
        if clear.contains(&(true, p.style.clone())) {
            p.para = ParaAttrs::default();
            p.chars = CharAttrs::default();
            runs.push(range.start..range.end.saturating_add(1).min(n));
        }
    }
    runs.extend(story.runs().filter(|(_, f)| clear.contains(&(false, f.style.clone()))).map(|(r, _)| r));
    for r in runs {
        story.format_chars(r, |f| f.over = CharAttrs::default());
    }
}

/// Map the import's styles by `map` and then `rules` (clearing what a rule asks), resolving name
/// clashes as `conflict` says. Returns what the link keeps: `map`'s mappings and Auto Rename's
/// names, not what the rules mapped (they map again on Update).
fn apply_options(
    d: &Document,
    imp: &mut Imported,
    map: &HashMap<String, String>,
    rules: &[StyleRule],
    conflict: Conflict,
) -> HashMap<String, String> {
    let (ruled, clear) = resolve_rules(d, imp, map, rules);
    clear_overrides(&mut imp.story, &clear);
    let mut all = map.clone();
    all.extend(ruled.iter().map(|(k, v)| (k.clone(), v.clone())));
    let mut kept = map_styles(d, imp, &all, conflict);
    kept.retain(|k, _| !ruled.contains_key(k));
    kept
}

/// What `preset` maps of the import's styles onto styles the document has: its names, then its
/// rules (Import Options › Preset fills the mapping with it).
pub(super) fn preset_map(d: &Document, imp: &Imported, preset: &ImportPreset) -> BTreeMap<String, String> {
    let names: HashSet<&str> = imp.para_styles.iter().chain(&imp.char_styles).map(|s| s.name.as_str()).collect();
    let has = |to: &str| d.styles.para(to).is_some() || d.styles.char_style(to).is_some();
    let map: HashMap<String, String> =
        preset.style_map.iter().filter(|(from, to)| names.contains(from.as_str()) && has(to)).map(|(f, t)| (f.clone(), t.clone())).collect();
    let (ruled, _) = resolve_rules(d, imp, &map, &preset.style_rules);
    map.into_iter().chain(ruled).collect()
}

/// A `file.place`'s Word Import Options: those it gives, else its `preset`'s.
struct Options {
    map: HashMap<String, String>,
    rules: Vec<StyleRule>,
    conflict: Conflict,
    remove: bool,
}

fn options(s: &Session, p: &Value, cmd: &str) -> Result<Options> {
    let preset = match super::str_param(p, "preset") {
        Some(name) => super::place_presets::find(s, name, cmd)?,
        None => ImportPreset::default(),
    };
    let conflict = Conflict::parse(p.get("styleConflicts").and_then(Value::as_str).unwrap_or(&preset.style_conflicts), cmd)?;
    let map = match p.get("styleMap").and_then(Value::as_object) {
        Some(o) => o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect(),
        None => preset.style_map.into_iter().collect(),
    };
    let rules = rules_param(p, cmd)?.unwrap_or(preset.style_rules);
    let remove = p.get("removeStyles").and_then(Value::as_bool).unwrap_or(preset.remove_styles);
    Ok(Options { map, rules, conflict, remove })
}

fn add_styles(d: &mut Document, imp: &designcraft_textimport::Imported, conflict: Conflict) {
    let st = d.styles_mut();
    if conflict == Conflict::Redefine {
        for s in &imp.para_styles {
            if let Some(ps) = st.paragraph.iter_mut().find(|p| p.name == s.name) {
                (ps.para, ps.chars) = (s.para.clone(), s.chars.clone());
            }
        }
        for s in &imp.char_styles {
            if let Some(cs) = st.character.iter_mut().find(|c| c.name == s.name) {
                cs.chars = s.chars.clone();
            }
        }
    }
    for s in &imp.para_styles {
        if st.para(&s.name).is_none() {
            st.paragraph.push(ParagraphStyle {
                name: s.name.clone(),
                based_on: s.based_on.clone().or_else(|| Some(designcraft_doc::story::BASIC_PARAGRAPH.into())),
                next_style: None,
                para: s.para.clone(),
                chars: s.chars.clone(),
                shortcut: String::new(),
            });
        }
    }
    for s in &imp.char_styles {
        if !st.character.iter().any(|c| c.name == s.name) {
            st.character.push(CharacterStyle { name: s.name.clone(), based_on: s.based_on.clone(), chars: s.chars.clone(), shortcut: String::new() });
        }
    }
}

/// "Remove styles and formatting": every paragraph [Basic Paragraph], no local formatting.
fn strip(story: &mut Story) {
    let n = story.len();
    story.format_paras(0..n, |p| *p = ParaFormat { table: p.table, ..Default::default() });
    story.format_chars(0..n, |f| *f = Default::default());
}

/// The story's whole text replaced by the import's (its frames stay).
fn fill(story: &mut Story, from: &Story) {
    let n = story.len();
    story.delete(0..n);
    story.insert_story(0, from);
    if let (Some(first), Some(p0)) = (from.paras.first(), story.paras.first_mut()) {
        *p0 = ParaFormat { table: p0.table, ..first.clone() };
    }
}

/// A workbook's options from `file.place` or `place.check`: `sheet`, `range`, `headerRows`,
/// `tableStyle`.
pub(super) fn workbook_param(p: &Value, cmd: &str) -> Result<WorkbookPart> {
    let text = |k: &str| p.get(k).and_then(Value::as_str).map_or_else(String::new, |v| v.trim().to_string());
    let header_rows = match p.get("headerRows") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad(cmd, "headerRows: a number of rows, 0 or more"))?,
    };
    Ok(WorkbookPart { sheet: text("sheet"), range: text("range"), header_rows, table_style: text("tableStyle") })
}

/// File `name` imported with a workbook's options, which other files can't take.
pub(super) fn import_part(name: &str, bytes: &[u8], w: &WorkbookPart) -> std::result::Result<designcraft_textimport::Imported, String> {
    if *w != WorkbookPart::default() && !name.to_ascii_lowercase().ends_with(".xlsx") {
        return Err(format!("{name} isn't a workbook: sheet, range, headerRows and tableStyle are for .xlsx files"));
    }
    let opts = designcraft_textimport::XlsxOptions {
        sheet: Some(w.sheet.clone()).filter(|s| !s.is_empty()),
        range: Some(w.range.clone()).filter(|r| !r.is_empty()),
        header_rows: w.header_rows as usize,
    };
    designcraft_textimport::import_with(name, bytes, &opts).map_err(|e| e.to_string())
}

/// The import's tables in table style `name` (none when empty), after the header rows are set
/// so the style's header cells land on them.
fn style_tables(d: &Document, imp: &mut designcraft_textimport::Imported, name: &str) {
    if name.is_empty() {
        return;
    }
    let ts = d.styles.resolve_table_style(name);
    for t in imp.story.tables.values_mut() {
        ts.apply_to(std::sync::Arc::make_mut(t), &d.styles.cell);
    }
}

/// The link a story placed from `path` keeps (Create Links When Placing Text Files).
/// `before`: the file's size and time before `bytes` were read (its time is kept only when the
/// size matches what was read and it can be trusted, see [`super::links::settled`]).
fn text_link(
    path: &str,
    bytes: &[u8],
    before: Option<designcraft_doc::Fingerprint>,
    map: std::collections::HashMap<String, String>,
    conflict: Conflict,
    remove: bool,
    story: &Story,
) -> TextFileLink {
    TextFileLink {
        path: path.to_string(),
        size: bytes.len() as u64,
        hash: super::links::fnv_hex(bytes),
        mtime: before.filter(|f| f.size == bytes.len() as u64 && super::links::settled(f)).map_or(0, |f| f.mtime),
        style_map: map.into_iter().collect(),
        style_conflicts: conflict.name().into(),
        remove_styles: remove,
        story_hash: super::links::story_hash(story),
        base: Some(std::sync::Arc::new(Story { frames: Vec::new(), file_link: None, ..story.clone() })),
        last_update: None,
        workbook: WorkbookPart::default(),
        style_rules: Vec::new(),
    }
}

/// The story's link, or why there is none.
fn linked(d: &Document, sid: StoryId) -> Result<TextFileLink> {
    d.story(sid).and_then(|s| s.file_link.clone()).ok_or_else(|| bad("links.update", format!("story {} isn't linked to a file", sid.0)))
}

/// What a linked file's import options leave to record on its link again.
struct Again {
    map: std::collections::HashMap<String, String>,
    conflict: Conflict,
    workbook: WorkbookPart,
    rules: Vec<StyleRule>,
}

/// The linked file imported again with the import options it was placed with, and its styles
/// added to the document as on placing: the import, and the options to record.
fn import_again(d: &mut Document, link: &TextFileLink, path: &str, bytes: &[u8]) -> Result<(designcraft_textimport::Imported, Again)> {
    let name = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().to_string());
    let mut imp = import_part(&name, bytes, &link.workbook).map_err(|e| bad("links.update", format!("{name}: {e}")))?;
    if link.remove_styles {
        strip(&mut imp.story);
        imp.para_styles.clear();
        imp.char_styles.clear();
    }
    let conflict = Conflict::parse(&link.style_conflicts, "links.update")?;
    // A mapping to a style the document no longer has is dropped: that style is imported again.
    let map: std::collections::HashMap<String, String> = link
        .style_map
        .iter()
        .filter(|(_, to)| d.styles.para(to).is_some() || d.styles.char_style(to).is_some())
        .map(|(from, to)| (from.clone(), to.clone()))
        .collect();
    let map = apply_options(d, &mut imp, &map, &link.style_rules, conflict);
    add_styles(d, &imp, conflict);
    let mut workbook = link.workbook.clone();
    if !workbook.table_style.is_empty() && !d.styles.table.iter().any(|t| t.name == workbook.table_style) {
        imp.warnings.push(format!("the table style “{}” no longer exists, so the table comes in without it", workbook.table_style));
        workbook.table_style.clear();
    }
    style_tables(d, &mut imp, &workbook.table_style);
    Ok((imp, Again { map, conflict, workbook, rules: link.style_rules.clone() }))
}

/// Update Link of a linked text file: `bytes` (read from `path`) imported again into the story
/// with the import options it was placed with, keeping its frames. Returns the import's warnings.
pub(crate) fn reimport(
    d: &mut Document,
    sid: StoryId,
    path: &str,
    bytes: &[u8],
    before: Option<designcraft_doc::Fingerprint>,
    allow_errors: bool,
) -> Result<(Vec<String>, Vec<designcraft_textimport::Problem>)> {
    let link = linked(d, sid)?;
    let (imp, again) = import_again(d, &link, path, bytes)?;
    allowed(&imp, path, allow_errors)?;
    let story = d.story_mut(sid).ok_or(designcraft_doc::DocError::NoStory(sid))?;
    fill(story, &imp.story);
    let applied = text_link(path, bytes, before, again.map, again.conflict, link.remove_styles, story);
    story.file_link = Some(TextFileLink { workbook: again.workbook, style_rules: again.rules, ..applied });
    mark(story, &Value::Null, &imp.problems);
    Ok((imp.warnings, imp.problems))
}

/// Errors the file shows stop an update unless allowed: the app asks first, scripts pass
/// `allowErrors`. Values from other workbooks are only reported.
fn allowed(imp: &designcraft_textimport::Imported, path: &str, allow: bool) -> Result<()> {
    let blocking: Vec<String> = imp.problems.iter().filter(|p| p.kind.blocks()).map(designcraft_textimport::Problem::message).collect();
    if allow || blocking.is_empty() {
        return Ok(());
    }
    let name = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().to_string());
    let list = blocking.join("; ");
    let n = blocking.len();
    Err(bad("links.update", format!("{name} shows {n} error{}: {list}. Fix the file, or update with allowErrors", if n == 1 { "" } else { "s" })))
}

/// Update Link by merging (see [`designcraft_flow::merge_story`]): the file's changes come into the
/// story, the layout's own edits survive and are reported, and where both changed a sentence the
/// file's text wins and a Flow note keeps the layout's. Refused, with nothing changed, for text with
/// tracked changes or a merge the 0% check can't verify. Returns the warnings and the report.
pub(crate) fn remerge(
    d: &mut Document,
    sid: StoryId,
    path: &str,
    bytes: &[u8],
    before: Option<designcraft_doc::Fingerprint>,
    allow_errors: bool,
) -> Result<(Vec<String>, Value, Vec<designcraft_textimport::Problem>)> {
    let link = linked(d, sid)?;
    let base = link.base.clone().ok_or_else(|| bad("links.update", format!("story {} has nothing to merge with", sid.0)))?;
    // The last update's highlights are the panel's, not the layout's formatting: off for the merge,
    // and this update's marks go on afterwards.
    let mut mine = d.story(sid).ok_or(designcraft_doc::DocError::NoStory(sid))?.clone();
    unmark(&mut mine, false);
    if mine.runs().any(|(_, f)| matches!(f.over.change, Some(designcraft_doc::ChangeMark::Inserted | designcraft_doc::ChangeMark::Deleted))) {
        return Err(bad("links.update", format!("story {}: accept or reject its tracked changes before updating it", sid.0)));
    }
    let (imp, again) = import_again(d, &link, path, bytes)?;
    allowed(&imp, path, allow_errors)?;
    // The file as a plain update would leave the story: what this merge and the next compare with.
    let mut theirs = Story::new(sid);
    fill(&mut theirs, &imp.story);
    let m = designcraft_flow::merge_story(&base, &mine, &theirs)
        .filter(|m| verified(&theirs, &mine, m))
        .ok_or_else(|| bad("links.update", format!("story {}: the merge couldn't be verified, so nothing was changed", sid.0)))?;
    let mut report = report(sid, &theirs, &mine, &m);
    let mut warnings = imp.warnings;
    warnings.extend(m.warnings.iter().cloned());
    let story = d.story_mut(sid).ok_or(designcraft_doc::DocError::NoStory(sid))?;
    *story = m.story;
    // The story counts as edited while it differs from the file as applied.
    let applied = text_link(path, bytes, before, again.map, again.conflict, link.remove_styles, story);
    let story_hash = super::links::story_hash(&theirs);
    story.file_link =
        Some(TextFileLink { base: Some(std::sync::Arc::new(theirs)), story_hash, workbook: again.workbook, style_rules: again.rules, ..applied });
    mark(story, &report, &imp.problems);
    place_report(d, sid, &mut report);
    Ok((warnings, report, imp.problems))
}

/// Take Flow's highlight marks off a story and its table cells; errors keep theirs with
/// `keep_errors` (Clear Highlights: they stay until fixed).
pub(crate) fn unmark(st: &mut Story, keep_errors: bool) {
    let clear = |f: &mut designcraft_doc::CharFormat| {
        if !(keep_errors && f.over.flow == Some(designcraft_doc::FlowMark::Error)) {
            f.over.flow = None;
        }
    };
    if st.runs().any(|(_, f)| f.over.flow.is_some()) {
        let n = st.len();
        st.format_chars(0..n, clear);
    }
    for t in st.tables.values_mut().filter(|t| t.cells.iter().any(|c| c.text.runs().any(|(_, f)| f.over.flow.is_some()))) {
        for c in std::sync::Arc::make_mut(t).cells.iter_mut() {
            let n = c.text.len();
            c.text.format_chars(0..n, clear);
        }
    }
}

/// Mark what an update brought in, for the page's highlights: each reported change, kept edit and
/// conflict (the file's text) on its stretch of the story or cell, and what shows an error — a
/// cell, or a Word field error's text.
fn mark(st: &mut Story, report: &Value, problems: &[designcraft_textimport::Problem]) {
    use designcraft_doc::FlowMark;
    let range = |e: &Value| -> Option<std::ops::Range<usize>> {
        let a = usize::try_from(e["at"]["start"].as_u64()?).ok()?;
        let b = usize::try_from(e["at"]["end"].as_u64()?).ok()?;
        Some(a..b)
    };
    for (key, flow) in [("changes", FlowMark::File), ("deviations", FlowMark::Kept), ("conflicts", FlowMark::Conflict)] {
        for e in report[key].as_array().into_iter().flatten() {
            let Some(r) = range(e) else { continue };
            let set = |f: &mut designcraft_doc::CharFormat| f.over.flow = Some(flow);
            match e.get("cell").filter(|c| c.is_object()) {
                Some(c) => {
                    let (row, col) =
                        (c["row"].as_u64().and_then(|v| usize::try_from(v).ok()), c["col"].as_u64().and_then(|v| usize::try_from(v).ok()));
                    if let (Some(t), Some(row), Some(col)) =
                        (c["table"].as_u64().and_then(|t| st.tables.get_mut(&t)).map(std::sync::Arc::make_mut), row, col)
                        && let Some(cell) = t.cell_mut(row, col)
                    {
                        cell.text.format_chars(r, set);
                    }
                }
                None => st.format_chars(r, set),
            }
        }
    }
    for p in problems {
        let flow = if p.kind.blocks() { FlowMark::Error } else { FlowMark::Notice };
        let set = |f: &mut designcraft_doc::CharFormat| f.over.flow = Some(flow);
        match p.cell {
            Some((row, col)) => {
                if let Some(cell) = st.tables.values_mut().next().map(std::sync::Arc::make_mut).and_then(|t| t.cell_mut(row, col)) {
                    let n = cell.text.len();
                    cell.text.format_chars(0..n, set);
                }
            }
            None if !p.shows.is_empty() => {
                let found: Vec<usize> = st.text.match_indices(p.shows.as_str()).map(|(i, _)| i).collect();
                for at in found {
                    st.format_chars(at..at + p.shows.len(), set);
                }
            }
            None => {}
        }
    }
}

/// The 0% check run before a merge changes the document: the merged text differs from the file's
/// only by the reported deviations, and the reported changes turn the layout's text into it.
pub(crate) fn verified(theirs: &Story, mine: &Story, m: &designcraft_flow::StoryMerge) -> bool {
    designcraft_flow::verified(theirs, mine, m)
}

/// What a merge did to a story, in whole sentences: the changes to its text, the layout's edits
/// that differ from the file, the conflicts (the file's text kept, the layout's in a Flow note) and
/// its warnings; table cells merged cell by cell add theirs, each with its `cell` (table, row,
/// column). Markers show as labels; a difference that is only conflict marks is in conflicts.
fn report(sid: StoryId, theirs: &Story, mine: &Story, m: &designcraft_flow::StoryMerge) -> Value {
    let mut entries: [Vec<Value>; 3] = Default::default();
    report_text(&mine.text, &theirs.text, &m.story.text, &m.text, None, &mut entries);
    for c in &m.cells {
        let cell = json!({"table": c.table, "row": c.row, "col": c.col});
        report_text(&c.layout, &c.file, &c.text.text, &c.text, Some(cell), &mut entries);
    }
    let [changes, deviations, conflicts] = entries;
    json!({"story": sid.0, "changes": changes, "deviations": deviations, "conflicts": conflicts, "warnings": m.warnings})
}

/// Report entries for one merged text (`mine` → `out`, against the file's `theirs`): its changes,
/// deviations and conflicts, in whole sentences, each with where it is in `out` (`at`, for Go To)
/// and how it changed word by word (`words`: changes from before to after, deviations from the
/// file to the layout, conflicts from the layout to the file).
fn report_text(mine: &str, theirs: &str, out: &str, m: &designcraft_flow::MergeOut, cell: Option<Value>, into: &mut [Vec<Value>; 3]) {
    let raw = |s: &'_ str, r: &std::ops::Range<usize>| s.get(r.clone()).unwrap_or_default().to_string();
    let text = |s: &str, r: &std::ops::Range<usize>| designcraft_flow::shown(&raw(s, r)).trim().to_string();
    let notes_only = |a: String, b: String| a.replace(designcraft_doc::NOTE_MARK, "") == b.replace(designcraft_doc::NOTE_MARK, "");
    let entry = |mut v: Value, at: std::ops::Range<usize>, was: Option<std::ops::Range<usize>>, from: &str, to: &str| {
        if let Some(o) = v.as_object_mut() {
            if let Some(c) = &cell {
                o.insert("cell".into(), c.clone());
            }
            o.insert("at".into(), json!({"start": at.start, "end": at.end}));
            if let Some(w) = was {
                o.insert("was".into(), json!({"start": w.start, "end": w.end}));
            }
            let words = designcraft_flow::words(from, to).into_iter().map(|(k, t)| json!({"kind": k.name(), "text": t}));
            o.insert("words".into(), Value::Array(words.collect()));
        }
        v
    };
    let [changes, deviations, conflicts] = into;
    // Where each conflict is in `out`: its mark, then the file's version. A change there is the
    // conflict, listed once, under conflicts.
    let marked: Vec<std::ops::Range<usize>> =
        m.conflicts.iter().map(|c| c.at..c.at.saturating_add(designcraft_doc::NOTE_MARK.len_utf8()).saturating_add(c.theirs.len())).collect();
    for h in designcraft_flow::sentences(mine, out, &designcraft_flow::diff_text(mine, out)) {
        if marked.iter().any(|c| c.start <= h.new.start && h.new.end <= c.end) {
            continue;
        }
        let (before, after) = (text(mine, &h.old), text(out, &h.new));
        changes.push(entry(json!({"before": before, "after": after}), h.new.clone(), Some(h.old.clone()), &before, &after));
    }
    for h in designcraft_flow::sentences(theirs, out, &m.deviations) {
        if !notes_only(raw(theirs, &h.old), raw(out, &h.new)) {
            let (file, layout) = (text(theirs, &h.old), text(out, &h.new));
            deviations.push(entry(json!({"file": file, "layout": layout}), h.new.clone(), kept_from(m, &h.new), &file, &layout));
        }
    }
    for c in &m.conflicts {
        // The merged text has the conflict mark at `at`, then the file's version.
        let (file, layout) = (text(theirs, &c.theirs), text(mine, &c.mine));
        let start = c.at.saturating_add(designcraft_doc::NOTE_MARK.len_utf8());
        conflicts.push(entry(
            json!({"file": file, "layout": layout}),
            start..start.saturating_add(c.theirs.len()),
            Some(c.mine.clone()),
            &layout,
            &file,
        ));
    }
}

/// Where the merged text's range `r` is in the layout's text before the update (`mine`): the span
/// of the layout's side of the segments it covers. `None` when none of it came from the layout.
fn kept_from(m: &designcraft_flow::MergeOut, r: &std::ops::Range<usize>) -> Option<std::ops::Range<usize>> {
    let mut span: Option<std::ops::Range<usize>> = None;
    for seg in m.segments.iter().filter(|s| s.out.start < r.end && r.start < s.out.end) {
        let Some(mine) = &seg.mine else { continue };
        // A segment from the layout holds its text unchanged, so offsets carry over one for one.
        let start = mine.start.saturating_add(r.start.saturating_sub(seg.out.start));
        let end = mine.end.saturating_sub(seg.out.end.saturating_sub(r.end));
        span = Some(span.map_or(start..end, |s| s.start.min(start)..s.end.max(end)));
    }
    span
}

/// Where a report's differences land once the updated story is laid out: the pages its frames are
/// on (`pages`), each entry's page, page name and height on the page from the top (`y`, 0–1; a
/// table cell's is its table's), `overset` for one in text that doesn't fit, and whether the story
/// still fits (`fit`: overset, and how many lines don't fit).
fn place_report(d: &Document, sid: StoryId, report: &mut Value) {
    let Some(story) = d.story(sid) else { return };
    let specs = designcraft_compose::frame_specs(d, sid);
    let cs = designcraft_compose::compose(d, story, &specs, &Default::default());
    // Frames come back in the order of their specs.
    let frames: Vec<_> = specs.iter().zip(&cs.frames).collect();
    fn name(sp: &designcraft_compose::FrameSpec) -> String {
        sp.page_name.clone().unwrap_or_default()
    }
    let mut pages: Vec<Value> = Vec::new();
    for (sp, _) in &frames {
        if let Some(p) = sp.page
            && !pages.iter().any(|v| v["index"] == p)
        {
            pages.push(json!({"index": p, "name": name(sp)}));
        }
    }
    let locate = |at: usize| -> Option<Value> {
        if cs.overset_at.is_some_and(|o| at >= o) {
            let (sp, _) = frames.last()?;
            return Some(json!({"page": sp.page, "pageName": name(sp), "y": 1.0, "overset": true}));
        }
        let (sp, f) = frames.iter().find(|(_, f)| f.range.contains(&at)).or_else(|| frames.iter().rev().find(|(_, f)| f.range.start <= at))?;
        let line = f.lines.iter().find(|l| l.range.contains(&at)).or_else(|| f.lines.iter().rev().find(|l| l.range.start <= at));
        let y = match (line, sp.page_rect) {
            (Some(l), Some((page, _))) if page.height() > 0.0 => ((l.baseline - page.y0) / page.height()).clamp(0.0, 1.0),
            _ => 0.0,
        };
        Some(json!({"page": sp.page, "pageName": name(sp), "y": (y * 1000.0).round() / 1000.0}))
    };
    // A table cell is placed where its table is anchored.
    let paras = story.para_ranges();
    let table_at = |t: u64| story.paras.iter().position(|p| p.table == Some(t)).and_then(|i| paras.get(i)).map(|r| r.start);
    for key in ["changes", "deviations", "conflicts"] {
        let Some(list) = report.get_mut(key).and_then(Value::as_array_mut) else { continue };
        for e in list.iter_mut() {
            let at = match e.get("cell").and_then(|c| c["table"].as_u64()) {
                Some(t) => table_at(t),
                None => e["at"]["start"].as_u64().and_then(|v| usize::try_from(v).ok()),
            };
            let place = at.and_then(locate);
            if let (Some(Value::Object(place)), Some(o)) = (place, e.as_object_mut()) {
                o.extend(place);
            }
        }
    }
    let lines = if cs.overset_at.is_some() { overset_lines(d, story, &specs) } else { 0 };
    if let Some(o) = report.as_object_mut() {
        o.insert("pages".into(), Value::Array(pages));
        o.insert("fit".into(), json!({"overset": cs.overset_at.is_some(), "lines": lines}));
    }
}

/// How many lines of the story don't fit: its overset text composed in one more frame, as wide as
/// the last frame's last column and tall enough to hold it.
fn overset_lines(d: &Document, story: &Story, specs: &[designcraft_compose::FrameSpec]) -> usize {
    let Some(last) = specs.last() else { return 0 };
    let col = last.columns().last().copied().unwrap_or(last.area);
    let mut more = specs.to_vec();
    let mut extra = last.clone();
    extra.area = designcraft_geom::Rect::new(col.x0, col.y0, col.x1, col.y0 + 1.0e6);
    extra.opts.columns = 1;
    extra.exclusions.clear();
    more.push(extra);
    designcraft_compose::compose(d, story, &more, &Default::default()).frames.get(specs.len()).map_or(0, |f| f.lines.len())
}

/// Is the story overset in its frames, and how much text do its full frames hold (bytes, frames)?
/// A frame a column, frame or page break ends early is left out: it says nothing about how much
/// a frame holds.
fn fit(d: &Document, sid: StoryId) -> (Option<usize>, usize, usize) {
    let cs = designcraft_compose::compose_story(d, sid, &Default::default());
    let text = d.story(sid).map_or("", |s| s.text.as_str());
    let (mut held, mut full) = (0, 0);
    for f in &cs.frames {
        let Some(shown) = text.get(f.range.clone()) else { continue };
        if !shown.is_empty() && !shown.chars().any(designcraft_doc::is_break_char) {
            held += shown.len();
            full += 1;
        }
    }
    (cs.overset_at, held, full)
}

/// The frames autoflow sets on a page: one per column inside the margins, in the order a story
/// of direction `dir` runs through them. A vertical story gets one frame on the live area.
fn flow_rects(d: &Document, dir: TextDirection, vertical: bool, page: usize) -> Vec<Rect> {
    let Some(p) = d.page(page) else { return Vec::new() };
    if vertical {
        return vec![p.margin_rect()];
    }
    let mut cols = p.column_rects();
    if dir == TextDirection::RightToLeft {
        cols.reverse();
    }
    cols
}

fn same_rect(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < 0.01)
}

/// Thread new frames until the story fits: first into the columns after the last frame's column
/// on its page (when it sits in one), then one frame per column on each page it adds. With
/// `primary` (Smart Text Reflow of the primary text frame), each added page gets one frame on the
/// live area with the page's columns instead. Empty frames it made at the end of the thread are
/// removed again, except the first on a page it didn't add; a page it added that holds only such
/// a frame goes with it. Returns pages added.
/// A story on a parent spread doesn't autoflow: its frames thread only on that parent. Errors
/// name `cmd`, the command that autoflows.
pub(crate) fn autoflow(d: &mut Document, sid: StoryId, max_pages: usize, cmd: &str, primary: bool) -> Result<usize> {
    let last = d.story(sid).and_then(|s| s.frames.last().copied()).ok_or_else(|| bad(cmd, "the story has no frame"))?;
    if !matches!(d.find(last).map(|l| l.spread), Some(SpreadRef::Doc(_))) {
        return Err(bad(cmd, "autoflow works only from a frame on a document page"));
    }
    let (dir, story_vertical) = d.story(sid).map(|s| (s.direction, s.vertical)).ok_or_else(|| bad(cmd, "no such story"))?;
    // One frame per page: on the live area, its columns those of the page.
    let vertical = story_vertical || primary;
    let mut added = 0;
    let mut made = Vec::new();
    let mut new_pages = Vec::new();
    let mut first = true;
    loop {
        let (overset, held, full) = fit(d, sid);
        let Some(at) = overset else { break };
        let len = d.story(sid).map_or(0, |s| s.len());
        // Frames still needed at the rate the full ones are filled (at least one); with no full
        // frame to go by yet, one more.
        let want = held.checked_div(full).map_or(1, |per_frame| len.saturating_sub(at).div_ceil(per_frame.max(1))).max(1);
        let last = *d.story(sid).and_then(|s| s.frames.last()).ok_or_else(|| bad(cmd, "the story has no frame"))?;
        let page = d.page_of_item(last).unwrap_or(d.page_count().saturating_sub(1));
        let layer = d.default_layer();
        let mut prev = last;
        if std::mem::take(&mut first) {
            let rects = flow_rects(d, dir, vertical, page);
            let next = d.item(last).and_then(|it| rects.iter().position(|r| same_rect(*r, it.bounds()))).map_or(rects.len(), |k| k + 1);
            if let (Some(rest), Some((si, _))) = (rects.get(next..).filter(|r| !r.is_empty()), d.page_loc(page)) {
                for r in rest.iter().take(want) {
                    prev = add_flow_frame(d, si, *r, layer, prev)?;
                    made.push(prev);
                }
                continue;
            }
        }
        if added >= max_pages {
            return Err(bad(cmd, format!("autoflow stopped after {max_pages} pages")));
        }
        let per_page = if vertical { 1 } else { d.page(page).map_or(1, |p| p.columns.count.max(1) as usize) };
        let pages = want.div_ceil(per_page).clamp(1, max_pages - added);
        let parent = d.page_loc(page).and_then(|(si, pi)| d.spreads[si].pages[pi].parent);
        new_pages.extend(d.insert_pages(Some(page), pages, parent)?);
        for pg in page + 1..=page + pages {
            let (si, pi) = d.page_loc(pg).ok_or_else(|| bad(cmd, "page insert failed"))?;
            let columns = d.spreads[si].pages[pi].columns.clone();
            for r in flow_rects(d, dir, vertical, pg) {
                prev = add_flow_frame(d, si, r, layer, prev)?;
                made.push(prev);
                if primary && let Some(tf) = d.item_mut(prev).and_then(|it| it.text_frame_mut()) {
                    tf.options.columns = columns.count.max(1);
                    tf.options.gutter = columns.gutter;
                }
            }
        }
        added += pages;
    }
    let cs = designcraft_compose::compose_story(d, sid, &Default::default());
    while let Some(&f) = made.last() {
        let frames = d.story(sid).map_or(&[][..], |s| s.frames.as_slice());
        let [.., before, tail] = frames else { break };
        if *tail != f || cs.frame(f).is_some_and(|ft| !ft.range.is_empty()) {
            break;
        }
        let page = d.page_of_item(f);
        if d.page_of_item(*before) != page {
            // The first frame on its page: the estimate added more pages than the text needs.
            // Drop the page too when autoflow added it and the frame is all it holds.
            let Some(pg) = page.filter(|pg| d.page(*pg).is_some_and(|p| new_pages.contains(&p.id))) else { break };
            let alone = d.spreads.iter().flat_map(|sp| sp.items.iter()).all(|it| it.id == f || d.page_of_item(it.id) != Some(pg));
            if !alone {
                break;
            }
            d.remove_item(f)?;
            d.delete_pages(&[pg])?;
            added = added.saturating_sub(1);
        } else {
            d.remove_item(f)?;
        }
        made.pop();
    }
    Ok(added)
}

fn add_flow_frame(
    d: &mut Document,
    si: usize,
    r: Rect,
    layer: designcraft_doc::LayerId,
    prev: designcraft_doc::ItemId,
) -> Result<designcraft_doc::ItemId> {
    let (fid, _) = d.add_text_frame(SpreadRef::Doc(si), r, layer, "", ParaFormat::default())?;
    d.thread(prev, fid)?;
    Ok(fid)
}

pub(super) fn place_text(
    s: &mut Session,
    p: &Value,
    name: &str,
    bytes: &[u8],
    path: Option<String>,
    before: Option<designcraft_doc::Fingerprint>,
) -> Result<Value> {
    let workbook = workbook_param(p, "file.place")?;
    let mut imp = import_part(name, bytes, &workbook).map_err(|e| bad("file.place", e))?;
    if !workbook.table_style.is_empty() && !s.doc()?.doc.styles.table.iter().any(|t| t.name == workbook.table_style) {
        return Err(bad("file.place", format!("no table style `{}`", workbook.table_style)));
    }
    style_tables(&s.doc()?.doc, &mut imp, &workbook.table_style);
    let Options { map, rules, conflict, remove } = options(s, p, "file.place")?;
    if remove {
        strip(&mut imp.story);
        imp.para_styles.clear();
        imp.char_styles.clear();
    }
    let map = apply_options(&s.doc()?.doc, &mut imp, &map, &rules, conflict);
    // Create Links When Placing Text Files (only a file on disk can be linked).
    let link = path.filter(|_| p.get("link").and_then(Value::as_bool).unwrap_or(s.prefs.link_text_files));
    // A file too large for the Links panel to check isn't linked.
    let (link, too_large) = match link {
        Some(_) if !super::links::checkable_size(bytes.len()) => (None, true),
        l => (l, false),
    };
    let autoflow_on = p.get("autoflow").and_then(Value::as_bool).unwrap_or(false);
    let st = s.doc()?;
    let caret = st.selection.text.filter(|t| t.cell.is_none());
    let frame = super::id_param(p, "frame").or_else(|| {
        st.selection.items.iter().copied().find(|i| st.doc.item(*i).is_some_and(|it| matches!(it.content, Content::Unassigned | Content::Text(_))))
    });
    let page = p.get("page").and_then(Value::as_u64).map(|v| (v as usize).saturating_sub(1));
    let rect = super::rect_param(p, "rect");
    let mut warnings = imp.warnings.clone();
    warnings.extend(imp.problems.iter().map(designcraft_textimport::Problem::message));
    if too_large {
        warnings.push("not linked: the file is too large to keep linked".into());
    }
    s.edit(|d, sel| {
        add_styles(d, &imp, conflict);
        let sid = if let Some(t) = caret {
            // Into the text at the insertion point (replacing the selection).
            let story = d.story_mut(t.story).ok_or(designcraft_doc::DocError::NoStory(t.story))?;
            let r = t.range();
            story.delete(r.clone());
            // An insertion point in an empty paragraph takes the first imported paragraph's style
            // (insert_story would keep the empty paragraph's own, losing the styleMap result).
            let p0 = story.para_at(r.start);
            let blank = story.para_ranges().get(p0).is_some_and(|x| x.is_empty());
            let end = story.insert_story(r.start, &imp.story);
            if blank && let (Some(first), Some(p)) = (imp.story.paras.first(), story.paras.get_mut(p0)) {
                *p = ParaFormat { table: p.table, ..first.clone() };
            }
            sel.text = Some(TextSel { anchor: end, focus: end, ..t });
            if link.is_some() {
                warnings.push("not linked: a linked text file fills a whole story, not an insertion point".into());
            }
            t.story
        } else {
            let fid = match frame {
                Some(f) => f,
                None => {
                    let pg = page.unwrap_or(0).min(d.page_count().saturating_sub(1));
                    let (si, pi) = d.page_loc(pg).ok_or_else(|| bad("file.place", "no such page"))?;
                    // An autoflowed story starts in the first column; a placed one on the live area.
                    let first_column = autoflow_on.then(|| flow_rects(d, d.new_story_direction(), false, pg).first().copied()).flatten();
                    let r: Rect = rect.or(first_column).unwrap_or_else(|| d.spreads[si].pages[pi].margin_rect());
                    let layer = d.default_layer();
                    d.add_text_frame(SpreadRef::Doc(si), r, layer, "", ParaFormat::default())?.0
                }
            };
            // Empty frames become text frames.
            let sid = match d.item(fid).map(|i| i.content.clone()) {
                Some(Content::Text(t)) => t.story,
                Some(_) => {
                    let sid = StoryId(d.alloc());
                    let mut ns = Story::new(sid);
                    ns.direction = d.new_story_direction();
                    ns.frames = vec![fid];
                    d.stories.insert(sid, std::sync::Arc::new(ns));
                    if let Some(it) = d.item_mut(fid) {
                        it.content = Content::Text(designcraft_doc::TextFrame { story: sid, options: Default::default() });
                    }
                    sid
                }
                None => return Err(bad("file.place", "no such frame")),
            };
            let story = d.story_mut(sid).ok_or(designcraft_doc::DocError::NoStory(sid))?;
            fill(story, &imp.story);
            story.file_link = link.as_deref().map(|path| TextFileLink {
                workbook: workbook.clone(),
                style_rules: rules.clone(),
                ..text_link(path, bytes, before, map.clone(), conflict, remove, story)
            });
            *sel = designcraft_doc::Selection::items(vec![fid]);
            sid
        };
        let pages = if autoflow_on { autoflow(d, sid, 2000, "file.place", false)? } else { 0 };
        let overset = fit(d, sid).0.is_some();
        Ok(json!({"story": sid.0, "pagesAdded": pages, "overset": overset, "warnings": warnings}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_text_file_with_autoflow() {
        let dir = std::env::temp_dir().join(format!("dc-place-text-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("long.txt");
        let para = "The quick brown fox jumps over the lazy dog, again and again, to fill the page. ".repeat(12);
        std::fs::write(&path, vec![para.as_str(); 60].join("\n")).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "autoflow": true})).unwrap();
        assert!(r["pagesAdded"].as_u64().unwrap() >= 2, "{r}");
        assert_eq!(r["overset"], false);
        let d = &s.doc().unwrap().doc;
        let sid = StoryId(r["story"].as_u64().unwrap());
        assert_eq!(d.story(sid).unwrap().frames.len(), d.page_count());
        assert_eq!(d.story(sid).unwrap().paras.len(), 60);
        // Into the insertion point of another frame.
        let fr = s.execute("frame.create", &json!({"rect": [72, 72, 300, 200], "content": "text", "text": "Start  end"})).unwrap();
        let sid2 = fr["story"].as_u64().unwrap();
        s.execute("text.select", &json!({"story": sid2, "anchor": 6, "focus": 6})).unwrap();
        let small = dir.join("small.rtf");
        std::fs::write(&small, br"{\rtf1 {\b middle}}").unwrap();
        s.execute("file.place", &json!({"path": small.to_string_lossy()})).unwrap();
        let st = s.doc().unwrap().doc.story(StoryId(sid2)).unwrap().clone();
        assert_eq!(st.text, "Start middle end");
        assert_eq!(st.format_after(6).over.font_style.as_deref(), Some("Bold"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autoflow_sets_one_frame_per_column() {
        // Regression (#625): autoflow set one frame across the margins, over the gutter.
        let dir = std::env::temp_dir().join(format!("dc-place-columns-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("long.txt");
        let para = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(12);
        std::fs::write(&path, vec![para.as_str(); 40].join("\n")).unwrap();
        for binding in ["leftToRight", "rightToLeft"] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"preset": "Letter", "pages": 1, "columns": 2, "gutter": 18, "margins": 54})).unwrap();
            s.execute("layout.documentSetup", &json!({"binding": binding})).unwrap();
            let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "page": 1, "autoflow": true})).unwrap();
            assert_eq!(r["overset"], false, "{r}");
            let d = &s.doc().unwrap().doc;
            let frames = &d.story(StoryId(r["story"].as_u64().unwrap())).unwrap().frames;
            assert!(d.page_count() >= 3, "{binding}: {r}");
            // Two frames on every page but maybe the last, in column order, each on its column.
            assert!(frames.len() > 2 * (d.page_count() - 1), "{binding}: {} frames", frames.len());
            for (i, f) in frames.iter().enumerate() {
                let page = d.page_of_item(*f).unwrap();
                assert_eq!(page, i / 2, "{binding}: frame {i}");
                let mut cols = d.page(page).unwrap().column_rects();
                if binding == "rightToLeft" {
                    cols.reverse();
                }
                assert_eq!(d.item(*f).unwrap().bounds(), cols[i % 2], "{binding}: frame {i}");
            }
        }
        // One column: one frame on the live area per page, as before.
        let mut s = Session::new();
        s.execute("file.new", &json!({"preset": "Letter", "pages": 1, "margins": 54})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "page": 1, "autoflow": true})).unwrap();
        let d = &s.doc().unwrap().doc;
        let frames = &d.story(StoryId(r["story"].as_u64().unwrap())).unwrap().frames;
        assert_eq!(frames.len(), d.page_count());
        for f in frames {
            assert_eq!(d.item(*f).unwrap().bounds(), d.page(d.page_of_item(*f).unwrap()).unwrap().margin_rect());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autoflow_keeps_the_frame_the_user_drew() {
        let dir = std::env::temp_dir().join(format!("dc-place-drawn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("long.txt");
        std::fs::write(&path, vec!["Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(12); 30].join("\n")).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({"preset": "Letter", "pages": 1, "columns": 2, "gutter": 18, "margins": 54})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "rect": [100, 100, 400, 300], "autoflow": true})).unwrap();
        assert_eq!(r["overset"], false, "{r}");
        let d = &s.doc().unwrap().doc;
        let frames = &d.story(StoryId(r["story"].as_u64().unwrap())).unwrap().frames;
        assert_eq!(d.item(frames[0]).unwrap().bounds(), Rect::new(100.0, 100.0, 400.0, 300.0));
        let cols = d.page(1).unwrap().column_rects();
        assert_eq!(d.page_of_item(frames[1]), Some(1));
        assert_eq!(d.item(frames[1]).unwrap().bounds(), cols[0]);
        assert_eq!(d.item(frames[2]).unwrap().bounds(), cols[1]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autoflow_adds_only_the_pages_breaks_ask_for() {
        // Regression (#262): a frame a break ends early made autoflow take it for a frame that
        // holds a few characters, and add a page per few characters left (33 for this text).
        let dir = std::env::temp_dir().join(format!("dc-place-breaks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, rtf, pages) in [
            ("lead.rtf", r"{\rtf1 \page\par A Short Heading\par First short paragraph.\par Second short paragraph.\par}", 2),
            ("each.rtf", r"{\rtf1 One\par\page\par Two\par\page\par Three\par\page\par Four\par}", 4),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, rtf).unwrap();
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "autoflow": true})).unwrap();
            assert_eq!(r["overset"], false, "{name}: {r}");
            let d = &s.doc().unwrap().doc;
            assert_eq!(d.page_count(), pages, "{name}: {r}");
            assert_eq!(d.story(StoryId(r["story"].as_u64().unwrap())).unwrap().frames.len(), pages, "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autoflow_adds_the_pages_odd_and_even_page_breaks_ask_for() {
        // From page 1, an odd page break needs pages 2 and 3 (2 stays empty), an even one page 2.
        // A frame such a break ends early holds few characters; taken for a full frame, it made
        // autoflow add a page per few characters left.
        use designcraft_doc::{EVEN_PAGE_BREAK, ODD_PAGE_BREAK, build::NewDocument};
        for (brk, added) in [(ODD_PAGE_BREAK, 2), (EVEN_PAGE_BREAK, 1)] {
            let mut d = Document::new(&NewDocument { pages: 1, facing_pages: false, primary_text_frame: true, ..Default::default() });
            let sid = d.settings.primary_story.unwrap();
            d.story_mut(sid).unwrap().insert(
                0,
                &format!(
                    "a{brk}
{}",
                    "word ".repeat(200)
                ),
            );
            assert_eq!(autoflow(&mut d, sid, 50, "file.place", false).unwrap(), added, "{brk:?}");
            assert_eq!(d.page_count(), added + 1, "{brk:?}");
            assert_eq!(designcraft_compose::compose_story(&d, sid, &Default::default()).overset_at, None, "{brk:?}");
        }
    }
}

#[cfg(test)]
mod place_pdf_tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn place_pdf_renders_and_exports_as_vector() {
        let dir = std::env::temp_dir().join(format!("dc-place-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("ad.pdf");
        // A PDF made by our own exporter: a black square on a page.
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let id = s.execute("frame.create", &json!({"rect": [0, 0, 100, 100]})).unwrap()["id"].as_u64().unwrap();
        s.execute("object.fill", &json!({"ids": [id], "swatch": "[Black]"})).unwrap();
        s.execute("file.exportPdf", &json!({"path": src.to_string_lossy()})).unwrap();
        // Place it in a new document.
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("file.place", &json!({"path": src.to_string_lossy(), "x": 100, "y": 100})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        let it = d.spreads[0].items.last().unwrap();
        let b = it.bounds();
        assert!((b.width() - 200.0).abs() < 1.0 && (b.height() - 100.0).abs() < 1.0, "sized by the PDF page: {b:?} {r}");
        let asset = d.assets.values().next().unwrap();
        assert_eq!(asset.mime, "application/pdf");
        // On screen: the square is black, the rest of the placed page white.
        let mut rr = designcraft_render::Renderer::new();
        let img = rr.render_page(&d, &s.cache, 0, 1.0, false, &Default::default()).unwrap();
        assert!(img.pixel(150, 150)[0] < 90, "{:?}", img.pixel(150, 150));
        assert!(img.pixel(250, 150)[0] > 200, "{:?}", img.pixel(250, 150));
        // Exported: still a valid PDF, larger than a page without the placed PDF would be.
        let out = dir.join("out.pdf");
        s.execute("file.exportPdf", &json!({"path": out.to_string_lossy()})).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let pdf = hayro_syntax::Pdf::new(std::sync::Arc::new(bytes.clone())).expect("valid pdf");
        assert_eq!(pdf.pages().len(), 1);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Subtype /Form") || text.contains("/Subtype/Form"), "embedded as a form XObject");
        assert!(!text.contains("/Subtype /Image") && !text.contains("/Subtype/Image"), "not rasterized");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn place_svg_on_screen_and_as_vectors_in_pdf() {
        let dir = std::env::temp_dir().join(format!("dc-place-svg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("logo.svg");
        std::fs::write(
            &src,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="80"><rect width="80" height="80" fill="#0000ff"/></svg>"##,
        )
        .unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("file.place", &json!({"path": src.to_string_lossy(), "x": 100, "y": 100})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        let b = d.spreads[0].items.last().unwrap().bounds();
        assert!((b.width() - 120.0).abs() < 1.0 && (b.height() - 60.0).abs() < 1.0, "160×80 px = 120×60 pt: {b:?}");
        assert_eq!(d.assets.values().next().unwrap().mime, "image/svg+xml");
        let mut rr = designcraft_render::Renderer::new();
        let img = rr.render_page(&d, &s.cache, 0, 1.0, false, &Default::default()).unwrap();
        let p = img.pixel(130, 130);
        assert!(p[2] > 200 && p[0] < 60, "blue square on screen: {p:?}");
        let out = dir.join("out.pdf");
        s.execute("file.exportPdf", &json!({"path": out.to_string_lossy()})).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        hayro_syntax::Pdf::new(std::sync::Arc::new(bytes.clone())).expect("valid pdf");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("/Subtype /Image") && !text.contains("/Subtype/Image"), "not rasterized");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod reflow_tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn smart_text_reflow_adds_and_removes_pages() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1, "primaryTextFrame": true})).unwrap();
        let sid = s.doc().unwrap().doc.settings.primary_story.expect("primary story").0;
        assert_eq!(s.doc().unwrap().doc.story(designcraft_doc::StoryId(sid)).unwrap().frames.len(), 1);
        s.execute("text.select", &json!({"story": sid, "anchor": 0, "focus": 0})).unwrap();
        let long = "Words keep coming and the page fills up quickly with them. ".repeat(250);
        s.execute("text.insert", &json!({"text": long})).unwrap();
        let pages = s.doc().unwrap().doc.page_count();
        assert!(pages >= 3, "pages added: {pages}");
        let d = s.doc().unwrap().doc.clone();
        assert_eq!(d.story(designcraft_doc::StoryId(sid)).unwrap().frames.len(), pages);
        // One undo takes the typing and its pages back.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.page_count(), 1);
        s.execute("edit.redo", &json!({})).unwrap();
        // Delete the text: the empty pages at the end go.
        let n = s.doc().unwrap().doc.story(designcraft_doc::StoryId(sid)).unwrap().len();
        s.execute("text.select", &json!({"story": sid, "anchor": 0, "focus": n})).unwrap();
        s.execute("text.delete", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.page_count(), 1);
        s.doc().unwrap().doc.check().unwrap();
        // Off: no pages added.
        s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
        s.execute("text.insert", &json!({"text": long})).unwrap();
        assert_eq!(s.doc().unwrap().doc.page_count(), 1);
    }

    #[test]
    fn smart_text_reflow_adds_primary_frames_with_the_page_columns() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1, "columns": 2, "gutter": 18, "primaryTextFrame": true})).unwrap();
        let sid = s.doc().unwrap().doc.settings.primary_story.expect("primary story");
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("text.insert", &json!({"text": "Words keep coming and the page fills up quickly with them. ".repeat(250)})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        let frames = &d.story(sid).unwrap().frames;
        assert!(d.page_count() >= 2);
        assert_eq!(frames.len(), d.page_count());
        for f in frames {
            let it = d.item(*f).unwrap();
            assert_eq!(it.bounds(), d.page(d.page_of_item(*f).unwrap()).unwrap().margin_rect());
            let tf = it.text_frame().unwrap();
            assert_eq!((tf.options.columns, tf.options.gutter), (2, 18.0));
        }
        let n = d.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": n})).unwrap();
        s.execute("text.delete", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.page_count(), 1);
    }

    /// #165: deleting the pages or frames of a primary story doesn't make more pages. A page the
    /// text still needs comes back once, as with InDesign's Smart Text Reflow; a frame's page stays,
    /// empty, and one page comes for the text the frame held; pages the text no longer needs delete
    /// by hand and stay deleted; with Smart Text Reflow off, a page with text deletes and the story
    /// oversets.
    #[test]
    fn deleting_pages_or_frames_of_a_primary_story_adds_no_pages() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"preset": "Letter", "pages": 1, "primaryTextFrame": true})).unwrap();
        let sid = s.doc().unwrap().doc.settings.primary_story.expect("primary story");
        let words = "Words keep coming and the page fills up quickly with them. ".repeat(250);
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("text.insert", &json!({"text": words})).unwrap();
        let pages = |s: &Session| s.doc().unwrap().doc.page_count();
        let overset = |s: &Session| designcraft_compose::compose_story(&s.doc().unwrap().doc, sid, &Default::default()).overset_at.is_some();
        let n = pages(&s);
        assert!(n >= 3, "{n} pages");
        s.execute("layout.pages.delete", &json!({"pages": [n - 1]})).unwrap();
        assert_eq!((pages(&s), overset(&s)), (n, false), "the last page");
        s.execute("layout.pages.delete", &json!({"pages": [1]})).unwrap();
        assert_eq!((pages(&s), overset(&s)), (n, false), "a middle page");
        let last = *s.doc().unwrap().doc.story(sid).unwrap().frames.last().unwrap();
        s.execute("edit.clear", &json!({"ids": [last.0]})).unwrap();
        assert_eq!((pages(&s), overset(&s)), (n + 1, false), "the last frame");
        let len = s.doc().unwrap().doc.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 400, "focus": len})).unwrap();
        s.execute("text.delete", &json!({})).unwrap();
        let emptied = pages(&s);
        s.execute("layout.pages.delete", &json!({"pages": (1..emptied).collect::<Vec<_>>()})).unwrap();
        s.execute("frame.create", &json!({"rect": [100, 650, 300, 700]})).unwrap();
        assert_eq!((pages(&s), overset(&s)), (1, false), "the emptied pages stay deleted");
        s.doc().unwrap().doc.check().unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 400, "focus": 400})).unwrap();
        s.execute("text.insert", &json!({"text": words})).unwrap();
        let n = pages(&s);
        s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
        s.execute("layout.pages.delete", &json!({"pages": [n - 1]})).unwrap();
        assert_eq!((pages(&s), overset(&s)), (n - 1, true), "Smart Text Reflow off");
    }

    /// Placing text into the primary frame adds only the pages the text needs, also when a wrap
    /// leaves page 1 little room or the text ends at a page boundary (#1165).
    #[test]
    fn smart_text_reflow_place_adds_no_empty_pages() {
        let dir = std::env::temp_dir().join(format!("dc-reflow-place-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (n, wrap, want) in [(280, true, 5), (250, false, 3)] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"preset": "Letter", "pages": 1, "primaryTextFrame": true})).unwrap();
            if wrap {
                let id = s.execute("frame.create", &json!({"rect": [36, 36, 576, 600]})).unwrap()["id"].as_u64().unwrap();
                s.execute("object.textWrap", &json!({"ids": [id], "mode": "boundingBox"})).unwrap();
            }
            let sid = s.doc().unwrap().doc.settings.primary_story.expect("primary story");
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
            let path = dir.join(format!("words-{n}.txt"));
            std::fs::write(&path, "Words keep coming and the page fills up quickly with them. ".repeat(n)).unwrap();
            s.execute("file.place", &json!({"path": path.to_string_lossy()})).unwrap();
            let d = s.doc().unwrap().doc.clone();
            assert_eq!(d.page_count(), want, "{n} lines");
            let frames = &d.story(sid).unwrap().frames;
            assert_eq!(frames.len(), d.page_count(), "{n} lines");
            let cs = designcraft_compose::compose_story(&d, sid, &Default::default());
            assert!(cs.overset_at.is_none(), "{n} lines");
            for f in frames {
                assert!(cs.frame(*f).is_some_and(|ft| !ft.range.is_empty()), "{n} lines: empty frame {f:?}");
            }
            d.check().unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod pdf_page_tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn place_a_chosen_pdf_page() {
        let dir = std::env::temp_dir().join(format!("dc-pdf-page-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("two.pdf");
        // Page 1 white, page 2 a different size with a black square.
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 2, "facingPages": false, "width": 200, "height": 100})).unwrap();
        s.execute("layout.pageSize", &json!({"pages": [2], "width": 300, "height": 150})).unwrap();
        let id = s.execute("frame.create", &json!({"spread": 1, "rect": [0, 0, 300, 150]})).unwrap()["id"].as_u64().unwrap();
        s.execute("object.fill", &json!({"ids": [id], "swatch": "[Black]"})).unwrap();
        s.execute("file.exportPdf", &json!({"path": src.to_string_lossy()})).unwrap();
        s.execute("file.new", &json!({})).unwrap();
        assert!(s.execute("file.place", &json!({"path": src.to_string_lossy(), "pdfPage": 3})).is_err());
        s.execute("file.place", &json!({"path": src.to_string_lossy(), "pdfPage": 2, "x": 100, "y": 100})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        let it = d.spreads[0].items.last().unwrap();
        let b = it.bounds();
        assert!((b.width() - 300.0).abs() < 1.0 && (b.height() - 150.0).abs() < 1.0, "page 2's size: {b:?}");
        assert_eq!(d.assets.values().next().unwrap().page, 1);
        let mut rr = designcraft_render::Renderer::new();
        rr.threads = 0;
        let img = rr.render_page(&d, &s.cache, 0, 1.0, false, &Default::default()).unwrap();
        assert!(img.pixel(250, 170)[0] < 60, "page 2 (black) shows: {:?}", img.pixel(250, 170));
        let pdf = designcraft_pdf::export_pdf(&d, &s.cache, &Default::default()).unwrap();
        assert!(String::from_utf8_lossy(&pdf).contains("/Subtype /Form") || String::from_utf8_lossy(&pdf).contains("/Subtype/Form"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn word_file() -> Vec<u8> {
        use std::io::Write;
        const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let styles = format!(
            r#"<w:styles {W}><w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
            <w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:rPr><w:sz w:val="56"/></w:rPr></w:style>
            <w:style w:type="paragraph" w:styleId="Body"><w:name w:val="Body"/><w:rPr><w:sz w:val="20"/></w:rPr></w:style></w:styles>"#
        );
        let doc = format!(
            r#"<w:document {W}><w:body><w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Head</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="Body"/></w:pPr><w:r><w:t>Text</w:t></w:r></w:p></w:body></w:document>"#
        );
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        for (n, c) in [("word/document.xml", doc), ("word/styles.xml", styles)] {
            w.start_file(n, o).unwrap();
            w.write_all(c.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn word_import_options_map_and_rename_styles() {
        let b64 = crate::cmd::base64_encode(&word_file());
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("style.paragraph.create", &json!({"name": "Body", "chars": {"size": 9}})).unwrap();
        s.execute("style.paragraph.create", &json!({"name": "Headline", "chars": {"size": 30}})).unwrap();
        let info = s.execute("place.styles", &json!({"base64": b64, "name": "a.docx"})).unwrap();
        assert_eq!(info["conflicts"], json!(["Body"]));
        // Title → the document's Headline; Body clashes → imported as Body_wrd_1.
        let r = s
            .execute("file.place", &json!({"base64": b64, "name": "a.docx", "styleMap": {"Title": "Headline"}, "styleConflicts": "autoRename"}))
            .unwrap();
        let d = &s.doc().unwrap().doc;
        let st = d.story(designcraft_doc::StoryId(r["story"].as_u64().unwrap())).unwrap();
        assert_eq!(st.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Headline", "Body_wrd_1"]);
        assert!(d.styles.para("Title").is_none(), "mapped styles aren't imported");
        assert_eq!(d.styles.para("Body").unwrap().chars.size, Some(9.0), "the document's Body is untouched");
        // Redefine: the imported definition replaces the document's.
        s.execute("file.place", &json!({"base64": b64, "name": "a.docx", "styleConflicts": "redefine", "rect": [72, 400, 300, 500]})).unwrap();
        assert_eq!(s.doc().unwrap().doc.styles.para("Body").unwrap().chars.size, Some(10.0));
    }

    #[test]
    fn word_first_paragraph_into_empty_caret_keeps_mapped_style() {
        // Regression (#1208): placing into a new text frame (which leaves its caret selected) left the first paragraph unstyled.
        let b64 = crate::cmd::base64_encode(&word_file());
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("style.paragraph.create", &json!({"name": "Headline"})).unwrap();
        let fr = s.execute("frame.create", &json!({"rect": [72, 72, 300, 200], "content": "text"})).unwrap();
        let sid = fr["story"].as_u64().unwrap();
        s.execute("file.place", &json!({"base64": b64, "name": "a.docx", "frame": fr["id"], "styleMap": {"Title": "Headline"}})).unwrap();
        let st = s.doc().unwrap().doc.story(designcraft_doc::StoryId(sid)).unwrap().clone();
        assert_eq!(st.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Headline", "Body"]);
    }

    #[test]
    fn word_normal_comes_in_as_its_own_style() {
        // Regression (#722): Normal's formatting (over Word's document defaults) was dropped.
        use std::io::Write;
        const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let styles = format!(
            r#"<w:styles {W}><w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults>
            <w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:rFonts w:ascii="Georgia"/><w:sz w:val="26"/></w:rPr></w:style></w:styles>"#
        );
        let doc = format!(r#"<w:document {W}><w:body><w:p><w:r><w:t>Plain</w:t></w:r></w:p></w:body></w:document>"#);
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, c) in [("word/document.xml", doc), ("word/styles.xml", styles)] {
            w.start_file(n, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(c.as_bytes()).unwrap();
        }
        let b64 = crate::cmd::base64_encode(&w.finish().unwrap().into_inner());
        let basic = designcraft_doc::story::BASIC_PARAGRAPH;
        let placed = |s: &Session, story: &serde_json::Value| {
            let d = &s.doc().unwrap().doc;
            let st = d.story(designcraft_doc::StoryId(story["story"].as_u64().unwrap())).unwrap();
            let (_, para_chars) = d.styles.resolve_para(&st.paras[0]);
            let c = d.styles.resolve_char(&para_chars, st.format_after(0));
            (st.paras[0].style.clone(), c.size, c.font_family)
        };
        let georgia = |style: &str| (style.to_string(), 13.0, "Georgia".to_string());
        // Default options: the document gains Normal and the text looks as it did in Word.
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let info = s.execute("place.styles", &json!({"base64": b64, "name": "a.docx"})).unwrap();
        assert_eq!(info["paragraph"], json!(["Normal"]));
        assert_eq!(info["conflicts"], json!([]));
        let r = s.execute("file.place", &json!({"base64": b64, "name": "a.docx"})).unwrap();
        assert_eq!(placed(&s, &r), georgia("Normal"));
        assert!(s.doc().unwrap().doc.styles.para(basic).unwrap().chars.is_empty(), "[Basic Paragraph] untouched");
        // Normal mapped to [Basic Paragraph]: no new style, and the document's definition is used
        // (mapped styles aren't imported, whatever the conflict option).
        s.execute("file.new", &json!({})).unwrap();
        let r =
            s.execute("file.place", &json!({"base64": b64, "name": "a.docx", "styleMap": {"Normal": basic}, "styleConflicts": "redefine"})).unwrap();
        assert!(s.doc().unwrap().doc.styles.para("Normal").is_none());
        assert!(s.doc().unwrap().doc.styles.para(basic).unwrap().chars.is_empty());
        let (style, size, _) = placed(&s, &r);
        assert_eq!((style.as_str(), size), (basic, 12.0));
        // A document with its own Normal: the usual conflict rules.
        s.execute("style.paragraph.create", &json!({"name": "Normal", "chars": {"size": 9}})).unwrap();
        let r = s.execute("file.place", &json!({"base64": b64, "name": "a.docx", "rect": [72, 400, 300, 500]})).unwrap();
        assert_eq!(placed(&s, &r).1, 9.0, "use existing");
        let r =
            s.execute("file.place", &json!({"base64": b64, "name": "a.docx", "styleConflicts": "autoRename", "rect": [72, 520, 300, 600]})).unwrap();
        assert_eq!(placed(&s, &r), georgia("Normal_wrd_1"));
    }
}

#[cfg(test)]
mod style_rule_tests {
    use super::*;

    /// A Word file with six styles, each used: paragraph styles Normal (10 pt), heading 1 (on
    /// Normal, bold, 16 pt), heading 2 (on heading 1, 13 pt) and Quote (on Normal, italic);
    /// character styles Strong (bold) and Emphasis (italic). The Normal paragraph is centred and
    /// ends in a 15 pt run of its own; the Quote paragraph is right-aligned. `extra`: a heading 3
    /// (on heading 1) paragraph at the end.
    fn six_styles(extra: bool) -> Vec<u8> {
        use std::io::Write;
        const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let (h3_style, h3) = if extra {
            (
                r#"<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Heading1"/><w:rPr><w:sz w:val="22"/></w:rPr></w:style>"#,
                r#"<w:p><w:pPr><w:pStyle w:val="Heading3"/></w:pPr><w:r><w:t>Sub</w:t></w:r></w:p>"#,
            )
        } else {
            ("", "")
        };
        let styles = format!(
            r#"<w:styles {W}><w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:sz w:val="20"/></w:rPr></w:style>
            <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
            <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Heading1"/><w:rPr><w:sz w:val="26"/></w:rPr></w:style>
            <w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:rPr><w:i/></w:rPr></w:style>
            <w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/></w:rPr></w:style>
            <w:style w:type="character" w:styleId="Emphasis"><w:name w:val="Emphasis"/><w:rPr><w:i/></w:rPr></w:style>{h3_style}</w:styles>"#
        );
        let doc = format!(
            r#"<w:document {W}><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Section</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="Normal"/><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">Plain </w:t></w:r><w:r><w:rPr><w:rStyle w:val="Strong"/></w:rPr><w:t>strong</w:t></w:r><w:r><w:t xml:space="preserve"> and </w:t></w:r><w:r><w:rPr><w:rStyle w:val="Emphasis"/></w:rPr><w:t>em</w:t></w:r><w:r><w:rPr><w:sz w:val="30"/></w:rPr><w:t xml:space="preserve"> big</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="Quote"/><w:jc w:val="right"/></w:pPr><w:r><w:t>Said</w:t></w:r></w:p>{h3}</w:body></w:document>"#
        );
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        for (n, c) in [("word/document.xml", doc), ("word/styles.xml", styles)] {
            w.start_file(n, o).unwrap();
            w.write_all(c.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    /// A new document of `s` with paragraph styles Head, Body and its own Quote (11 pt), and
    /// character styles Bold and Em.
    fn magazine(s: &mut Session) {
        s.execute("file.new", &json!({})).unwrap();
        for (name, size) in [("Head", 20), ("Body", 9), ("Quote", 11)] {
            s.execute("style.paragraph.create", &json!({"name": name, "chars": {"size": size}})).unwrap();
        }
        for name in ["Bold", "Em"] {
            s.execute("style.character.create", &json!({"name": name})).unwrap();
        }
    }

    fn magazine_session() -> Session {
        let mut s = Session::new();
        magazine(&mut s);
        s
    }

    /// The placed story's paragraph styles, and its character-styled runs (text, style).
    fn placed_styles(s: &Session, r: &Value) -> (Vec<String>, Vec<(String, String)>) {
        let st = s.doc().unwrap().doc.story(StoryId(r["story"].as_u64().unwrap())).unwrap().clone();
        let runs = st
            .runs()
            .filter(|(_, f)| f.style != designcraft_doc::NO_CHAR_STYLE)
            .map(|(rg, f)| (st.text.get(rg).unwrap_or_default().to_string(), f.style.clone()))
            .collect();
        (st.paras.iter().map(|p| p.style.clone()).collect(), runs)
    }

    fn six_b64() -> String {
        crate::cmd::base64_encode(&six_styles(false))
    }

    #[test]
    fn glob_matches_names_ignoring_case() {
        for (pattern, name, yes) in [
            ("heading*", "Heading 1", true),
            ("heading ?", "heading 1", true),
            ("heading ?", "heading 12", false),
            ("*", "", true),
            ("", "", true),
            ("", "a", false),
            ("a*b*c", "aXXbYYc", true),
            ("a*b*c", "aXXbYY", false),
            ("*title", "Subtitle", true),
            ("?*", "", false),
            ("**", "x", true),
            ("ÉTÉ*", "été 2026", true),
            ("normal", "Normal", true),
        ] {
            assert_eq!(glob(pattern, name), yes, "`{pattern}` and `{name}`");
        }
    }

    /// Rules by name: the headings become Head, Normal Body and Strong Bold; Quote and Emphasis,
    /// which no rule matches, follow Style Name Conflicts as before.
    #[test]
    fn rules_map_word_styles_by_name() {
        let mut s = magazine_session();
        let rules = json!([{"match": "heading*", "kind": "paragraph", "to": "Head"}, {"match": "normal", "to": "Body"}, {"match": "str*", "kind": "character", "to": "Bold"}]);
        let r = s.execute("file.place", &json!({"base64": six_b64(), "name": "a.docx", "styleRules": rules})).unwrap();
        let (paras, runs) = placed_styles(&s, &r);
        assert_eq!(paras, ["Head", "Head", "Body", "Quote"]);
        assert_eq!(runs, [("strong".to_string(), "Bold".to_string()), ("em".to_string(), "Emphasis".to_string())]);
        let st = &s.doc().unwrap().doc.styles;
        assert!(st.para("heading 1").is_none() && st.para("Normal").is_none() && st.char_style("Strong").is_none(), "mapped styles aren't imported");
        assert_eq!(st.para("Quote").unwrap().chars.size, Some(11.0), "Quote clashes: the document's (useExisting)");
        assert!(st.char_style("Emphasis").is_some(), "Emphasis is new: imported");
    }

    /// Rules by attributes, through what each style is based on: bold paragraph styles (heading 2
    /// is, through heading 1) become Head, italic character styles Em. Quote is italic too, but a
    /// paragraph style: the character rule leaves it, though the document has a paragraph Em.
    #[test]
    fn rules_map_word_styles_by_their_attributes() {
        let mut s = magazine_session();
        s.execute("style.paragraph.create", &json!({"name": "Em"})).unwrap();
        let rules = json!([
            {"kind": "paragraph", "attrs": {"chars": {"fontStyle": "Bold"}}, "to": "Head"},
            {"kind": "character", "attrs": {"chars": {"fontStyle": "Italic"}}, "to": "Em"},
            {"match": "normal", "to": "Body"},
        ]);
        let r =
            s.execute("file.place", &json!({"base64": six_b64(), "name": "a.docx", "styleRules": rules, "styleConflicts": "autoRename"})).unwrap();
        let (paras, runs) = placed_styles(&s, &r);
        assert_eq!(paras, ["Head", "Head", "Body", "Quote_wrd_1"], "Quote is italic, not bold: Auto Rename");
        assert_eq!(runs, [("strong".to_string(), "Strong".to_string()), ("em".to_string(), "Em".to_string())]);
    }

    #[test]
    fn a_style_map_entry_beats_the_rules_and_the_first_matching_rule_wins() {
        let mut s = magazine_session();
        s.execute("style.paragraph.create", &json!({"name": "Sub"})).unwrap();
        let rules = json!([
            {"match": "heading*", "to": "Head"},
            {"match": "heading 1", "to": "Body"},
            {"match": "quote", "to": "Nope"},
            {"match": "q*", "kind": "paragraph", "to": "Body"},
        ]);
        let r =
            s.execute("file.place", &json!({"base64": six_b64(), "name": "a.docx", "styleMap": {"heading 2": "Sub"}, "styleRules": rules})).unwrap();
        // heading 1: the first rule; heading 2: the map; Quote: no style Nope here, so the next rule.
        assert_eq!(placed_styles(&s, &r).0, ["Head", "Sub", "Normal", "Body"]);
    }

    /// `keepOverrides: false`: the text a rule maps loses its own formatting (Normal's centring
    /// and its 15 pt run); text the rule doesn't map keeps its own (Quote's alignment).
    #[test]
    fn a_rule_can_clear_the_local_formatting_of_what_it_maps() {
        for keep in [true, false] {
            let mut s = magazine_session();
            let rules = json!([{"match": "normal", "to": "Body", "keepOverrides": keep}]);
            let r = s.execute("file.place", &json!({"base64": six_b64(), "name": "a.docx", "styleRules": rules})).unwrap();
            let st = s.doc().unwrap().doc.story(StoryId(r["story"].as_u64().unwrap())).unwrap().clone();
            let big = st.format_after(st.text.find("big").unwrap()).over.size;
            assert_eq!((st.paras[2].para.align.is_some(), big), (keep, keep.then_some(15.0)), "keepOverrides: {keep}");
            assert!(st.paras[3].para.align.is_some(), "Quote's own alignment stays");
            assert_eq!(st.format_after(st.text.find("strong").unwrap()).style, "Strong", "a run keeps its character style");
        }
    }

    #[test]
    fn style_rules_that_are_not_rules_are_refused() {
        let mut s = magazine_session();
        for rules in [json!({"to": "Body"}), json!([{"to": 3}]), json!([{"match": "a", "kind": "table", "to": "Body"}]), json!([{"match": "a"}])] {
            let e = s.execute("file.place", &json!({"base64": six_b64(), "name": "a.docx", "styleRules": rules})).unwrap_err().to_string();
            assert!(e.contains("styleRules") && !e.contains("unknown parameter"), "{rules}: {e}");
        }
    }

    /// Presets: saved for the user or in the document; a name is looked up in the document first
    /// (case doesn't count), and what a call gives wins over its preset.
    #[test]
    fn import_presets_are_found_in_the_document_first() {
        let mut s = magazine_session();
        s.execute(
            "place.savePreset",
            &json!({"name": "Magazine", "styleRules": [{"match": "heading*", "to": "Head"}], "styleConflicts": "autoRename"}),
        )
        .unwrap();
        s.execute("place.savePreset", &json!({"name": "magazine", "scope": "document", "styleMap": {"heading 1": "Body"}})).unwrap();
        let list = s.execute("place.presets", &json!({})).unwrap();
        let rows: Vec<(String, String)> =
            list.as_array().unwrap().iter().map(|p| (p["name"].as_str().unwrap().to_string(), p["scope"].as_str().unwrap().to_string())).collect();
        assert_eq!(rows, [("magazine".to_string(), "document".to_string()), ("Magazine".to_string(), "user".to_string())]);
        assert_eq!(list[1]["styleRules"], json!([{"match": "heading*", "to": "Head"}]));
        let r = s.execute("file.place", &json!({"base64": six_b64(), "name": "a.docx", "preset": "MAGAZINE"})).unwrap();
        assert_eq!(placed_styles(&s, &r).0[..2], ["Body", "heading 2"], "the document's preset");
        // The document's preset is saved with it.
        let bytes = crate::cmd::base64_decode(s.execute("file.serialize", &json!({})).unwrap()["base64"].as_str().unwrap());
        let mut t = Session::new();
        t.execute("file.openBytes", &json!({"name": "m.designcraft", "base64": crate::cmd::base64_encode(&bytes)})).unwrap();
        assert_eq!(t.execute("place.presets", &json!({})).unwrap()[0]["scope"], "document");
        // Deleted from the document, the name finds the user's: its rules and its Auto Rename.
        s.execute("place.deletePreset", &json!({"name": "Magazine", "scope": "document"})).unwrap();
        magazine(&mut s);
        let r = s.execute("file.place", &json!({"base64": six_b64(), "name": "a.docx", "preset": "Magazine"})).unwrap();
        assert_eq!(placed_styles(&s, &r).0, ["Head", "Head", "Normal", "Quote_wrd_1"]);
        magazine(&mut s);
        let r =
            s.execute("file.place", &json!({"base64": six_b64(), "name": "a.docx", "preset": "Magazine", "styleConflicts": "useExisting"})).unwrap();
        assert_eq!(placed_styles(&s, &r).0[3], "Quote", "the call's styleConflicts wins");
        // Saving a name again replaces that preset.
        s.execute("place.savePreset", &json!({"name": "MAGAZINE", "removeStyles": true})).unwrap();
        let list = s.execute("place.presets", &json!({})).unwrap();
        assert_eq!((list.as_array().unwrap().len(), &list[0]["name"], &list[0]["removeStyles"]), (1, &json!("MAGAZINE"), &json!(true)));
    }

    #[test]
    fn what_is_no_preset_is_refused() {
        let mut s = magazine_session();
        let err = |s: &mut Session, cmd: &str, p: Value| s.execute(cmd, &p).unwrap_err().to_string();
        assert!(err(&mut s, "file.place", json!({"base64": six_b64(), "name": "a.docx", "preset": "Nope"})).contains("no import preset `Nope`"));
        assert!(err(&mut s, "place.savePreset", json!({"name": "  "})).contains("name"));
        assert!(err(&mut s, "place.savePreset", json!({"name": "x".repeat(101)})).contains("100"));
        assert!(err(&mut s, "place.savePreset", json!({"name": "A", "scope": "team"})).contains("scope"));
        assert!(err(&mut s, "place.savePreset", json!({"name": "A", "styleConflicts": "merge"})).contains("styleConflicts"));
        assert!(err(&mut s, "place.savePreset", json!({"name": "A", "styleRules": [{"to": 3}]})).contains("styleRules"));
        assert!(err(&mut s, "place.deletePreset", json!({"name": "Nope"})).contains("no import preset `Nope`"));
        for i in 0..500 {
            s.execute("place.savePreset", &json!({"name": format!("P{i}")})).unwrap();
        }
        assert!(err(&mut s, "place.savePreset", json!({"name": "One more"})).contains("500"));
        s.execute("place.savePreset", &json!({"name": "P7", "removeStyles": true})).unwrap();
        assert!(s.prefs.import_presets.iter().any(|p| p.name == "P7" && p.remove_styles), "replacing one is still possible");
    }

    /// Import Options: what a preset maps of the file's styles, and its other options.
    #[test]
    fn place_styles_shows_what_a_preset_maps_for_the_file() {
        let mut s = magazine_session();
        let preset = json!({"name": "Mag", "styleMap": {"Quote": "Body", "Gone": "Body", "Strong": "Nope"}, "styleRules": [{"match": "heading*", "kind": "paragraph", "to": "Head"}], "styleConflicts": "redefine"});
        s.execute("place.savePreset", &preset).unwrap();
        let info = s.execute("place.styles", &json!({"base64": six_b64(), "name": "a.docx", "preset": "Mag"})).unwrap();
        assert_eq!(
            info["map"],
            json!({"heading 1": "Head", "heading 2": "Head", "Quote": "Body"}),
            "only the file's styles, onto styles this document has"
        );
        assert_eq!((&info["styleConflicts"], &info["removeStyles"]), (&json!("redefine"), &json!(false)));
        assert!(s.execute("place.styles", &json!({"base64": six_b64(), "name": "a.docx"})).unwrap().get("map").is_none());
    }

    /// A linked file's rules are kept on its link: an update, merging or replacing, maps a style
    /// that is new in the file by them, and they stay for the next one.
    #[test]
    fn update_maps_styles_new_in_the_file_by_the_rules() {
        for mode in ["merge", "replace"] {
            let dir = std::env::temp_dir().join(format!("dc-style-rules-{}-{mode}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("story.docx");
            std::fs::write(&path, six_styles(false)).unwrap();
            let mut s = magazine_session();
            let rules = json!([{"match": "heading*", "kind": "paragraph", "to": "Head"}]);
            let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "link": true, "styleRules": rules})).unwrap();
            let sid = StoryId(r["story"].as_u64().unwrap());
            std::fs::write(&path, six_styles(true)).unwrap();
            s.execute("links.update", &json!({"story": sid.0, "mode": mode})).unwrap();
            let d = &s.doc().unwrap().doc;
            let st = d.story(sid).unwrap();
            assert_eq!((st.paras.len(), st.paras.last().unwrap().style.as_str()), (5, "Head"), "{mode}: heading 3 by the rule");
            assert!(d.styles.para("heading 3").is_none(), "{mode}");
            assert_eq!(st.file_link.as_ref().unwrap().style_rules.len(), 1, "{mode}: the rules stay on the link");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
