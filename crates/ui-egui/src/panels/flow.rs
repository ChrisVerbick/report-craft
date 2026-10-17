//! Flow (Window › Editorial › Flow): the linked Word, Excel and text files, a
//! review of what updating one would change (a dry run of `links.update`), and what its last
//! update did.

use std::collections::HashMap;

use egui::text::LayoutJob;
use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, TextFormat, pos2, vec2};
use serde_json::{Value, json};

use crate::DesignApp;
use crate::theme::{Tokens, semibold, serif};

/// Marks on paper (the page strip), whatever the theme.
const PAPER_FILE: Color32 = Color32::from_rgb(0x1f, 0x6f, 0xeb);
const PAPER_YOURS: Color32 = Color32::from_rgb(0xc7, 0x7d, 0x00);
const PAPER_ERROR: Color32 = Color32::from_rgb(0xd9, 0x30, 0x25);
const PAPER_NOTICE: Color32 = Color32::from_rgb(0x6b, 0x6b, 0x6b);

/// What a line of the review shows: its mark and its section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Error,
    Notice,
    Conflict,
    Change,
    Kept,
    Warning,
}

/// The panel's memory between frames.
#[derive(Clone, Default)]
struct State {
    selected: Option<u64>,
    /// Each file's review: the document (uid, revision) it was made for, and the dry run's result
    /// or why the update would be refused.
    reviews: HashMap<u64, ((u64, u64), Result<Value, String>)>,
    /// The story whose "Update with N errors" box is ticked.
    allow: Option<u64>,
}

/// A line hovered in the panel, outlined on the page (`canvas::draw_flow_hover`).
#[derive(Clone, Debug, PartialEq)]
pub struct Hover {
    pub story: u64,
    /// A table cell (table, row, column) instead of a stretch of text.
    pub cell: Option<(u64, usize, usize)>,
    pub range: std::ops::Range<usize>,
    pub color: Color32,
}

/// One line of a review or record.
struct Line {
    kind: Kind,
    /// A cell or paragraph name ("B4"), above the text.
    at: Option<String>,
    /// The report entry it shows (its words, where it is), or none for an error or a warning.
    entry: Option<Value>,
    plain: String,
    sub: Option<String>,
    loc: String,
    page: Option<u64>,
    y: f32,
}

pub fn show(app: &mut DesignApp, ui: &mut egui::Ui) {
    let Some(st) = app.session.active() else { return };
    // The dock's scroll bar floats over the panel's right edge (egui's floating bars take no
    // width): what's aligned right stays clear of it.
    ui.set_max_width(ui.available_width() - ui.spacing().scroll.bar_width);
    let rev = (st.uid, st.revision);
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language.clone();
    let tr = |s: &'static str| crate::i18n::tr(&lang, s).to_string();
    let key = egui::Id::new("flow_panel");
    let mut state: State = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    if let Some(sid) = app.ui.flow_request.take() {
        state.selected = Some(sid);
        state.reviews.remove(&sid);
    }
    let rows = text_rows(app, ui, rev);
    let modified = |r: &Value| r["status"] == "modified";
    if state.selected.is_none_or(|s| !rows.iter().any(|r| r["story"].as_u64() == Some(s))) {
        state.selected = rows.iter().find(|r| modified(r)).or(rows.first()).and_then(|r| r["story"].as_u64());
    }

    ui.horizontal(|ui| {
        crate::rtl::label(ui, RichText::new(tr("Linked files")).font(semibold(11.5)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(rows.iter().any(modified), egui::Button::new(tr("Check all"))).clicked() {
                for sid in rows.iter().filter(|r| modified(r)).filter_map(|r| r["story"].as_u64()) {
                    state.reviews.insert(sid, (rev, dry_run(app, sid)));
                }
            }
            if ui.button(tr("Place with Flow…")).clicked() {
                let _ = app.run("app.placeWithFlow", json!({}));
            }
        });
    });
    if rows.is_empty() {
        crate::rtl::label(
            ui,
            RichText::new(tr("No linked text files. Place a Word, Excel or text file with Create Links to update it here.")).color(t.text_dim),
        );
        ui.data_mut(|d| d.insert_temp(key, state));
        return;
    }
    for r in &rows {
        let Some(sid) = r["story"].as_u64() else { continue };
        let review = state.reviews.get(&sid).filter(|(at, _)| *at == rev).and_then(|(_, r)| r.as_ref().ok());
        if file_row(ui, &t, &lang, r, state.selected == Some(sid), review.filter(|_| modified(r))).clicked() {
            state.selected = Some(sid);
        }
    }
    ui.separator();

    let Some(row) = state.selected.and_then(|s| rows.iter().find(|r| r["story"].as_u64() == Some(s))).cloned() else {
        ui.data_mut(|d| d.insert_temp(key, state));
        return;
    };
    let sid = row["story"].as_u64().unwrap_or(0);
    let name = row["name"].as_str().unwrap_or_default().to_string();
    crate::rtl::label(ui, RichText::new(&name).font(semibold(12.0)));
    match row["status"].as_str().unwrap_or_default() {
        "missing" => {
            crate::rtl::label(ui, tr("Can't find this file. Its story keeps the text from the last update."));
            if ui.button(tr("Relink in the Links panel")).clicked() {
                let _ = app.run("window.panel", json!({"panel": "links"}));
            }
        }
        "modified" => {
            if !state.reviews.get(&sid).is_some_and(|(at, _)| *at == rev) {
                state.reviews.insert(sid, (rev, dry_run(app, sid)));
            }
            let review = state.reviews.get(&sid).map(|(_, r)| r.clone());
            let when = match ago(row["lastUpdate"]["at"].as_i64().unwrap_or(0), &lang) {
                w if w.is_empty() => tr("The file changed since it was placed or last updated."),
                w => format!("{} {w}.", tr("The file changed since its last update,")),
            };
            crate::rtl::label(ui, RichText::new(when).color(t.text_dim));
            if row["merges"] != true {
                let text = if row["edited"] == true {
                    tr("Updates replace this story's text: it was placed before Flow could merge. Your edits to it in the layout will be lost.")
                } else {
                    tr("Updates replace this story's text: it was placed before Flow could merge.")
                };
                crate::rtl::label(ui, RichText::new(text).color(t.flow_error));
            }
            let mut recheck = false;
            let mut update = false;
            match review {
                Some(Ok(result)) => {
                    let report = result["reports"].as_array().and_then(|r| r.iter().find(|x| x["story"].as_u64() == Some(sid))).cloned();
                    let errors: Vec<Value> = result["errors"]
                        .as_array()
                        .map(|e| e.iter().filter(|x| x["story"].as_u64() == Some(sid)).cloned().collect())
                        .unwrap_or_default();
                    // An update that replaces the story warns that edits were replaced: the notice
                    // above says it before the update.
                    let warnings: Vec<String> = result["warnings"]
                        .as_array()
                        .map(|w| {
                            w.iter().filter_map(Value::as_str).filter(|w| !w.contains("in the layout were replaced")).map(str::to_string).collect()
                        })
                        .unwrap_or_default();
                    let sections = sections(sid, report.as_ref(), &errors, &warnings, true);
                    report_view(app, ui, &t, &lang, sid, report.as_ref(), &sections, true);
                    let blocking = errors.iter().filter(|e| e["blocks"] != false).count();
                    if blocking > 0 {
                        let label = format!("{} {blocking} {}", tr("Update with"), tr(if blocking == 1 { "error" } else { "errors" }));
                        let mut allow = state.allow == Some(sid);
                        if ui.checkbox(&mut allow, label).changed() {
                            state.allow = allow.then_some(sid);
                        }
                    }
                    ui.horizontal(|ui| {
                        recheck = ui.button(tr("Check again")).clicked();
                        let primary = egui::Button::new(RichText::new(tr("Update")).color(Color32::WHITE)).fill(t.accent_strong);
                        update = ui.add_enabled(blocking == 0 || state.allow == Some(sid), primary).clicked();
                    });
                }
                Some(Err(why)) => {
                    crate::rtl::label(ui, RichText::new(why).color(t.flow_error));
                    recheck = ui.button(tr("Check again")).clicked();
                }
                None => {}
            }
            if recheck {
                state.reviews.remove(&sid);
            }
            if update {
                let allow = state.allow == Some(sid);
                if crate::panels::properties::run_update_links(app, json!({"story": sid, "allowErrors": allow})).is_ok() {
                    state.reviews.remove(&sid);
                    state.allow = None;
                    ui.data_mut(|d| d.remove::<(f64, (u64, u64), Vec<Value>)>(egui::Id::new("flow_rows")));
                }
            }
        }
        _ => match row.get("lastUpdate").filter(|l| l.is_object()) {
            Some(rec) => {
                let when = ago(rec["at"].as_i64().unwrap_or(0), &lang);
                let head = if when.is_empty() { tr("Updated") } else { format!("{} {when}", tr("Updated")) };
                crate::rtl::label(ui, RichText::new(head).color(t.text_dim));
                let report = Some(&rec["report"]).filter(|r| r["replaced"] != true);
                let errors = rec["errors"].as_array().cloned().unwrap_or_default();
                let sections = sections(sid, report, &errors, &[], false);
                report_view(app, ui, &t, &lang, sid, report, &sections, false);
                // The page's highlights of what this update brought in (F2.9).
                let marks = flow_marks(app, sid);
                if marks.0 || marks.1 {
                    ui.add_space(6.0);
                    let mut on = app.session.prefs.flow_highlights;
                    if ui.checkbox(&mut on, tr("Highlight on the page")).changed() {
                        let _ = app.run("prefs.set", json!({"flowHighlights": on}));
                    }
                    if ui.add_enabled(marks.0, egui::Button::new(tr("Clear highlights"))).clicked() {
                        let _ = app.run("links.clearHighlights", json!({"story": sid}));
                    }
                    if marks.1 {
                        crate::rtl::label(ui, RichText::new(tr("Errors stay highlighted until they're fixed.")).color(t.text_dim).size(10.5));
                    }
                }
            }
            None => {
                crate::rtl::label(ui, RichText::new(tr("Up to date.")).color(t.text_dim));
            }
        },
    }
    ui.data_mut(|d| d.insert_temp(key, state));
}

/// Place with Flow's dialog, below the import options: what the file shows as errors, a preview,
/// "Place with N errors" when it shows errors, and that it is placed linked.
pub(crate) fn place_check(ui: &mut egui::Ui, lang: &str, d: &mut crate::dialogs::Dialog) {
    let t = Tokens::get(ui.ctx());
    let tr = |s: &'static str| crate::i18n::tr(lang, s).to_string();
    let problems = d.fields.get("problems").and_then(Value::as_array).cloned().unwrap_or_default();
    ui.add_space(8.0);
    crate::rtl::label(ui, RichText::new(tr("Found in the file")).font(semibold(12.0)));
    if problems.is_empty() {
        crate::rtl::label(ui, RichText::new(tr("Nothing that needs your attention.")).color(t.text_dim));
    }
    let width = ui.available_width();
    for (kind, lines) in sections(0, None, &problems, &[], true) {
        for (i, l) in lines.iter().enumerate() {
            line_row(ui, &t, lang, l, ui.id().with(("place_check", kind as u8, i)));
        }
    }
    ui.add_space(8.0);
    crate::rtl::label(ui, RichText::new(tr("Preview")).font(semibold(12.0)));
    let preview = d.fields.get("preview").cloned().unwrap_or(Value::Null);
    egui::Frame::NONE.fill(Color32::WHITE).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        ui.set_width(width - 16.0);
        let ink = Color32::from_gray(26);
        if let Some(rows) = preview["rows"].as_array() {
            egui::Grid::new("place_preview").spacing([12.0, 3.0]).show(ui, |ui| {
                for r in rows {
                    for c in r.as_array().into_iter().flatten() {
                        ui.label(RichText::new(c.as_str().unwrap_or_default()).color(ink).size(11.0));
                    }
                    ui.end_row();
                }
            });
            if let Some(more) = preview["more"].as_u64().filter(|m| *m > 0) {
                ui.label(
                    RichText::new(format!("{more} {}", tr(if more == 1 { "more row" } else { "more rows" })))
                        .color(Color32::from_gray(107))
                        .size(10.5),
                );
            }
        } else {
            for p in preview["paragraphs"].as_array().into_iter().flatten() {
                ui.add(egui::Label::new(RichText::new(p.as_str().unwrap_or_default()).font(serif(12.0)).color(ink)).wrap());
            }
        }
    });
    ui.add_space(6.0);
    crate::rtl::label(
        ui,
        RichText::new(tr(
            "Placed linked: when the file changes, Flow shows you the update before anything on the page changes, and keeps your layout edits.",
        ))
        .color(t.text_dim)
        .size(10.5),
    );
    let blocking = problems.iter().filter(|e| e["blocks"] != false).count();
    if blocking > 0 {
        let mut allow = d.fields.get("allow").and_then(Value::as_bool).unwrap_or(false);
        let label = format!("{} {blocking} {}", tr("Place with"), tr(if blocking == 1 { "error" } else { "errors" }));
        if ui.checkbox(&mut allow, label).changed() {
            d.fields.insert("allow".into(), json!(allow));
        }
    }
}

/// `links.list`'s linked text files, read at most twice a second (it looks at the files).
fn text_rows(app: &mut DesignApp, ui: &egui::Ui, rev: (u64, u64)) -> Vec<Value> {
    let key = egui::Id::new("flow_rows");
    let now = ui.input(|i| i.time);
    let cached: Option<(f64, (u64, u64), Vec<Value>)> = ui.data(|d| d.get_temp(key));
    match cached {
        Some((at, r, rows)) if r == rev && now - at < 0.5 => rows,
        _ => {
            let all = app.session.execute("links.list", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            let rows: Vec<Value> = all.into_iter().filter(|r| r.get("story").is_some()).collect();
            ui.data_mut(|d| d.insert_temp(key, (now, rev, rows.clone())));
            rows
        }
    }
}

/// What updating story `sid` would do, from a dry run that changes nothing; or why it would be
/// refused. Errors the file shows are let through so the review can list them.
fn dry_run(app: &mut DesignApp, sid: u64) -> Result<Value, String> {
    app.session.execute("links.update", &json!({"story": sid, "dryRun": true, "allowErrors": true})).map_err(|e| e.to_string())
}

/// One linked file: its kind, name and status, and once checked a tally of what an update brings.
fn file_row(ui: &mut egui::Ui, t: &Tokens, lang: &str, r: &Value, selected: bool, review: Option<&Value>) -> egui::Response {
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let (icon, _) = ui.allocate_exact_size(vec2(13.0, 14.0), Sense::hover());
            file_icon(ui.painter(), icon, r["name"].as_str().unwrap_or_default(), t.text_dim);
            let (status, color) = match r["status"].as_str().unwrap_or_default() {
                "modified" => ("File changed", t.text),
                "missing" => ("Missing", t.flow_error),
                "tooLarge" => ("Too large to check", t.text_dim),
                _ => ("Up to date", t.text_dim),
            };
            crate::rtl::label(ui, r["name"].as_str().unwrap_or_default());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                crate::rtl::label(ui, RichText::new(crate::i18n::tr(lang, status)).color(color));
            });
        });
        if let Some(result) = review {
            let sid = r["story"].as_u64();
            let report = result["reports"].as_array().and_then(|x| x.iter().find(|x| x["story"].as_u64() == sid)).cloned();
            let errors: Vec<Value> =
                result["errors"].as_array().map(|e| e.iter().filter(|x| x["story"].as_u64() == sid).cloned().collect()).unwrap_or_default();
            let tally = sections(sid.unwrap_or(0), report.as_ref(), &errors, &[], true);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                for (kind, lines) in tally.iter().filter(|(k, _)| *k != Kind::Warning) {
                    let (m, _) = ui.allocate_exact_size(vec2(12.0, 14.0), Sense::hover());
                    paint_mark(ui.painter(), m.center(), *kind, t);
                    crate::rtl::label(ui, lines.len().to_string());
                    ui.add_space(4.0);
                }
            });
        }
    });
    let rect = inner.response.rect.expand2(vec2(4.0, 2.0));
    let resp = ui.interact(rect, ui.id().with(("flow_file", r["story"].as_u64())), Sense::click());
    if selected || resp.hovered() {
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 0.0, if selected { t.row_selected } else { t.hover }));
    }
    resp
}

/// A review or a record in sections, attention first: errors, values from other workbooks,
/// conflicts, changes from the file, the layout's edits kept, warnings. Empty sections are left out.
fn sections(sid: u64, report: Option<&Value>, errors: &[Value], warnings: &[String], before: bool) -> Vec<(Kind, Vec<Line>)> {
    let entries = |key: &str| report.and_then(|r| r[key].as_array()).cloned().unwrap_or_default();
    let line = |kind: Kind, e: Value| {
        let plain = match kind {
            Kind::Change => e["after"].as_str(),
            _ => e["layout"].as_str(),
        }
        .unwrap_or_default()
        .to_string();
        let at = e.get("cell").filter(|c| c.is_object()).map(cell_name);
        let loc = if e["overset"] == true { "overset".to_string() } else { e["pageName"].as_str().map(|p| format!("p. {p}")).unwrap_or_default() };
        let sub = (kind == Kind::Conflict).then(|| if before { "Yours moves to a note." } else { "Yours is in a note." }.to_string());
        Line { kind, at, plain, sub, loc, page: e["page"].as_u64(), y: e["y"].as_f64().unwrap_or(1.0) as f32, entry: Some(e) }
    };
    let error = |e: &Value| {
        let blocks = e["blocks"] != false;
        let formula = e["formula"].as_str().map(|f| format!("={f}"));
        let sub = match (e["kind"].as_str(), formula) {
            (Some("otherWorkbook"), Some(f)) => Some(format!("From {f}. It may be out of date.")),
            (Some("notCalculated"), Some(f)) => Some(format!("Formula {f} has no calculated value: open and save the file in Excel.")),
            (Some("fieldError"), _) => Some("Word can't fill this field.".to_string()),
            (_, Some(f)) => Some(format!("Formula {f}")),
            _ => None,
        };
        Line {
            kind: if blocks { Kind::Error } else { Kind::Notice },
            at: e["at"].as_str().map(str::to_string),
            entry: e.get("cell").filter(|c| c.is_object()).map(|c| json!({"cell": c, "story": sid})),
            plain: e["shows"].as_str().unwrap_or_default().to_string(),
            sub,
            loc: String::new(),
            page: None,
            y: 0.0,
        }
    };
    let mut out: Vec<(Kind, Vec<Line>)> = vec![
        (Kind::Error, errors.iter().filter(|e| e["blocks"] != false).map(error).collect()),
        (Kind::Notice, errors.iter().filter(|e| e["blocks"] == false).map(error).collect()),
        (Kind::Conflict, entries("conflicts").into_iter().map(|e| line(Kind::Conflict, e)).collect()),
        (Kind::Change, entries("changes").into_iter().map(|e| line(Kind::Change, e)).collect()),
        (Kind::Kept, entries("deviations").into_iter().map(|e| line(Kind::Kept, e)).collect()),
    ];
    let mut warns: Vec<String> = warnings.to_vec();
    for w in report.and_then(|r| r["warnings"].as_array()).into_iter().flatten().filter_map(Value::as_str) {
        if !warns.iter().any(|x| x == w) {
            warns.push(w.to_string());
        }
    }
    out.push((
        Kind::Warning,
        warns
            .into_iter()
            .map(|w| Line { kind: Kind::Warning, at: None, entry: None, plain: w, sub: None, loc: String::new(), page: None, y: 0.0 })
            .collect(),
    ));
    out.retain(|(_, l)| !l.is_empty());
    out
}

/// A placed table's cell as Excel names it from the table's first cell ("B2"), for a cell entry.
fn cell_name(c: &Value) -> String {
    let (row, col) = (c["row"].as_u64().unwrap_or(0), c["col"].as_u64().unwrap_or(0));
    let mut letters = Vec::new();
    let mut n = col.saturating_add(1);
    while n > 0 {
        letters.push(char::from(b'A' + ((n - 1) % 26) as u8));
        n = (n - 1) / 26;
    }
    format!("{}{}", letters.iter().rev().collect::<String>(), row.saturating_add(1))
}

/// The page strip, the 0% check, the fit, and the sections. `before`: a review (Go To finds the
/// place in the story as it is now); otherwise a record.
#[allow(clippy::too_many_arguments)]
fn report_view(
    app: &mut DesignApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    lang: &str,
    sid: u64,
    report: Option<&Value>,
    sections: &[(Kind, Vec<Line>)],
    before: bool,
) {
    let tr = |s: &'static str| crate::i18n::tr(lang, s).to_string();
    if let Some(r) = report {
        if let Some(page) = strip(ui, t, r, sections) {
            crate::canvas::go_to_page(app, page as usize);
        }
        ui.horizontal(|ui| {
            let (m, _) = ui.allocate_exact_size(vec2(12.0, 14.0), Sense::hover());
            let c = m.center();
            ui.painter().line(vec![c + vec2(-4.0, 0.0), c + vec2(-1.0, 3.0), c + vec2(4.5, -3.5)], Stroke::new(1.6, t.flow_ok));
            crate::rtl::label(ui, tr(if before { "Every difference is listed. No other text changes." } else { "Every difference was listed." }));
        });
        let lines = r["fit"]["lines"].as_u64().unwrap_or(0);
        if r["fit"]["overset"] == true {
            let n = if lines == 1 { tr("1 line won't fit") } else { format!("{lines} {}", tr("lines won't fit")) };
            let rest = tr(if before { "the story will end overset." } else { "the story ends overset." });
            crate::rtl::label(ui, RichText::new(format!("{n}: {rest}")).color(t.flow_error));
        } else if r.get("fit").is_some() {
            crate::rtl::label(ui, RichText::new(tr("The story still fits its frames.")).color(t.text_dim));
        }
    }
    for (kind, lines) in sections {
        let (title, note) = match (kind, before) {
            (Kind::Error, true) => ("Errors in the file", Some("Fix them in the file and check again, or update with them.")),
            (Kind::Error, false) => ("Errors brought in", None),
            (Kind::Notice, _) => ("Values from other workbooks", before.then_some("These may be out of date. They don't stop the update.")),
            (Kind::Conflict, true) => ("Conflicts", Some("You and the file both changed these. The file's text goes in; yours moves to a note.")),
            (Kind::Conflict, false) => ("Conflicts, moved to notes", None),
            (Kind::Change, _) => ("Changes from the file", None),
            (Kind::Kept, _) => ("Your edits kept", before.then_some("The file differs here. The layout keeps your version.")),
            (Kind::Warning, _) => ("Warnings", None),
        };
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let (m, _) = ui.allocate_exact_size(vec2(12.0, 14.0), Sense::hover());
            paint_mark(ui.painter(), m.center(), *kind, t);
            crate::rtl::label(ui, RichText::new(tr(title)).font(semibold(11.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                crate::rtl::label(ui, RichText::new(lines.len().to_string()).color(t.text_dim));
            });
        });
        if let Some(note) = note {
            crate::rtl::label(ui, RichText::new(tr(note)).color(t.text_dim).size(10.5));
        }
        let table = first_table(app, sid);
        for (i, l) in lines.iter().enumerate() {
            let resp = line_row(ui, t, lang, l, ui.id().with(("flow_line", *kind as u8, i)));
            if let Some(e) = &l.entry {
                if resp.hovered() {
                    app.ui.flow_hover = hover(sid, *kind, e, before, table);
                }
                if resp.clicked() {
                    go_to(app, sid, e, before);
                }
            }
        }
    }
}

/// One line: its mark, where it is (a cell), the report's words as they change, a note, and the
/// page. Clicking it goes there.
fn line_row(ui: &mut egui::Ui, t: &Tokens, lang: &str, l: &Line, id: egui::Id) -> egui::Response {
    let bg = ui.painter().add(egui::Shape::Noop);
    let width = ui.available_width();
    let inner = ui.horizontal_top(|ui| {
        let (m, _) = ui.allocate_exact_size(vec2(12.0, 16.0), Sense::hover());
        paint_mark(ui.painter(), m.center(), l.kind, t);
        let text_w = (width - 12.0 - 40.0).max(60.0);
        ui.vertical(|ui| {
            ui.set_width(text_w);
            ui.spacing_mut().item_spacing.y = 1.0;
            if let Some(at) = &l.at {
                crate::rtl::label(ui, RichText::new(at).font(semibold(11.0)));
            }
            ui.add(egui::Label::new(quote(t, l, text_w)).wrap());
            if let Some(sub) = &l.sub {
                crate::rtl::label(ui, RichText::new(crate::i18n::tr(lang, sub)).color(t.text_dim).size(10.5));
            }
        });
        crate::rtl::label(ui, RichText::new(&l.loc).color(t.text_dim).size(10.0));
    });
    let rect = inner.response.rect.expand2(vec2(2.0, 1.0));
    let sense = if l.entry.is_some() { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, id, sense);
    if l.entry.is_some() && resp.hovered() {
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 0.0, t.hover));
    }
    resp
}

/// The report's words in its own face: the file's additions underlined and its deletions struck
/// in blue; the layout's text kept underlined in amber, the file's version it replaced struck;
/// in a conflict the file's text underlined in blue and the layout's, going to a note, struck in
/// amber. An error shows what the file shows, in red.
fn quote(t: &Tokens, l: &Line, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let font = if l.kind == Kind::Warning { FontId::proportional(11.0) } else { serif(12.0) };
    let base = TextFormat { font_id: font, color: if l.kind == Kind::Error { t.flow_error } else { t.text }, ..Default::default() };
    let (ins, del) = match l.kind {
        Kind::Kept => (t.flow_yours, t.flow_yours),
        Kind::Conflict => (t.flow_file, t.flow_yours),
        _ => (t.flow_file, t.flow_file),
    };
    match l.entry.as_ref().and_then(|e| e["words"].as_array()).filter(|w| !w.is_empty()) {
        Some(words) => {
            let mut after_deleted = false;
            for w in words {
                let mut f = base.clone();
                // A little room between what goes and what comes in its place.
                let mut gap = 0.0;
                match w["kind"].as_str() {
                    Some("inserted") => {
                        f.underline = Stroke::new(1.5, ins);
                        gap = if after_deleted { 3.0 } else { 0.0 };
                    }
                    Some("deleted") => {
                        f.strikethrough = Stroke::new(1.0, del);
                        f.color = t.text_dim;
                    }
                    _ => {}
                }
                after_deleted = w["kind"] == "deleted";
                job.append(w["text"].as_str().unwrap_or_default(), gap, f);
            }
        }
        None => job.append(&l.plain, 0.0, base),
    }
    job
}

/// What hovering a line outlines on the page: its cell, or its text as it is now (while
/// reviewing) or after the update. `table`: the story's table, for an error that names only a
/// row and column.
fn hover(sid: u64, kind: Kind, e: &Value, before: bool, table: Option<u64>) -> Option<Hover> {
    let color = match kind {
        Kind::Kept | Kind::Conflict => PAPER_YOURS,
        Kind::Error => PAPER_ERROR,
        Kind::Notice => PAPER_NOTICE,
        _ => PAPER_FILE,
    };
    let index = |v: &Value| v.as_u64().and_then(|n| usize::try_from(n).ok());
    if let Some(c) = e.get("cell").filter(|c| c.is_object()) {
        let cell = (c["table"].as_u64().or(table)?, index(&c["row"])?, index(&c["col"])?);
        return Some(Hover { story: sid, cell: Some(cell), range: 0..0, color });
    }
    let at = if before { e.get("was").filter(|w| w.is_object()).unwrap_or(&e["at"]) } else { &e["at"] };
    Some(Hover { story: sid, cell: None, range: index(&at["start"])?..index(&at["end"])?, color })
}

/// Go to a line's place: select the text (as it is now, while reviewing) or the cell, and show
/// its page.
fn go_to(app: &mut DesignApp, sid: u64, e: &Value, before: bool) {
    if let Some(c) = e.get("cell").filter(|c| c.is_object()) {
        let (row, col) = (c["row"].as_u64().unwrap_or(0), c["col"].as_u64().unwrap_or(0));
        let table = c["table"].as_u64().or_else(|| first_table(app, sid));
        let _ = app.run("table.select", json!({"story": sid, "table": table, "rows": [row, row], "cols": [col, col]}));
    } else {
        let at = if before { e.get("was").filter(|w| w.is_object()).unwrap_or(&e["at"]) } else { &e["at"] };
        if let (Some(a), Some(b)) = (at["start"].as_u64(), at["end"].as_u64()) {
            let _ = app.run("text.select", json!({"story": sid, "anchor": a, "focus": b}));
        }
    }
    if let Some(page) = e["page"].as_u64() {
        crate::canvas::go_to_page(app, page as usize);
    }
}

/// Whether story `sid` has Flow highlights, other than errors and of errors, in its text or cells.
fn flow_marks(app: &DesignApp, sid: u64) -> (bool, bool) {
    use designcraft_doc::FlowMark;
    let Some(st) = app.session.active().and_then(|d| d.doc.story(designcraft_doc::StoryId(sid))) else { return (false, false) };
    let texts = std::iter::once(st).chain(st.tables.values().flat_map(|t| t.cells.iter().map(|c| &c.text)));
    let mut found = (false, false);
    for text in texts {
        for (_, f) in text.runs() {
            match f.over.flow {
                Some(FlowMark::Error) => found.1 = true,
                Some(FlowMark::None) | None => {}
                Some(_) => found.0 = true,
            }
        }
    }
    found
}

/// The story's first table (an error in a spreadsheet cell names its row and column only).
fn first_table(app: &DesignApp, sid: u64) -> Option<u64> {
    app.session.active().and_then(|st| st.doc.story(designcraft_doc::StoryId(sid))).and_then(|s| s.tables.keys().next().copied())
}

/// The story's pages in miniature with a tick where each difference lands, and a boxed + after
/// the last page when the story won't fit. Returns the page clicked.
fn strip(ui: &mut egui::Ui, t: &Tokens, report: &Value, sections: &[(Kind, Vec<Line>)]) -> Option<u64> {
    let pages = report["pages"].as_array().filter(|p| !p.is_empty())?;
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        for p in pages {
            let index = p["index"].as_u64().unwrap_or(0);
            let name = p["name"].as_str().unwrap_or_default();
            let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 58.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Page {name}")));
            let page = Rect::from_min_size(rect.min, vec2(34.0, 44.0));
            let painter = ui.painter();
            painter.rect_filled(page.translate(vec2(1.0, 1.0)), 0.0, Color32::from_black_alpha(110));
            painter.rect_filled(page, 0.0, Color32::WHITE);
            let mut y = page.top() + 6.0;
            while y < page.bottom() - 5.0 {
                painter.line_segment([pos2(page.left() + 6.0, y), pos2(page.right() - 4.0, y)], Stroke::new(1.0, Color32::from_gray(225)));
                y += 3.0;
            }
            for (kind, lines) in sections {
                for l in lines.iter().filter(|l| l.page == Some(index)) {
                    paint_tick(painter, pos2(page.left() + 2.0, page.top() + l.y.clamp(0.0, 1.0) * page.height()), *kind);
                }
            }
            painter.text(pos2(page.center().x, page.bottom() + 7.0), Align2::CENTER_CENTER, name, FontId::proportional(10.0), t.text_dim);
            if resp.clicked() {
                clicked = Some(index);
            }
        }
        if report["fit"]["overset"] == true {
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 44.0), Sense::hover());
            paint_mark(ui.painter(), pos2(r.center().x, r.top() + 22.0), Kind::Error, t);
        }
    });
    clicked
}

/// A difference's mark in the panel: a bar for a change from the file, dots for an edit of yours
/// kept, a circled ? for a conflict, a filled ! for an error, a circled i for a value from another
/// workbook, a dash for a warning.
fn paint_mark(p: &egui::Painter, c: egui::Pos2, kind: Kind, t: &Tokens) {
    match kind {
        Kind::Change => {
            p.rect_filled(Rect::from_center_size(c, vec2(3.0, 13.0)), 1.0, t.flow_file);
        }
        Kind::Kept => {
            for i in 0..3 {
                p.circle_filled(pos2(c.x, c.y - 4.5 + i as f32 * 4.5), 1.4, t.flow_yours);
            }
        }
        Kind::Conflict => {
            p.circle_stroke(c, 5.5, Stroke::new(1.4, t.flow_yours));
            p.text(c, Align2::CENTER_CENTER, "?", semibold(9.0), t.flow_yours);
        }
        Kind::Error => {
            p.circle_filled(c, 6.0, t.flow_error_fill);
            p.text(c, Align2::CENTER_CENTER, "!", semibold(9.5), Color32::WHITE);
        }
        Kind::Notice => {
            p.circle_stroke(c, 5.5, Stroke::new(1.2, t.text_dim));
            p.text(c, Align2::CENTER_CENTER, "i", semibold(9.0), t.text_dim);
        }
        Kind::Warning => {
            p.line_segment([c + vec2(-4.0, 0.0), c + vec2(4.0, 0.0)], Stroke::new(1.6, t.text_dim));
        }
    }
}

/// A difference's tick on a miniature page, in paper colours.
fn paint_tick(p: &egui::Painter, at: egui::Pos2, kind: Kind) {
    match kind {
        Kind::Change => {
            p.rect_filled(Rect::from_min_size(at - vec2(0.0, 1.0), vec2(9.0, 2.0)), 0.0, PAPER_FILE);
        }
        Kind::Kept => {
            for i in 0..3 {
                p.circle_filled(pos2(at.x + 1.0 + i as f32 * 3.5, at.y), 1.0, PAPER_YOURS);
            }
        }
        Kind::Conflict => {
            p.circle_stroke(at + vec2(3.0, 0.0), 2.5, Stroke::new(1.2, PAPER_YOURS));
        }
        Kind::Error => {
            p.circle_filled(at + vec2(3.0, 0.0), 2.5, PAPER_ERROR);
        }
        Kind::Notice => {
            p.circle_stroke(at + vec2(3.0, 0.0), 2.5, Stroke::new(1.0, PAPER_NOTICE));
        }
        Kind::Warning => {}
    }
}

/// A small sign of what kind of file it is: a page for a document, a grid for a spreadsheet,
/// lines for text.
fn file_icon(p: &egui::Painter, r: Rect, name: &str, c: Color32) {
    let s = Stroke::new(1.1, c);
    let lower = name.to_ascii_lowercase();
    let r = r.shrink2(vec2(1.0, 1.0));
    if lower.ends_with(".xlsx") {
        p.rect_stroke(r, 0.0, s, egui::StrokeKind::Inside);
        for f in [0.35, 0.68] {
            let y = r.top() + r.height() * f;
            p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], s);
        }
        let x = r.left() + r.width() * 0.4;
        p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], s);
    } else if lower.ends_with(".docx") || lower.ends_with(".rtf") || lower.ends_with(".doc") {
        p.rect_stroke(r, 0.0, s, egui::StrokeKind::Inside);
        for f in [0.45, 0.65] {
            let y = r.top() + r.height() * f;
            p.line_segment([pos2(r.left() + 2.5, y), pos2(r.right() - 2.5, y)], s);
        }
    } else {
        for f in [0.2, 0.45, 0.7, 0.95] {
            let y = r.top() + r.height() * f;
            p.line_segment([pos2(r.left(), y), pos2(r.right() - if f > 0.9 { 4.0 } else { 0.0 }, y)], s);
        }
    }
}

/// "just now", "5 minutes ago"…, for a time in seconds since 1970 (empty when unknown, as on the web).
#[cfg(not(target_arch = "wasm32"))]
fn ago(at: i64, lang: &str) -> String {
    if at <= 0 {
        return String::new();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let s = now.saturating_sub(at).max(0);
    let n = |v: i64, one: &'static str, many: &'static str| format!("{v} {}", crate::i18n::tr(lang, if v == 1 { one } else { many }));
    match s {
        0..=59 => crate::i18n::tr(lang, "just now").to_string(),
        60..=3599 => n(s / 60, "minute ago", "minutes ago"),
        3600..=86_399 => n(s / 3600, "hour ago", "hours ago"),
        _ => n(s / 86_400, "day ago", "days ago"),
    }
}

#[cfg(target_arch = "wasm32")]
fn ago(_at: i64, _lang: &str) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable as _;
    use serde_json::json;

    use crate::DesignApp;

    /// A document with a text file `name` placed linked from a temporary folder: the file, the
    /// app and the story.
    fn linked(name: &str, text: &str) -> (std::path::PathBuf, DesignApp, u64) {
        let dir = std::env::temp_dir().join(format!("dc-flow-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let mut app = DesignApp::new(designcraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", &json!({})).unwrap();
        let r = app.session.execute("file.place", &json!({"path": path.to_string_lossy(), "link": true, "rect": [72, 72, 300, 200]})).unwrap();
        (path, app, r["story"].as_u64().unwrap())
    }

    fn text(app: &DesignApp, sid: u64) -> String {
        app.session.doc().unwrap().doc.story(designcraft_doc::StoryId(sid)).unwrap().text.clone()
    }

    fn harness(app: DesignApp) -> egui_kittest::Harness<'static, DesignApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut DesignApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        )
    }

    /// The dock's scroll bar floats over a panel's right edge (egui's floating bars take no
    /// width): what the panel aligns right stays clear of it.
    #[test]
    fn what_is_aligned_right_stays_clear_of_the_scroll_bar() {
        let (path, app, _) = linked("scrollbar.txt", "One.");
        // The app's fonts take effect from the frame after they're installed.
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(320.0, 400.0)).build_ui_state(
            move |ui, app: &mut DesignApp| {
                if !fonts {
                    fonts = true;
                    crate::theme::install_fonts(ui.ctx(), "en");
                    return;
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| super::show(app, ui));
            },
            app,
        );
        h.run_steps(3);
        // The scroll area spans the window less the same margin on each side.
        let margin = h.get_by_label("Linked files").rect().min.x;
        let bar = egui::style::ScrollStyle::default().bar_width;
        for label in ["Up to date", "Check all"] {
            let right = h.get_by_label(label).rect().max.x;
            assert!(right <= 320.0 - margin - bar, "{label} ends at {right}, under the bar");
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Update on a linked text file (the Links panel, Update Link) opens its review in Flow:
    /// nothing changes until it is updated there.
    #[test]
    fn update_on_linked_text_opens_its_review() {
        let (path, mut app, sid) = linked("open.txt", "One.");
        std::fs::write(&path, "One. Two.").unwrap();
        crate::panels::properties::update_links(&mut app, json!({"story": sid}));
        assert_eq!((app.ui.open_panel.as_deref(), app.ui.flow_request), (Some("flow"), Some(sid)));
        assert!(app.ui.dialog.is_none());
        assert_eq!(text(&app, sid), "One.");
        app.ui.open_panel = None;
        app.ui.flow_request = None;
        crate::menus::activate(&mut app, "links.update", &serde_json::Value::Null);
        assert_eq!((app.ui.open_panel.as_deref(), app.ui.flow_request), (Some("flow"), Some(sid)), "Update Link on all links");
        assert_eq!(text(&app, sid), "One.");
    }

    /// The review shows what the file changes and that nothing else does; Update applies it and
    /// the panel then shows the update's record.
    #[test]
    fn the_review_shows_the_changes_and_update_applies_them() {
        let (path, mut app, sid) = linked("review.txt", "One.");
        std::fs::write(&path, "One. Two.").unwrap();
        crate::panels::properties::update_links(&mut app, json!({"story": sid}));
        let mut h = harness(app);
        h.run_steps(3);
        h.get_by_label("Changes from the file");
        h.get_by_label("Every difference is listed. No other text changes.");
        h.get_by_label("Update").click();
        h.run_steps(3);
        assert_eq!(text(h.state(), sid), "One. Two.");
        // The panel shows the update's record.
        h.get_by_label("Updated just now");
        h.get_by_label("Every difference was listed.");
    }

    /// A file that shows errors lists them first, and Update waits until "Update with N errors"
    /// is ticked.
    #[test]
    fn errors_come_first_and_update_waits_for_the_box() {
        let (path, mut app, sid) = linked("errors.txt", "One.");
        std::fs::write(&path, "See Error! Reference source not found. here.").unwrap();
        crate::panels::properties::update_links(&mut app, json!({"story": sid}));
        let mut h = harness(app);
        h.run_steps(3);
        h.get_by_label("Errors in the file");
        h.get_by_label("Update").click();
        h.run_steps(2);
        assert_eq!(text(h.state(), sid), "One.", "not without the box");
        h.get_by_label("Update with 1 error").click();
        h.run_steps(2);
        h.get_by_label("Update").click();
        h.run_steps(3);
        assert_eq!(text(h.state(), sid), "See Error! Reference source not found. here.");
    }

    /// Not a check: renders a review into `plan/shots/` to look at
    /// (`cargo test -p designcraft-ui-egui --lib -- --ignored render_the_flow_review`).
    #[test]
    #[ignore]
    fn render_the_flow_review() {
        let (path, mut app, sid) = linked("report.txt", "Revenue grew in every region. Costs were flat. Staff numbers held.");
        let edit = |app: &mut DesignApp, from: &str, to: &str| {
            app.session
                .edit(|d, _| {
                    let st = d.story_mut(designcraft_doc::StoryId(sid)).unwrap();
                    let at = st.text.find(from).unwrap();
                    st.replace(at..at + from.len(), to);
                    Ok(())
                })
                .unwrap();
        };
        edit(&mut app, "Costs were flat.", "Costs were held steady.");
        edit(&mut app, "Staff numbers held.", "Staff numbers held firm.");
        std::fs::write(&path, "Revenue grew in every region except the north. Costs fell by two percent. Staff numbers held. Orders rose.").unwrap();
        app.run("window.brightness", json!({"brightness": "mediumDark"})).unwrap();
        crate::panels::properties::update_links(&mut app, json!({"story": sid}));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).wgpu().build_ui_state(
            |ui, app: &mut DesignApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(6);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan/shots");
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(dir.join("f2.12-review.png")).unwrap();
        h.get_by_label("Update").click();
        h.run_steps(6);
        h.render().unwrap().save(dir.join("f2.12-last-update.png")).unwrap();
    }

    /// Clicking a line goes to it: while reviewing, the text as it is now is selected.
    #[test]
    fn clicking_a_line_selects_its_text() {
        let (path, mut app, sid) = linked("goto.txt", "One. Two.");
        std::fs::write(&path, "One. Three.").unwrap();
        crate::panels::properties::update_links(&mut app, json!({"story": sid}));
        let mut h = harness(app);
        h.run_steps(3);
        h.get_by_label_contains("Three.").click();
        h.run_steps(2);
        let sel = h.state().session.active().unwrap().selection.text.unwrap();
        assert_eq!((sel.story.0, sel.anchor.min(sel.focus), sel.anchor.max(sel.focus)), (sid, 5, 9), "Two. is selected");
    }

    /// Hovering a line outlines its place: its text as it is now while reviewing, after the
    /// update in the record, or its cell.
    #[test]
    fn a_hovered_line_outlines_its_place() {
        let e = json!({"at": {"start": 10, "end": 20}, "was": {"start": 3, "end": 7}});
        assert_eq!(super::hover(5, super::Kind::Change, &e, true, None).map(|h| h.range), Some(3..7));
        assert_eq!(super::hover(5, super::Kind::Change, &e, false, None).map(|h| h.range), Some(10..20));
        let cell = json!({"cell": {"row": 1, "col": 2}, "story": 5});
        assert_eq!(super::hover(5, super::Kind::Error, &cell, true, Some(9)).and_then(|h| h.cell), Some((9, 1, 2)));
    }

    /// After an update the record offers its highlights: switched off and on for the page, and
    /// cleared.
    #[test]
    fn the_record_switches_and_clears_its_highlights() {
        let (path, mut app, sid) = linked("highlights.txt", "One.");
        std::fs::write(&path, "One. Two.").unwrap();
        crate::panels::properties::update_links(&mut app, json!({"story": sid}));
        let mut h = harness(app);
        h.run_steps(3);
        h.get_by_label("Update").click();
        h.run_steps(3);
        assert_eq!(super::flow_marks(h.state(), sid), (true, false));
        h.get_by_label("Highlight on the page").click();
        h.run_steps(2);
        assert!(!h.state().session.prefs.flow_highlights);
        h.get_by_label("Highlight on the page").click();
        h.run_steps(2);
        assert!(h.state().session.prefs.flow_highlights);
        h.get_by_label("Clear highlights").click();
        h.run_steps(2);
        assert_eq!(super::flow_marks(h.state(), sid), (false, false));
    }

    /// Place with Flow shows what a file shows as errors and a preview before it lands, waits for
    /// "Place with N errors" when it shows errors, and places it linked.
    #[test]
    fn place_with_flow_checks_the_file_and_places_it_linked() {
        let dir = std::env::temp_dir().join(format!("dc-flow-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("letter.txt");
        std::fs::write(&path, "Dear readers.\nSee Error! Reference source not found. below.").unwrap();
        let mut app = DesignApp::new(designcraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", &json!({})).unwrap();
        let picked = path.to_string_lossy().to_string();
        app.services.pick_open = Some(Box::new(move |_| Some(picked.clone())));
        app.run("app.placeWithFlow", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.id.as_str(), d.fields["blocking"].as_u64()), ("placeWithFlow", Some(1)));
        assert_eq!(d.fields["preview"]["paragraphs"][0], "Dear readers.");
        // Not without the box: nothing is placed and the dialog stays.
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
        assert_eq!(app.session.doc().unwrap().doc.stories.len(), 0);
        app.ui.dialog.as_mut().unwrap().fields.insert("allow".into(), json!(true));
        crate::dialogs::confirm(&mut app).unwrap();
        let rows = app.session.execute("links.list", &json!({})).unwrap();
        let linked: Vec<_> = rows.as_array().unwrap().iter().filter(|r| r.get("story").is_some()).collect();
        assert_eq!(linked.len(), 1, "placed linked: {rows}");
        assert_eq!(linked[0]["merges"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Not a check: renders Place with Flow for a workbook that shows an error, into `plan/shots/`
    /// (`cargo test -p designcraft-ui-egui --lib -- --ignored render_place_with_flow`).
    #[test]
    #[ignore]
    fn render_place_with_flow() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("dc-flow-render-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("q3-figures.xlsx");
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("xl/worksheets/sheet1.xml", zip::write::SimpleFileOptions::default()).unwrap();
        let cell = |r: usize, c: &str, v: &str| format!(r#"<c r="{c}{r}" t="inlineStr"><is><t>{v}</t></is></c>"#);
        let mut rows = String::new();
        for (r, (item, cost, margin)) in
            [("Item", "Cost", "Margin"), ("Paper", "3,200", "22%"), ("Ink", "1,850", "20%"), ("Labels", "960", "19%")].iter().enumerate()
        {
            rows.push_str(&format!(r#"<row r="{}">{}{}{}</row>"#, r + 1, cell(r + 1, "A", item), cell(r + 1, "B", cost), cell(r + 1, "C", margin)));
        }
        rows.push_str(r#"<row r="5"><c r="A5" t="inlineStr"><is><t>Total</t></is></c><c r="B5"><f>[2]Freight!C9</f><v>6010</v></c><c r="C5" t="e"><f>VLOOKUP(A5,Rates!A:B,2,0)</f><v>#N/A</v></c></row>"#);
        w.write_all(format!("<worksheet><sheetData>{rows}</sheetData></worksheet>").as_bytes()).unwrap();
        std::fs::write(&path, w.finish().unwrap().into_inner()).unwrap();
        let mut app = DesignApp::new(designcraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", &json!({})).unwrap();
        app.run("window.brightness", json!({"brightness": "mediumDark"})).unwrap();
        let picked = path.to_string_lossy().to_string();
        app.services.pick_open = Some(Box::new(move |_| Some(picked.clone())));
        app.run("app.placeWithFlow", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).wgpu().build_ui_state(
            |ui, app: &mut DesignApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(6);
        let shots = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan/shots");
        std::fs::create_dir_all(&shots).unwrap();
        h.render().unwrap().save(shots.join("f2.13-place.png")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A link that replaces its story's text (placed before Flow could merge) says so.
    #[test]
    fn a_replacing_link_says_so() {
        let (path, mut app, sid) = linked("replace.txt", "One.");
        app.session
            .edit(|d, _| {
                if let Some(l) = d.story_mut(designcraft_doc::StoryId(sid)).and_then(|st| st.file_link.as_mut()) {
                    l.base = None;
                }
                Ok(())
            })
            .unwrap();
        app.session.execute("story.setText", &json!({"story": sid, "text": "Mine."})).unwrap();
        std::fs::write(&path, "Theirs.").unwrap();
        crate::panels::properties::update_links(&mut app, json!({"story": sid}));
        let mut h = harness(app);
        h.run_steps(3);
        h.get_by_label_contains("replace this story");
        h.get_by_label("Update").click();
        h.run_steps(3);
        assert_eq!(text(h.state(), sid), "Theirs.");
    }
}
