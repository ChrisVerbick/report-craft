//! Import Options › Preset: saved Word/RTF import options (`styleMap`, `styleRules`,
//! `styleConflicts`, `removeStyles`), for every document (with the user's preferences) or in one
//! document. `file.place {preset}` places with one; a name is looked up in the document first.

use std::collections::BTreeMap;

use designcraft_doc::ImportPreset;
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, ok, str_param};
use crate::{Result, Session};

/// A preset's name has at most this many characters, and a scope at most this many presets, as
/// for document presets.
const MAX_NAME: usize = 100;
const MAX_PRESETS: usize = 500;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "place.presets", "Import Presets", [], None,
            "{} → [{name, scope: document|user, styleMap, styleRules, styleConflicts, removeStyles}] — the saved Word/RTF import presets: the document's first, then the user's; `file.place {preset: name}` places with one",
            always, presets),
        cmd!(
            "place.savePreset",
            "Save Import Preset",
            [],
            None,
            "{name, scope?: user|document (default user: every document), styleMap?, styleRules?, styleConflicts?, removeStyles? (as file.place)} — saved under `name` (at most 100 characters), replacing the scope's preset of that name (case doesn't count); at most 500 in a scope. A document's presets are saved with it",
            always,
            save
        ),
        cmd!("place.deletePreset", "Delete Import Preset", [], None, "{name, scope?: user|document (default user)}", always, delete),
    ]
}

/// The import preset `name` (case doesn't count): the active document's, else the user's.
pub(super) fn find(s: &Session, name: &str, cmd: &str) -> Result<ImportPreset> {
    let named = |p: &&ImportPreset| p.name.eq_ignore_ascii_case(name.trim());
    let doc = s.doc().ok().and_then(|st| st.doc.settings.import_presets.iter().find(named).cloned());
    doc.or_else(|| s.prefs.import_presets.iter().find(named).cloned()).ok_or_else(|| bad(cmd, format!("no import preset `{name}`")))
}

/// Where a preset is kept.
#[derive(Clone, Copy)]
enum Scope {
    User,
    Document,
}

fn scope(p: &Value, cmd: &str) -> Result<Scope> {
    match p.get("scope") {
        None | Some(Value::Null) => Ok(Scope::User),
        Some(v) => match v.as_str() {
            Some("user") => Ok(Scope::User),
            Some("document") => Ok(Scope::Document),
            _ => Err(bad(cmd, "`scope` is user or document")),
        },
    }
}

fn row(p: &ImportPreset, scope: &str) -> Value {
    let conflicts = if p.style_conflicts.is_empty() { "useExisting" } else { p.style_conflicts.as_str() };
    json!({"name": p.name, "scope": scope, "styleMap": p.style_map, "styleRules": p.style_rules, "styleConflicts": conflicts, "removeStyles": p.remove_styles})
}

fn presets(s: &mut Session, _: &Value) -> Result<Value> {
    let doc = s.doc().ok().map(|st| st.doc.settings.import_presets.clone()).unwrap_or_default();
    Ok(Value::Array(doc.iter().map(|p| row(p, "document")).chain(s.prefs.import_presets.iter().map(|p| row(p, "user"))).collect()))
}

/// The preset's name from `name`, trimmed, or why it can't be one.
fn name(p: &Value, cmd: &str) -> Result<String> {
    let name = str_param(p, "name").unwrap_or_default().trim().to_string();
    if name.is_empty() {
        return Err(bad(cmd, "give the preset a `name`"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(bad(cmd, format!("a preset name has at most {MAX_NAME} characters")));
    }
    Ok(name)
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "place.savePreset";
    let name = name(p, ID)?;
    let style_conflicts = match p.get("styleConflicts") {
        None | Some(Value::Null) => String::new(),
        Some(v) => super::place_text::conflicts_name(v.as_str().unwrap_or("?"), ID)?.to_string(),
    };
    let style_map = match p.get("styleMap") {
        None | Some(Value::Null) => BTreeMap::new(),
        Some(v) => {
            let o = v.as_object().ok_or_else(|| bad(ID, "`styleMap` maps imported style names to document styles"))?;
            let pair = |(k, v): (&String, &Value)| {
                v.as_str().map(|t| (k.clone(), t.to_string())).ok_or_else(|| bad(ID, format!("`styleMap`: `{k}` must map to a style name")))
            };
            o.iter().map(pair).collect::<Result<BTreeMap<_, _>>>()?
        }
    };
    let style_rules = super::place_text::rules_param(p, ID)?.unwrap_or_default();
    let remove_styles = p.get("removeStyles").and_then(Value::as_bool).unwrap_or(false);
    let preset = ImportPreset { name: name.clone(), style_map, style_rules, style_conflicts, remove_styles };
    let put = |list: &mut Vec<ImportPreset>| -> Result<()> {
        match list.iter().position(|x| x.name.eq_ignore_ascii_case(&name)) {
            Some(i) => {
                if let Some(x) = list.get_mut(i) {
                    *x = preset.clone();
                }
            }
            None if list.len() >= MAX_PRESETS => return Err(bad(ID, format!("at most {MAX_PRESETS} import presets"))),
            None => list.push(preset.clone()),
        }
        Ok(())
    };
    match scope(p, ID)? {
        Scope::User => put(&mut s.prefs.import_presets)?,
        Scope::Document => s.edit(|d, _| put(&mut d.settings.import_presets))?,
    }
    Ok(json!({"name": name}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "place.deletePreset";
    let name = str_param(p, "name").unwrap_or_default().trim().to_string();
    let named = |x: &ImportPreset| x.name.eq_ignore_ascii_case(&name);
    let found = match scope(p, ID)? {
        Scope::User => {
            let before = s.prefs.import_presets.len();
            s.prefs.import_presets.retain(|x| !named(x));
            s.prefs.import_presets.len() < before
        }
        Scope::Document => {
            let there = s.doc()?.doc.settings.import_presets.iter().any(named);
            if there {
                s.edit(|d, _| {
                    d.settings.import_presets.retain(|x| !named(x));
                    Ok(())
                })?;
            }
            there
        }
    };
    if !found {
        return Err(bad(ID, format!("no import preset `{name}`")));
    }
    ok()
}
