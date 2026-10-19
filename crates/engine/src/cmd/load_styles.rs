//! Load Styles: the styles of another document (`.designcraft` or IDML) come into this one —
//! paragraph and character styles, and object, table and cell styles when a request names them —
//! with the styles, swatches, stroke styles and lists they name.

use std::collections::{BTreeMap, HashMap, HashSet};

use designcraft_color::{Swatch, SwatchValue};
use designcraft_doc::{
    CellStroke, CellStrokeAttrs, CellStyle, CharAttrs, Document, NO_CELL_STYLE, NO_CHAR_STYLE, NO_OBJECT_STYLE, NO_PARA_STYLE, NO_TABLE_STYLE,
    NumberedList, ObjectStyle, ParaAttrs, ParagraphStyle, StrokeStyleDef, StrokeType, Styles, TableStyle,
};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc, str_param};
use crate::{Result, Session};

const ID: &str = "style.load";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "style.load",
            "Load Styles…",
            [],
            None,
            "{path | base64, name?: the file's name (`.idml` reads IDML), paragraph?: [name] | true, character?: [name] | true, object?: [name] | true, table?: [name] | true, cell?: [name] | true, conflicts?: redefine|useExisting|autoRename} — styles of another document (.designcraft or .idml). Paragraph and character styles load all of their kind unless a list names some (`[]`: none); object, table and cell styles load only when named (`true`: all of the kind). The styles, swatches, stroke styles and lists they name come along where this document has none. A style this document defines differently under the same name takes the loaded definition (redefine, the default), stays as it is (useExisting), or the loaded one gets a new name (autoRename) → {paragraph: {added, replaced, renamed: {name: new name}}, character, object, table, cell: {…}, swatches: [name], strokeStyles: [name]}",
            has_doc,
            load
        ),
        cmd!(query "style.load.list", "List Styles in File", [], None,
            "{path | base64, name?} → {paragraph, character, object, table, cell: [{name, conflict}]} — the styles `style.load` can bring; `conflict`: this document defines a style of that name differently",
            has_doc, list),
    ]
}

/// A kind of style Load Styles brings.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Family {
    Paragraph,
    Character,
    Object,
    Table,
    Cell,
}

/// In the order of [`plan`]'s result.
const FAMILIES: [Family; 5] = [Family::Paragraph, Family::Character, Family::Object, Family::Table, Family::Cell];

impl Family {
    /// Its key in the parameters and the report.
    fn key(self) -> &'static str {
        match self {
            Family::Paragraph => "paragraph",
            Family::Character => "character",
            Family::Object => "object",
            Family::Table => "table",
            Family::Cell => "cell",
        }
    }

    /// The built-in that stands for no style. It never loads.
    fn none(self) -> &'static str {
        match self {
            Family::Paragraph => NO_PARA_STYLE,
            Family::Character => NO_CHAR_STYLE,
            Family::Object => NO_OBJECT_STYLE,
            Family::Table => NO_TABLE_STYLE,
            Family::Cell => NO_CELL_STYLE,
        }
    }

    /// The names of its styles in `st`, in order.
    fn names(self, st: &Styles) -> Vec<&str> {
        match self {
            Family::Paragraph => st.paragraph.iter().map(|s| s.name.as_str()).collect(),
            Family::Character => st.character.iter().map(|s| s.name.as_str()).collect(),
            Family::Object => st.object.iter().map(|s| s.name.as_str()).collect(),
            Family::Table => st.table.iter().map(|s| s.name.as_str()).collect(),
            Family::Cell => st.cell.iter().map(|s| s.name.as_str()).collect(),
        }
    }

    /// Its loaded styles among [`plan`]'s.
    fn of(self, kinds: &mut [Kind; 5]) -> &mut Kind {
        let [paragraph, character, object, table, cell] = kinds;
        match self {
            Family::Paragraph => paragraph,
            Family::Character => character,
            Family::Object => object,
            Family::Table => table,
            Family::Cell => cell,
        }
    }
}

/// What happens to a loaded style when the document defines its name differently.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Conflict {
    /// The loaded definition replaces the document's.
    Redefine,
    /// The document's style stays.
    UseExisting,
    /// The loaded style comes in under a free name (`Name 2`).
    AutoRename,
}

/// What loading does with one style of the file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    /// The document keeps what it has.
    Skip,
    Add,
    Replace,
    /// Added under a new name.
    Rename,
}

/// The loaded styles of one family, by their name in the file.
#[derive(Default)]
struct Kind {
    actions: HashMap<String, Action>,
    renames: HashMap<String, String>,
    added: Vec<String>,
    replaced: Vec<String>,
}

impl Kind {
    /// The name a style of the file has in this document.
    fn follow(&self, name: &mut String) {
        if let Some(new) = self.renames.get(name.as_str()) {
            *name = new.clone();
        }
    }

    fn changes(&self) -> bool {
        !(self.added.is_empty() && self.replaced.is_empty() && self.renames.is_empty())
    }

    fn report(&self) -> Value {
        let renamed: BTreeMap<&String, &String> = self.renames.iter().collect();
        json!({"added": self.added, "replaced": self.replaced, "renamed": renamed})
    }
}

/// The document a command reads styles from.
fn source(p: &Value, id: &str) -> Result<Document> {
    let (bytes, name) = match (str_param(p, "path"), str_param(p, "base64")) {
        (_, Some(b)) => (super::file::base64_decode(b), str_param(p, "name").unwrap_or("").to_string()),
        (Some(path), None) => (crate::io::read_file(path).map_err(|e| bad(id, e))?, path.to_string()),
        _ => return Err(bad(id, "give `path` or `base64`")),
    };
    if name.to_lowercase().ends_with(".idml") {
        designcraft_idml::import_idml(&bytes).map_err(|e| bad(id, e.to_string()))
    } else {
        super::file::from_bytes(&bytes).map_err(|e| bad(id, e.to_string()))
    }
}

/// The styles of `f` to load: every one (`None`) or the named ones. Text styles load unless a
/// request lists some; object, table and cell styles only when it names them (`true`: all).
fn names(p: &Value, f: Family) -> Result<Option<Vec<String>>> {
    let key = f.key();
    let list = match p.get(key) {
        None | Some(Value::Null) => return Ok(matches!(f, Family::Object | Family::Table | Family::Cell).then(Vec::new)),
        Some(Value::Bool(all)) => return Ok((!all).then(Vec::new)),
        Some(v) => v.as_array().and_then(|a| a.iter().map(|n| n.as_str().map(str::to_string)).collect::<Option<Vec<_>>>()),
    };
    list.map(Some).ok_or_else(|| bad(ID, format!("`{key}` must be a list of style names, or true for all")))
}

/// The styles a paragraph style names: its base, its next style and its nested, line and GREP
/// styles.
fn para_refs(s: &ParagraphStyle) -> Vec<(Family, String)> {
    let p = &s.para;
    let chars = p
        .nested_styles
        .iter()
        .flatten()
        .map(|n| &n.style)
        .chain(p.grep_styles.iter().flatten().map(|g| &g.style))
        .chain(p.nested_line_styles.iter().flatten().map(|l| &l.style));
    s.based_on.iter().chain(&s.next_style).map(|n| (Family::Paragraph, n.clone())).chain(chars.map(|n| (Family::Character, n.clone()))).collect()
}

fn char_swatches(c: &CharAttrs) -> impl Iterator<Item = &String> {
    [&c.fill, &c.stroke, &c.underline_color, &c.strikethrough_color].into_iter().flatten()
}

fn para_swatches(p: &ParaAttrs) -> impl Iterator<Item = &String> {
    p.rule_above.iter().chain(&p.rule_below).map(|r| &r.color).chain(&p.shading_color).chain(&p.border_color)
}

/// The swatches and custom stroke styles the loaded styles name.
#[derive(Default)]
struct Uses {
    swatches: HashSet<String>,
    strokes: HashSet<String>,
}

impl Uses {
    fn kind(&mut self, k: &StrokeType) {
        if let StrokeType::Style { name } = k {
            self.strokes.insert(name.clone());
        }
    }

    fn cell_stroke(&mut self, s: &CellStroke) {
        self.swatches.extend([s.color.clone(), s.gap_color.clone()]);
        self.kind(&s.kind);
    }

    fn stroke_attrs(&mut self, s: &CellStrokeAttrs) {
        self.swatches.extend(s.color.iter().chain(&s.gap_color).cloned());
        if let Some(k) = &s.kind {
            self.kind(k);
        }
    }

    /// Fill, stroke and its gap, the column rule and the colours of the effects.
    fn object(&mut self, o: &ObjectStyle) {
        self.swatches.extend(o.fill.iter().map(|f| f.swatch.clone()));
        if let Some(s) = &o.stroke {
            self.swatches.extend([s.swatch.clone(), s.gap_swatch.clone()]);
            self.kind(&s.kind);
        }
        self.swatches.extend(o.text_frame.iter().map(|t| t.column_rule_color.clone()));
        if let Some(e) = &o.effects {
            let colors = [
                &e.drop_shadow.color,
                &e.inner_shadow.color,
                &e.outer_glow.color,
                &e.inner_glow.color,
                &e.satin.color,
                &e.bevel.highlight,
                &e.bevel.shadow,
            ];
            self.swatches.extend(colors.map(String::clone));
        }
    }

    fn cell(&mut self, c: &CellStyle) {
        self.swatches.extend(c.fill.clone());
        if let Some(s) = &c.stroke {
            self.cell_stroke(s);
        }
        for s in &c.strokes {
            self.stroke_attrs(s);
        }
    }

    fn table(&mut self, t: &TableStyle) {
        if let Some(b) = &t.border {
            self.cell_stroke(b);
        }
        for b in &t.borders {
            self.stroke_attrs(b);
        }
        if let Some(f) = &t.alt_rows {
            self.swatches.extend([f.first_color.clone(), f.next_color.clone()]);
        }
        for f in [&t.row_fills, &t.column_fills] {
            self.swatches.extend(f.first_color.iter().chain(&f.next_color).cloned());
        }
        for s in [&t.row_strokes, &t.column_strokes] {
            self.stroke_attrs(&s.first_stroke);
            self.stroke_attrs(&s.next_stroke);
        }
    }
}

/// The file's style `name` of `f`: whether the document's style of that name is the same
/// (`None`: it has none) and the styles it names. `None` when the file has no such style to load.
fn look(theirs: &Styles, mine: &Styles, f: Family, name: &str) -> Option<(Option<bool>, Vec<(Family, String)>)> {
    if name == f.none() {
        return None;
    }
    Some(match f {
        Family::Paragraph => {
            let s = theirs.para(name)?;
            (mine.para(name).map(|m| m == s), para_refs(s))
        }
        Family::Character => {
            let s = theirs.char_style(name)?;
            (mine.char_style(name).map(|m| m == s), s.based_on.iter().map(|b| (Family::Character, b.clone())).collect())
        }
        Family::Object => {
            let s = theirs.object_style(name)?;
            let refs = s.based_on.iter().map(|b| (Family::Object, b.clone())).chain(s.paragraph_style.iter().map(|p| (Family::Paragraph, p.clone())));
            (mine.object_style(name).map(|m| m == s), refs.collect())
        }
        Family::Table => {
            let s = theirs.table.iter().find(|s| s.name == name)?;
            let regions = [&s.header, &s.body, &s.footer, &s.left_column, &s.right_column].into_iter().flatten();
            let refs = s.based_on.iter().map(|b| (Family::Table, b.clone())).chain(regions.map(|c| (Family::Cell, c.clone())));
            (mine.table.iter().find(|m| m.name == name).map(|m| m == s), refs.collect())
        }
        Family::Cell => {
            let s = theirs.cell.iter().find(|s| s.name == name)?;
            let refs = s.based_on.iter().map(|b| (Family::Cell, b.clone())).chain(s.paragraph_style.iter().map(|p| (Family::Paragraph, p.clone())));
            (mine.cell.iter().find(|m| m.name == name).map(|m| m == s), refs.collect())
        }
    })
}

/// Decide what happens to every asked-for style and to the styles those name. A style the
/// document defines the same way is left alone; one that is only named by another is added when
/// the document has none, and never replaces one. `wanted` and the result follow [`FAMILIES`].
fn plan(doc: &Document, src: &Document, wanted: &[Option<Vec<String>>], conflict: Conflict) -> Result<[Kind; 5]> {
    let (theirs, mine) = (&src.styles, &doc.styles);
    let mut todo: Vec<(Family, String)> = Vec::new();
    for (f, wanted) in FAMILIES.into_iter().zip(wanted) {
        match wanted {
            Some(names) => {
                for n in names {
                    if look(theirs, mine, f, n).is_none() {
                        return Err(bad(ID, format!("the file has no {} style `{n}`", f.key())));
                    }
                    todo.push((f, n.clone()));
                }
            }
            None => todo.extend(f.names(theirs).into_iter().filter(|n| *n != f.none()).map(|n| (f, n.to_string()))),
        }
    }
    let asked: HashSet<(Family, String)> = todo.iter().cloned().collect();
    let mut kinds: [Kind; 5] = Default::default();
    while let Some(key) = todo.pop() {
        let kind = key.0.of(&mut kinds);
        if kind.actions.contains_key(&key.1) {
            continue;
        }
        // Is the document's style of that name the same, and which styles does the file's name?
        let Some((same, refs)) = look(theirs, mine, key.0, &key.1) else { continue };
        let action = match same {
            None => Action::Add,
            Some(false) if asked.contains(&key) => match conflict {
                Conflict::Redefine => Action::Replace,
                Conflict::UseExisting => Action::Skip,
                Conflict::AutoRename => Action::Rename,
            },
            Some(_) => Action::Skip,
        };
        if action != Action::Skip {
            todo.extend(refs);
        }
        kind.actions.insert(key.1, action);
    }
    // Free names for the renamed styles, in the file's order.
    for f in FAMILIES {
        let kind = f.of(&mut kinds);
        for name in f.names(theirs) {
            if kind.actions.get(name) != Some(&Action::Rename) || kind.renames.contains_key(name) {
                continue;
            }
            // A built-in style comes in as an ordinary one, without its brackets.
            let base = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')).filter(|n| !n.is_empty()).unwrap_or(name);
            let taken = |n: &str| {
                f.names(mine).contains(&n) || kind.actions.get(n).is_some_and(|a| *a != Action::Skip) || kind.renames.values().any(|r| r == n)
            };
            let new = Styles::unique_name(taken, base);
            kind.renames.insert(name.to_string(), new);
        }
    }
    Ok(kinds)
}

/// Puts a loaded style (named `theirs` in the file) into `list`: in place of the document's
/// style of its name when it replaces that, otherwise after the others.
fn put<T>(list: &mut Vec<T>, style: T, name: impl Fn(&T) -> &str, action: Action, kind: &mut Kind, theirs: &str) {
    match list.iter_mut().find(|m| name(m) == name(&style)) {
        Some(m) if action == Action::Replace => {
            *m = style;
            kind.replaced.push(theirs.to_string());
        }
        _ => {
            if action == Action::Add {
                kind.added.push(theirs.to_string());
            }
            list.push(style);
        }
    }
}

/// Does following `base_of` from `name` come back to it? A loop that doesn't reach it ends the
/// walk.
fn cycles<'a>(name: &str, base_of: impl Fn(&str) -> Option<&'a str>) -> bool {
    let mut seen = HashSet::new();
    let mut cur = base_of(name);
    while let Some(b) = cur {
        if b == name {
            return true;
        }
        if !seen.insert(b) {
            return false;
        }
        cur = base_of(b);
    }
    false
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let src = source(p, ID)?;
    let conflict = match p.get("conflicts") {
        None | Some(Value::Null) => Conflict::Redefine,
        Some(v) => match v.as_str() {
            Some("redefine") => Conflict::Redefine,
            Some("useExisting") => Conflict::UseExisting,
            Some("autoRename") => Conflict::AutoRename,
            _ => return Err(bad(ID, "`conflicts` must be redefine, useExisting or autoRename")),
        },
    };
    let mut wanted = Vec::new();
    for f in FAMILIES {
        wanted.push(names(p, f)?);
    }
    let doc = s.doc()?.doc.clone();
    let [mut para, mut chars, mut objects, mut tables, mut cells] = plan(&doc, &src, &wanted, conflict)?;

    let mut styles = (*doc.styles).clone();
    let (mut uses, mut lists): (Uses, HashSet<String>) = Default::default();
    // (family, name here) of every style that came in, for the based-on check.
    let mut loaded: Vec<(Family, String)> = Vec::new();
    for theirs in &src.styles.paragraph {
        let Some(action) = para.actions.remove(&theirs.name).filter(|a| *a != Action::Skip) else { continue };
        let mut style = theirs.clone();
        para.follow(&mut style.name);
        style.based_on.iter_mut().chain(&mut style.next_style).for_each(|n| para.follow(n));
        let p = &mut style.para;
        p.nested_styles.iter_mut().flatten().for_each(|n| chars.follow(&mut n.style));
        p.grep_styles.iter_mut().flatten().for_each(|g| chars.follow(&mut g.style));
        p.nested_line_styles.iter_mut().flatten().for_each(|l| chars.follow(&mut l.style));
        uses.swatches.extend(char_swatches(&style.chars).chain(para_swatches(&style.para)).cloned());
        lists.extend(style.para.list_name.iter().cloned());
        let tag = format!("p:{}", style.name);
        match src.styles.export_tags.get(&format!("p:{}", theirs.name)) {
            Some(t) => styles.export_tags.insert(tag, t.clone()),
            None => styles.export_tags.remove(&tag),
        };
        loaded.push((Family::Paragraph, style.name.clone()));
        put(&mut styles.paragraph, style, |s| s.name.as_str(), action, &mut para, &theirs.name);
    }
    for theirs in &src.styles.character {
        let Some(action) = chars.actions.remove(&theirs.name).filter(|a| *a != Action::Skip) else { continue };
        let mut style = theirs.clone();
        chars.follow(&mut style.name);
        style.based_on.iter_mut().for_each(|n| chars.follow(n));
        uses.swatches.extend(char_swatches(&style.chars).cloned());
        let tag = format!("c:{}", style.name);
        match src.styles.export_tags.get(&format!("c:{}", theirs.name)) {
            Some(t) => styles.export_tags.insert(tag, t.clone()),
            None => styles.export_tags.remove(&tag),
        };
        loaded.push((Family::Character, style.name.clone()));
        put(&mut styles.character, style, |s| s.name.as_str(), action, &mut chars, &theirs.name);
    }
    for theirs in &src.styles.object {
        let Some(action) = objects.actions.remove(&theirs.name).filter(|a| *a != Action::Skip) else { continue };
        let mut style = theirs.clone();
        objects.follow(&mut style.name);
        style.based_on.iter_mut().for_each(|n| objects.follow(n));
        style.paragraph_style.iter_mut().for_each(|n| para.follow(n));
        uses.object(&style);
        loaded.push((Family::Object, style.name.clone()));
        put(&mut styles.object, style, |s| s.name.as_str(), action, &mut objects, &theirs.name);
    }
    for theirs in &src.styles.table {
        let Some(action) = tables.actions.remove(&theirs.name).filter(|a| *a != Action::Skip) else { continue };
        let mut style = theirs.clone();
        tables.follow(&mut style.name);
        style.based_on.iter_mut().for_each(|n| tables.follow(n));
        let regions = [&mut style.header, &mut style.body, &mut style.footer, &mut style.left_column, &mut style.right_column];
        regions.into_iter().flatten().for_each(|n| cells.follow(n));
        uses.table(&style);
        loaded.push((Family::Table, style.name.clone()));
        put(&mut styles.table, style, |s| s.name.as_str(), action, &mut tables, &theirs.name);
    }
    for theirs in &src.styles.cell {
        let Some(action) = cells.actions.remove(&theirs.name).filter(|a| *a != Action::Skip) else { continue };
        let mut style = theirs.clone();
        cells.follow(&mut style.name);
        style.based_on.iter_mut().for_each(|n| cells.follow(n));
        style.paragraph_style.iter_mut().for_each(|n| para.follow(n));
        uses.cell(&style);
        loaded.push((Family::Cell, style.name.clone()));
        put(&mut styles.cell, style, |s| s.name.as_str(), action, &mut cells, &theirs.name);
    }
    // A loaded style and one the document keeps can name each other as their base.
    for (f, name) in &loaded {
        let looped = match f {
            Family::Paragraph => styles.para(name).and_then(|s| s.based_on.as_deref()).is_some_and(|b| styles.para_based_on_cycles(name, b)),
            Family::Character => styles.char_style(name).and_then(|s| s.based_on.as_deref()).is_some_and(|b| styles.char_based_on_cycles(name, b)),
            Family::Object => cycles(name, |n| styles.object_style(n).and_then(|s| s.based_on.as_deref())),
            Family::Table => cycles(name, |n| styles.table.iter().find(|s| s.name == n).and_then(|s| s.based_on.as_deref())),
            Family::Cell => cycles(name, |n| styles.cell.iter().find(|s| s.name == n).and_then(|s| s.based_on.as_deref())),
        };
        if looped {
            return Err(bad(ID, format!("{} style `{name}` would be based on itself; load the style it is based on as well", f.key())));
        }
    }

    let swatches = missing_swatches(&doc, &src, uses.swatches);
    let lists: Vec<NumberedList> = {
        let mut seen = HashSet::new();
        let has = |n: &str| doc.settings.lists.iter().any(|l| l.name == n);
        src.settings.lists.iter().filter(|l| lists.contains(&l.name) && !has(&l.name) && seen.insert(l.name.as_str())).cloned().collect()
    };
    // Custom stroke styles the document lacks; one of the same name stays the document's.
    let strokes: Vec<StrokeStyleDef> = {
        let mut seen = HashSet::new();
        let has = |n: &str| doc.stroke_styles.iter().any(|d| d.name == n);
        src.stroke_styles.iter().filter(|d| uses.strokes.contains(&d.name) && !has(&d.name) && seen.insert(d.name.as_str())).cloned().collect()
    };
    let report = json!({
        "paragraph": para.report(),
        "character": chars.report(),
        "object": objects.report(),
        "table": tables.report(),
        "cell": cells.report(),
        "swatches": swatches.iter().map(|w| &w.name).collect::<Vec<_>>(),
        "strokeStyles": strokes.iter().map(|d| &d.name).collect::<Vec<_>>(),
    });
    // Nothing to load leaves no undo step.
    if [&para, &chars, &objects, &tables, &cells].iter().any(|k| k.changes()) {
        s.edit(|d, _| {
            *d.styles_mut() = styles;
            d.swatches.extend(swatches);
            d.settings.lists.extend(lists);
            d.stroke_styles.extend(strokes);
            Ok(())
        })?;
    }
    Ok(report)
}

/// The swatches of the file that the loaded styles name and the document lacks, in the file's
/// order; a tint brings its base colour.
fn missing_swatches(doc: &Document, src: &Document, mut used: HashSet<String>) -> Vec<Swatch> {
    let mut todo: Vec<String> = used.iter().cloned().collect();
    while let Some(name) = todo.pop() {
        if let Some(Swatch { value: SwatchValue::Tint { base, .. }, .. }) = src.swatch(&name)
            && used.insert(base.clone())
        {
            todo.push(base.clone());
        }
    }
    let mut seen = HashSet::new();
    src.swatches.iter().filter(|w| used.contains(&w.name) && doc.swatch(&w.name).is_none() && seen.insert(w.name.as_str())).cloned().collect()
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let src = source(p, "style.load.list")?;
    let mine = &s.doc()?.doc.styles;
    let mut out = serde_json::Map::new();
    for f in FAMILIES {
        let rows: Vec<Value> = f
            .names(&src.styles)
            .into_iter()
            .filter_map(|n| look(&src.styles, mine, f, n).map(|(same, _)| json!({"name": n, "conflict": same == Some(false)})))
            .collect();
        out.insert(f.key().into(), rows.into());
    }
    Ok(out.into())
}

#[cfg(test)]
mod tests {
    use designcraft_color::{Color, Swatch, SwatchValue};
    use designcraft_doc::{
        AltFillsAttrs, CellStroke, CellStyle, CharAttrs, CharacterStyle, DropShadow, Effects, ExportTag, Fill, GrepStyle, NO_TABLE_STYLE,
        NumberedList, ObjectStyle, ParaAttrs, ParagraphStyle, Rule, Stroke, StrokeStyleDef, StrokeType, Styles, TableStyle,
    };
    use serde_json::{Value, json};

    use crate::Session;

    const BASIC: &str = "[Basic Paragraph]";

    fn para(name: &str, based_on: Option<&str>, size: f64) -> ParagraphStyle {
        ParagraphStyle {
            name: name.into(),
            based_on: based_on.map(str::to_string),
            next_style: None,
            para: ParaAttrs::default(),
            chars: CharAttrs { size: Some(size), ..Default::default() },
            shortcut: String::new(),
        }
    }

    fn chars(name: &str, based_on: Option<&str>, size: f64) -> CharacterStyle {
        CharacterStyle {
            name: name.into(),
            based_on: based_on.map(str::to_string),
            chars: CharAttrs { size: Some(size), ..Default::default() },
            shortcut: String::new(),
        }
    }

    fn new_doc() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s
    }

    fn file_of(s: &Session) -> String {
        super::super::file::base64_encode(&super::super::file::to_bytes(&s.doc().unwrap().doc).unwrap())
    }

    /// A client's document: Body in the brand colour with a GREP style, Head and Quote based on
    /// it, Small characters based on Lead, an 11 pt [Basic Paragraph], and a swatch no style uses.
    fn client() -> Session {
        let mut s = new_doc();
        s.edit(|d, _| {
            d.swatches.push(Swatch::color("Brand Teal", Color::cmyk(0.9, 0.0, 0.4, 0.2)));
            d.swatches.push(Swatch {
                name: "Brand Teal 40%".into(),
                value: SwatchValue::Tint { base: "Brand Teal".into(), tint: 0.4 },
                locked: false,
                named: true,
                hidden: false,
            });
            d.swatches.push(Swatch::color("Unused Red", Color::cmyk(0.0, 1.0, 1.0, 0.0)));
            let st = d.styles_mut();
            st.para_mut(BASIC).unwrap().chars.size = Some(11.0);
            st.character.push(chars("Lead", None, 9.0));
            st.character.push(chars("Small", Some("Lead"), 7.0));
            let mut body = para("Body", Some(BASIC), 10.0);
            body.chars.fill = Some("Brand Teal".into());
            body.para.grep_styles = Some(vec![GrepStyle { style: "Small".into(), pattern: r"\d+".into() }]);
            st.paragraph.push(body);
            let mut head = para("Head", Some("Body"), 20.0);
            head.next_style = Some("Body".into());
            st.paragraph.push(head);
            let mut quote = para("Quote", Some("Body"), 12.0);
            quote.para.rule_above = Some(Rule { on: true, color: "Brand Teal 40%".into(), ..Default::default() });
            st.paragraph.push(quote);
            Ok(())
        })
        .unwrap();
        s
    }

    fn styles(s: &Session) -> Styles {
        (*s.doc().unwrap().doc.styles).clone()
    }

    #[test]
    fn every_style_of_the_file_loads_with_the_swatches_it_uses() {
        let mut s = new_doc();
        let r = s.execute("style.load", &json!({"base64": file_of(&client())})).unwrap();
        let d = &s.doc().unwrap().doc;
        let head = d.styles.para("Head").unwrap();
        assert_eq!((head.based_on.as_deref(), head.next_style.as_deref(), head.chars.size), (Some("Body"), Some("Body"), Some(20.0)));
        assert_eq!(d.styles.para("Body").unwrap().chars.fill.as_deref(), Some("Brand Teal"));
        assert_eq!(d.styles.char_style("Small").unwrap().based_on.as_deref(), Some("Lead"));
        assert_eq!(d.swatch("Brand Teal"), client().doc().unwrap().doc.swatch("Brand Teal"));
        assert!(d.swatch("Brand Teal 40%").is_some());
        assert!(d.swatch("Unused Red").is_none(), "a swatch no loaded style uses stays behind");
        assert_eq!(r["paragraph"]["added"], json!(["Body", "Head", "Quote"]));
        assert_eq!(r["character"]["added"], json!(["Lead", "Small"]));
        assert_eq!(r["swatches"], json!(["Brand Teal", "Brand Teal 40%"]));
    }

    #[test]
    fn basic_paragraph_takes_the_definition_of_the_file() {
        let mut s = new_doc();
        let r = s.execute("style.load", &json!({"base64": file_of(&client())})).unwrap();
        assert_eq!(s.doc().unwrap().doc.styles.para(BASIC).unwrap().chars.size, Some(11.0));
        assert_eq!(r["paragraph"]["replaced"], json!([BASIC]));
    }

    #[test]
    fn a_named_style_brings_the_styles_it_names() {
        let mut s = new_doc();
        let r = s.execute("style.load", &json!({"base64": file_of(&client()), "paragraph": ["Head"], "character": []})).unwrap();
        let st = styles(&s);
        assert_eq!(r["paragraph"]["added"], json!(["Body", "Head"]), "Head is based on Body");
        assert_eq!(r["character"]["added"], json!(["Lead", "Small"]), "Body's GREP style names Small, which is based on Lead");
        assert!(st.para("Quote").is_none());
        assert_eq!(st.para(BASIC).unwrap().chars.size, None, "a style the document has is only replaced when it was asked for");
        assert_eq!(r["paragraph"]["replaced"], json!([]));
        assert_eq!(r["swatches"], json!(["Brand Teal"]));
    }

    /// The document already has a Body and a Lead of its own.
    fn doc_with_body_and_lead() -> Session {
        let mut s = new_doc();
        s.edit(|d, _| {
            let st = d.styles_mut();
            st.paragraph.push(para("Body", None, 8.0));
            st.character.push(chars("Lead", None, 5.0));
            Ok(())
        })
        .unwrap();
        s
    }

    #[test]
    fn a_style_of_the_same_name_takes_the_loaded_definition() {
        let mut s = doc_with_body_and_lead();
        let r = s.execute("style.load", &json!({"base64": file_of(&client())})).unwrap();
        let st = styles(&s);
        assert_eq!((st.para("Body").unwrap().chars.size, st.char_style("Lead").unwrap().chars.size), (Some(10.0), Some(9.0)));
        assert_eq!(r["paragraph"]["replaced"], json!([BASIC, "Body"]));
        assert_eq!(r["character"]["replaced"], json!(["Lead"]));
        assert_eq!(st.paragraph.iter().filter(|p| p.name == "Body").count(), 1);
    }

    #[test]
    fn use_existing_keeps_the_styles_of_the_document() {
        let mut s = doc_with_body_and_lead();
        let r = s.execute("style.load", &json!({"base64": file_of(&client()), "conflicts": "useExisting"})).unwrap();
        let st = styles(&s);
        assert_eq!((st.para("Body").unwrap().chars.size, st.char_style("Lead").unwrap().chars.size), (Some(8.0), Some(5.0)));
        assert_eq!(st.para(BASIC).unwrap().chars.size, None);
        assert_eq!(st.para("Head").unwrap().based_on.as_deref(), Some("Body"));
        assert_eq!((&r["paragraph"]["added"], &r["paragraph"]["replaced"]), (&json!(["Head", "Quote"]), &json!([])));
    }

    #[test]
    fn auto_rename_loads_a_clashing_style_under_a_new_name() {
        let mut s = doc_with_body_and_lead();
        let r = s.execute("style.load", &json!({"base64": file_of(&client()), "conflicts": "autoRename"})).unwrap();
        let st = styles(&s);
        assert_eq!((st.para("Body").unwrap().chars.size, st.para("Body 2").unwrap().chars.size), (Some(8.0), Some(10.0)));
        assert_eq!((st.char_style("Lead").unwrap().chars.size, st.char_style("Lead 2").unwrap().chars.size), (Some(5.0), Some(9.0)));
        let head = st.para("Head").unwrap();
        assert_eq!(
            (head.based_on.as_deref(), head.next_style.as_deref()),
            (Some("Body 2"), Some("Body 2")),
            "the loaded Head follows the loaded Body"
        );
        assert_eq!(st.char_style("Small").unwrap().based_on.as_deref(), Some("Lead 2"));
        assert_eq!(st.para("Body 2").unwrap().based_on.as_deref(), Some("Basic Paragraph"));
        assert_eq!(st.para("Basic Paragraph").unwrap().chars.size, Some(11.0), "a built-in style comes in without its brackets");
        assert_eq!(st.para(BASIC).unwrap().chars.size, None);
        assert_eq!(r["paragraph"]["renamed"], json!({"[Basic Paragraph]": "Basic Paragraph", "Body": "Body 2"}));
        assert_eq!(r["character"]["renamed"], json!({"Lead": "Lead 2"}));
        assert_eq!(r["paragraph"]["replaced"], json!([]));
    }

    #[test]
    fn a_style_that_is_the_same_in_both_is_no_conflict() {
        let mut s = new_doc();
        let file = file_of(&client());
        s.execute("style.load", &json!({"base64": file})).unwrap();
        let loaded = styles(&s);
        for conflicts in ["redefine", "autoRename"] {
            let r = s.execute("style.load", &json!({"base64": file, "conflicts": conflicts})).unwrap();
            let none = json!({"added": [], "replaced": [], "renamed": {}});
            let same = json!({"paragraph": none, "character": none, "object": none, "table": none, "cell": none, "swatches": [], "strokeStyles": []});
            assert_eq!(r, same, "{conflicts}");
        }
        assert_eq!(styles(&s), loaded);
        // Loading nothing leaves no undo step: one Undo takes back the first load.
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(styles(&s).para("Head").is_none());
    }

    #[test]
    fn loading_is_one_undo_step() {
        let mut s = doc_with_body_and_lead();
        let (before, swatches) = (styles(&s), s.doc().unwrap().doc.swatches.clone());
        s.execute("style.load", &json!({"base64": file_of(&client())})).unwrap();
        assert_ne!(styles(&s), before);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(styles(&s), before);
        assert_eq!(s.doc().unwrap().doc.swatches, swatches);
    }

    #[test]
    fn a_swatch_of_the_document_wins_over_the_one_in_the_file() {
        let mut s = new_doc();
        s.execute("swatch.create", &json!({"name": "Brand Teal", "color": "#ff0000"})).unwrap();
        let mine = s.doc().unwrap().doc.swatch("Brand Teal").cloned();
        let r = s.execute("style.load", &json!({"base64": file_of(&client())})).unwrap();
        assert_eq!(s.doc().unwrap().doc.swatch("Brand Teal").cloned(), mine);
        assert_eq!(r["swatches"], json!(["Brand Teal 40%"]));
    }

    #[test]
    fn a_tint_swatch_brings_its_base_colour() {
        let mut src = client();
        src.edit(|d, _| {
            let mut ruled = para("Ruled", None, 9.0);
            ruled.para.rule_below = Some(Rule { on: true, color: "Brand Teal 40%".into(), ..Default::default() });
            d.styles_mut().paragraph.push(ruled);
            Ok(())
        })
        .unwrap();
        let mut s = new_doc();
        let r = s.execute("style.load", &json!({"base64": file_of(&src), "paragraph": ["Ruled"], "character": []})).unwrap();
        assert_eq!(r["paragraph"]["added"], json!(["Ruled"]));
        assert_eq!(r["swatches"], json!(["Brand Teal", "Brand Teal 40%"]));
    }

    #[test]
    fn export_tags_and_named_lists_come_with_their_styles() {
        let mut src = client();
        src.edit(|d, _| {
            d.settings.lists.push(NumberedList { name: "Steps".into(), continue_across_stories: false });
            d.settings.lists.push(NumberedList { name: "Unused".into(), ..Default::default() });
            let st = d.styles_mut();
            st.export_tags.insert("p:Head".into(), ExportTag { tag: "h1".into(), class: "title".into() });
            st.export_tags.insert("c:Lead".into(), ExportTag { tag: "em".into(), class: String::new() });
            st.para_mut("Quote").unwrap().para.list_name = Some("Steps".into());
            Ok(())
        })
        .unwrap();
        let mut s = doc_with_body_and_lead();
        s.execute("style.load", &json!({"base64": file_of(&src), "conflicts": "autoRename"})).unwrap();
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.styles.export_tag("Head", false).map(|t| t.tag.as_str()), Some("h1"));
        assert_eq!(d.styles.export_tag("Lead 2", true).map(|t| t.tag.as_str()), Some("em"), "the tag follows the new name");
        assert_eq!(d.styles.export_tag("Lead", true), None);
        assert_eq!(d.settings.lists.iter().map(|l| (l.name.as_str(), l.continue_across_stories)).collect::<Vec<_>>(), [("Steps", false)]);
    }

    #[test]
    fn styles_load_from_an_idml_file() {
        let idml = client().execute("file.exportIdml", &json!({})).unwrap()["base64"].as_str().unwrap().to_string();
        let mut s = new_doc();
        let r = s.execute("style.load", &json!({"base64": idml, "name": "Client.IDML", "paragraph": ["Head"]})).unwrap();
        let d = &s.doc().unwrap().doc;
        let head = d.styles.para("Head").unwrap();
        assert_eq!((head.based_on.as_deref(), head.chars.size), (Some("Body"), Some(20.0)));
        assert_eq!(d.styles.para("Body").unwrap().chars.fill.as_deref(), Some("Brand Teal"));
        assert!(d.swatch("Brand Teal").is_some());
        assert_eq!(r["paragraph"]["added"], json!(["Body", "Head"]));
    }

    #[test]
    fn styles_load_from_a_path() {
        let path = std::env::temp_dir().join(format!("dc-load-styles-{}.designcraft", std::process::id()));
        std::fs::write(&path, super::super::file::to_bytes(&client().doc().unwrap().doc).unwrap()).unwrap();
        let mut s = new_doc();
        let r = s.execute("style.load", &json!({"path": path.to_str().unwrap(), "paragraph": [], "character": ["Small"]}));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(r.unwrap()["character"]["added"], json!(["Lead", "Small"]));
        assert!(styles(&s).para("Head").is_none());
    }

    #[test]
    fn the_file_lists_its_styles_and_which_ones_clash() {
        let mut s = doc_with_body_and_lead();
        let file = file_of(&client());
        let name = |n: &str, conflict: bool| json!({"name": n, "conflict": conflict});
        let r = s.execute("style.load.list", &json!({"base64": file})).unwrap();
        assert_eq!(r["paragraph"], json!([name(BASIC, true), name("Body", true), name("Head", false), name("Quote", false)]));
        assert_eq!(r["character"], json!([name("Lead", true), name("Small", false)]));
        s.execute("style.load", &json!({"base64": file})).unwrap();
        let r = s.execute("style.load.list", &json!({"base64": file})).unwrap();
        assert!(r["paragraph"].as_array().unwrap().iter().all(|p| p["conflict"] == false), "the same definition is no clash: {r}");
    }

    fn unchanged_after(s: &mut Session, params: Value, why: &str) {
        let before = s.doc().unwrap().doc.clone();
        let e = s.execute("style.load", &params).unwrap_err().to_string();
        assert!(e.contains(why), "{e}");
        assert_eq!(s.doc().unwrap().doc, before);
    }

    #[test]
    fn what_cannot_be_loaded_is_an_error_and_changes_nothing() {
        let mut s = doc_with_body_and_lead();
        let file = file_of(&client());
        unchanged_after(&mut s, json!({}), "`path` or `base64`");
        unchanged_after(&mut s, json!({"base64": "AAAA"}), "");
        unchanged_after(&mut s, json!({"base64": "AAAA", "name": "a.idml"}), "");
        unchanged_after(&mut s, json!({"base64": file, "paragraph": ["Head", "Nope"]}), "no paragraph style `Nope`");
        unchanged_after(&mut s, json!({"base64": file, "character": ["[None]"]}), "no character style `[None]`");
        unchanged_after(&mut s, json!({"base64": file, "paragraph": ["[No Paragraph Style]"]}), "no paragraph style");
        unchanged_after(&mut s, json!({"base64": file, "paragraph": "Head"}), "`paragraph` must be a list of style names");
        unchanged_after(&mut s, json!({"base64": file, "character": [1]}), "`character` must be a list of style names");
        unchanged_after(&mut s, json!({"base64": file, "conflicts": "merge"}), "`conflicts`");
    }

    #[test]
    fn a_style_that_would_be_based_on_itself_is_refused() {
        // In the file B is based on A; in the document A is based on B.
        let mut src = new_doc();
        src.edit(|d, _| {
            let st = d.styles_mut();
            st.paragraph.push(para("A", None, 9.0));
            st.paragraph.push(para("B", Some("A"), 9.0));
            st.character.push(chars("X", None, 9.0));
            st.character.push(chars("Y", Some("X"), 9.0));
            Ok(())
        })
        .unwrap();
        let mut s = new_doc();
        s.edit(|d, _| {
            let st = d.styles_mut();
            st.paragraph.push(para("B", None, 9.0));
            st.paragraph.push(para("A", Some("B"), 9.0));
            st.character.push(chars("Y", None, 9.0));
            st.character.push(chars("X", Some("Y"), 9.0));
            Ok(())
        })
        .unwrap();
        unchanged_after(&mut s, json!({"base64": file_of(&src), "paragraph": ["B"], "character": []}), "`B` would be based on itself");
        unchanged_after(&mut s, json!({"base64": file_of(&src), "paragraph": [], "character": ["Y"]}), "`Y` would be based on itself");
        // With A loaded as well both take the file's order.
        s.execute("style.load", &json!({"base64": file_of(&src), "character": []})).unwrap();
        assert_eq!((styles(&s).para("A").unwrap().based_on.clone(), styles(&s).para("B").unwrap().based_on.clone()), (None, Some("A".to_string())));
    }

    #[test]
    fn a_file_with_a_based_on_loop_of_its_own_does_not_hang() {
        let mut src = new_doc();
        src.edit(|d, _| {
            let st = d.styles_mut();
            st.paragraph.push(para("A", Some("B"), 9.0));
            st.paragraph.push(para("B", Some("A"), 9.0));
            st.paragraph.push(para("Next", None, 9.0));
            st.para_mut("Next").unwrap().next_style = Some("Next".into());
            Ok(())
        })
        .unwrap();
        let mut s = new_doc();
        unchanged_after(&mut s, json!({"base64": file_of(&src), "paragraph": ["A"]}), "would be based on itself");
        let r = s.execute("style.load", &json!({"base64": file_of(&src), "paragraph": ["Next"]})).unwrap();
        assert_eq!(r["paragraph"]["added"], json!(["Next"]));
    }

    /// The client's document with object, table and cell styles too. Data (on Base Table) gives
    /// its header Head Cell and its body Body Cell; Head Cell is based on Body Cell, and both name a
    /// paragraph style. Callout (on Box) names Quote and has a drop shadow. Body Cell's stroke and
    /// Callout's are the custom stroke style Dots.
    fn with_tables_and_objects() -> Session {
        let mut s = client();
        s.edit(|d, _| {
            for name in ["Shadow Blue", "Rule Red", "Cell Tint"] {
                d.swatches.push(Swatch::color(name, Color::cmyk(0.5, 0.2, 0.0, 0.1)));
            }
            d.stroke_styles.push(StrokeStyleDef { name: "Dots".into(), kind: StrokeType::Dotted });
            d.stroke_styles.push(StrokeStyleDef { name: "Unused Dashes".into(), kind: StrokeType::Dotted });
            let dots = || StrokeType::Style { name: "Dots".into() };
            let st = d.styles_mut();
            st.cell.push(CellStyle {
                name: "Body Cell".into(),
                fill: Some("Cell Tint".into()),
                stroke: Some(CellStroke { color: "Rule Red".into(), kind: dots(), ..Default::default() }),
                paragraph_style: Some("Body".into()),
                ..Default::default()
            });
            st.cell.push(CellStyle {
                name: "Head Cell".into(),
                based_on: Some("Body Cell".into()),
                fill: Some("Brand Teal".into()),
                paragraph_style: Some("Head".into()),
                ..Default::default()
            });
            st.table.push(TableStyle {
                name: "Base Table".into(),
                border: Some(CellStroke { color: "Rule Red".into(), ..Default::default() }),
                ..Default::default()
            });
            st.table.push(TableStyle {
                name: "Data".into(),
                based_on: Some("Base Table".into()),
                header: Some("Head Cell".into()),
                body: Some("Body Cell".into()),
                row_fills: AltFillsAttrs { first: Some(1), first_color: Some("Brand Teal 40%".into()), ..Default::default() },
                space_before: Some(6.0),
                ..Default::default()
            });
            st.object.push(ObjectStyle { name: "Box".into(), fill: Some(Fill { swatch: "Cell Tint".into(), ..Fill::none() }), ..Default::default() });
            st.object.push(ObjectStyle {
                name: "Callout".into(),
                based_on: Some("Box".into()),
                paragraph_style: Some("Quote".into()),
                stroke: Some(Stroke { swatch: "Rule Red".into(), kind: dots(), ..Default::default() }),
                effects: Some(Effects {
                    drop_shadow: DropShadow { on: true, color: "Shadow Blue".into(), ..Default::default() },
                    ..Default::default()
                }),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
        s
    }

    /// A document with its own Data, Body Cell and Box, unlike the file's.
    fn doc_with_own_data_body_cell_and_box() -> Session {
        let mut s = new_doc();
        s.edit(|d, _| {
            let st = d.styles_mut();
            st.table.push(TableStyle { name: "Data".into(), space_before: Some(3.0), ..Default::default() });
            st.cell.push(CellStyle { name: "Body Cell".into(), fill: Some("[Paper]".into()), ..Default::default() });
            st.object.push(ObjectStyle { name: "Box".into(), fill: Some(Fill { swatch: "[Black]".into(), ..Fill::none() }), ..Default::default() });
            Ok(())
        })
        .unwrap();
        s
    }

    fn table<'a>(st: &'a Styles, name: &str) -> &'a TableStyle {
        st.table.iter().find(|t| t.name == name).unwrap()
    }

    fn cell<'a>(st: &'a Styles, name: &str) -> &'a CellStyle {
        st.cell.iter().find(|c| c.name == name).unwrap()
    }

    fn object<'a>(st: &'a Styles, name: &str) -> &'a ObjectStyle {
        st.object.iter().find(|o| o.name == name).unwrap()
    }

    /// A script that asks for a few text styles, or names none at all, never touches the
    /// document's object, table and cell styles: those load only when a request names them.
    #[test]
    fn object_table_and_cell_styles_load_only_when_named() {
        let mut s = doc_with_own_data_body_cell_and_box();
        let file = file_of(&with_tables_and_objects());
        let before = styles(&s);
        let none = json!({"added": [], "replaced": [], "renamed": {}});
        for params in [json!({"base64": file, "paragraph": ["Body"]}), json!({"base64": file}), json!({"base64": file, "object": null})] {
            let r = s.execute("style.load", &params).unwrap();
            let after = styles(&s);
            assert_eq!((&after.object, &after.table, &after.cell), (&before.object, &before.table, &before.cell), "{params}");
            assert_eq!((&r["object"], &r["table"], &r["cell"]), (&none, &none, &none), "{params}");
            assert_eq!(r["strokeStyles"], json!([]), "{params}");
            assert!(s.doc().unwrap().doc.stroke_styles.is_empty());
        }
    }

    #[test]
    fn a_table_style_brings_its_base_its_cell_styles_and_what_they_name() {
        let mut s = new_doc();
        let src = with_tables_and_objects();
        let r = s.execute("style.load", &json!({"base64": file_of(&src), "table": ["Data"], "paragraph": [], "character": []})).unwrap();
        assert_eq!(r["table"]["added"], json!(["Base Table", "Data"]));
        assert_eq!(r["cell"]["added"], json!(["Body Cell", "Head Cell"]));
        assert_eq!(r["paragraph"]["added"], json!(["Body", "Head"]), "the cell styles' paragraph styles");
        assert_eq!(r["character"]["added"], json!(["Lead", "Small"]), "and the character styles Body names");
        assert_eq!(r["swatches"], json!(["Brand Teal", "Brand Teal 40%", "Rule Red", "Cell Tint"]));
        assert_eq!(r["strokeStyles"], json!(["Dots"]));
        let (st, theirs) = (styles(&s), styles(&src));
        assert_eq!(table(&st, "Data"), table(&theirs, "Data"));
        assert_eq!(st.resolve_table_style("Data"), theirs.resolve_table_style("Data"));
        assert_eq!(cell(&st, "Head Cell"), cell(&theirs, "Head Cell"));
        assert!(st.object.iter().all(|o| o.name != "Box"), "no object style was asked for");
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.stroke_styles, vec![StrokeStyleDef { name: "Dots".into(), kind: StrokeType::Dotted }]);
        assert!(d.swatch("Shadow Blue").is_none() && d.swatch("Unused Red").is_none());
    }

    #[test]
    fn an_object_style_brings_its_base_its_paragraph_style_and_the_colours_of_its_effects() {
        let mut s = new_doc();
        let src = with_tables_and_objects();
        let r = s.execute("style.load", &json!({"base64": file_of(&src), "object": ["Callout"], "paragraph": [], "character": []})).unwrap();
        assert_eq!(r["object"]["added"], json!(["Box", "Callout"]));
        assert_eq!(r["paragraph"]["added"], json!(["Body", "Quote"]), "Quote is based on Body");
        assert_eq!(r["swatches"], json!(["Brand Teal", "Brand Teal 40%", "Shadow Blue", "Rule Red", "Cell Tint"]));
        assert_eq!(r["strokeStyles"], json!(["Dots"]));
        let (st, theirs) = (styles(&s), styles(&src));
        assert_eq!(object(&st, "Callout"), object(&theirs, "Callout"));
        assert_eq!(object(&st, "Box"), object(&theirs, "Box"));
        assert!(st.table.iter().all(|t| t.name != "Data") && st.cell.iter().all(|c| c.name != "Body Cell"));
    }

    #[test]
    fn true_loads_every_style_of_a_kind_but_never_its_none() {
        let mut s = new_doc();
        let file = file_of(&with_tables_and_objects());
        let r = s
            .execute("style.load", &json!({"base64": file, "object": true, "table": true, "cell": true, "paragraph": false, "character": false}))
            .unwrap();
        assert_eq!(
            (&r["object"]["added"], &r["table"]["added"], &r["cell"]["added"]),
            (&json!(["Box", "Callout"]), &json!(["Base Table", "Data"]), &json!(["Body Cell", "Head Cell"]))
        );
        assert_eq!(r["object"]["replaced"], json!([]), "the file's built-ins are the document's");
        let list = s.execute("style.load.list", &json!({"base64": file})).unwrap();
        let rows = |k: &str| list[k].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(rows("object"), ["[Basic Graphics Frame]", "[Basic Text Frame]", "Box", "Callout"]);
        assert_eq!(rows("table"), ["[Basic Table]", "Base Table", "Data"]);
        assert_eq!(rows("cell"), ["Body Cell", "Head Cell"]);
        assert!(list["table"].as_array().unwrap().iter().all(|r| r["conflict"] == false), "loaded: the same, so no clash");
        unchanged_after(&mut s, json!({"base64": file, "object": ["[None]"]}), "no object style `[None]`");
        unchanged_after(&mut s, json!({"base64": file, "cell": ["[None]"]}), "no cell style `[None]`");
        unchanged_after(&mut s, json!({"base64": file, "table": ["Nope"]}), "no table style `Nope`");
        unchanged_after(&mut s, json!({"base64": file, "table": "Data"}), "`table` must be a list of style names");
        unchanged_after(&mut s, json!({"base64": file, "cell": [1]}), "`cell` must be a list of style names");
    }

    #[test]
    fn a_clashing_table_cell_or_object_style_follows_the_conflict_rule() {
        let file = file_of(&with_tables_and_objects());
        let asked = |conflicts: &str| json!({"base64": file, "table": ["Data"], "cell": ["Body Cell"], "object": ["Box"], "paragraph": [], "character": [], "conflicts": conflicts});
        let mut s = doc_with_own_data_body_cell_and_box();
        let r = s.execute("style.load", &asked("redefine")).unwrap();
        assert_eq!(
            (&r["table"]["replaced"], &r["cell"]["replaced"], &r["object"]["replaced"]),
            (&json!(["Data"]), &json!(["Body Cell"]), &json!(["Box"]))
        );
        let (st, theirs) = (styles(&s), styles(&with_tables_and_objects()));
        assert_eq!(
            (table(&st, "Data"), cell(&st, "Body Cell"), object(&st, "Box")),
            (table(&theirs, "Data"), cell(&theirs, "Body Cell"), object(&theirs, "Box"))
        );

        // Use Existing: the document keeps its own, and what only they would have brought stays out.
        let mut s = doc_with_own_data_body_cell_and_box();
        let mine = styles(&s);
        let r = s.execute("style.load", &asked("useExisting")).unwrap();
        assert_eq!(styles(&s), mine);
        assert_eq!(r["cell"]["added"], json!([]));
    }

    /// Auto Rename gives a clashing style of every kind a new name, and whatever names it follows:
    /// a table style's base and region cell styles, a cell style's base and paragraph style, an
    /// object style's base and paragraph style.
    #[test]
    fn auto_rename_renames_clashing_styles_and_what_names_them() {
        let mut s = doc_with_own_data_body_cell_and_box();
        s.edit(|d, _| {
            let st = d.styles_mut();
            st.table.push(TableStyle { name: "Base Table".into(), space_after: Some(1.0), ..Default::default() });
            st.paragraph.push(para("Body", None, 8.0));
            Ok(())
        })
        .unwrap();
        let file = file_of(&with_tables_and_objects());
        let r = s
            .execute(
                "style.load",
                &json!({"base64": file, "table": ["Base Table", "Data"], "cell": ["Body Cell", "Head Cell"], "object": ["Box", "Callout"], "paragraph": ["Body"], "character": [], "conflicts": "autoRename"}),
            )
            .unwrap();
        assert_eq!(r["table"]["renamed"], json!({"Base Table": "Base Table 2", "Data": "Data 2"}));
        assert_eq!(r["cell"]["renamed"], json!({"Body Cell": "Body Cell 2"}));
        assert_eq!(r["object"]["renamed"], json!({"Box": "Box 2"}));
        assert_eq!(r["paragraph"]["renamed"], json!({"Body": "Body 2"}));
        let st = styles(&s);
        let data = table(&st, "Data 2");
        assert_eq!(
            (data.based_on.as_deref(), data.header.as_deref(), data.body.as_deref()),
            (Some("Base Table 2"), Some("Head Cell"), Some("Body Cell 2"))
        );
        let (head, body) = (cell(&st, "Head Cell"), cell(&st, "Body Cell 2"));
        assert_eq!((head.based_on.as_deref(), body.paragraph_style.as_deref()), (Some("Body Cell 2"), Some("Body 2")));
        assert_eq!(object(&st, "Callout").based_on.as_deref(), Some("Box 2"));
        assert_eq!(
            (table(&st, "Data").space_before, cell(&st, "Body Cell").fill.as_deref()),
            (Some(3.0), Some("[Paper]")),
            "the document's own stay"
        );
    }

    #[test]
    fn table_and_object_styles_load_from_an_idml_file() {
        let idml = with_tables_and_objects().execute("file.exportIdml", &json!({})).unwrap()["base64"].as_str().unwrap().to_string();
        let mut s = new_doc();
        let r = s
            .execute(
                "style.load",
                &json!({"base64": idml, "name": "Client.idml", "table": ["Data"], "object": ["Callout"], "paragraph": [], "character": []}),
            )
            .unwrap();
        let st = styles(&s);
        let data = table(&st, "Data");
        assert_eq!((data.header.as_deref(), data.body.as_deref()), (Some("Head Cell"), Some("Body Cell")));
        assert_eq!(cell(&st, "Body Cell").paragraph_style.as_deref(), Some("Body"));
        assert_eq!(object(&st, "Callout").based_on.as_deref(), Some("Box"));
        assert!(st.table.iter().all(|t| t.name != NO_TABLE_STYLE), "IDML's [No table style] doesn't come along: {:?}", r["table"]);
        assert!(s.doc().unwrap().doc.swatch("Rule Red").is_some());
    }

    #[test]
    fn a_table_cell_or_object_style_that_would_be_based_on_itself_is_refused() {
        // In the file B is based on A; in the document A is based on B.
        let pair = |a_on_b: bool| {
            let (a, b) = if a_on_b { (Some("B"), None) } else { (None, Some("A")) };
            let mut s = new_doc();
            s.edit(|d, _| {
                let st = d.styles_mut();
                for (name, on) in [("A", a), ("B", b)] {
                    let based_on = on.map(str::to_string);
                    st.table.push(TableStyle { name: name.into(), based_on: based_on.clone(), ..Default::default() });
                    st.cell.push(CellStyle { name: name.into(), based_on: based_on.clone(), ..Default::default() });
                    st.object.push(ObjectStyle { name: name.into(), based_on, ..Default::default() });
                }
                Ok(())
            })
            .unwrap();
            s
        };
        let file = file_of(&pair(false));
        let mut s = pair(true);
        for kind in ["table", "cell", "object"] {
            unchanged_after(
                &mut s,
                json!({"base64": file, kind: ["B"], "paragraph": [], "character": []}),
                &format!("{kind} style `B` would be based on itself"),
            );
        }
    }
}
