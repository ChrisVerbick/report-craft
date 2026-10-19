//! Object Styles, Table Styles and Cell Styles panels: each family's styles. A click applies one
//! to the selection (Alt-click clears its overrides too); the context menu applies, clears,
//! redefines, breaks the link and deletes; ☰ loads styles from another file. The selection's style and whether it differs from it
//! come from `style.current`, as Redefine Style sees it.

use egui::{Sense, vec2};
use serde_json::json;

use crate::theme::Tokens;
use crate::{DesignApp, icons};

/// A family of styles these panels show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    Object,
    Table,
    Cell,
}

impl Family {
    /// Its name in commands (`style.<key>.apply`, `style.breakLink {kind}`, `style.current`).
    fn key(self) -> &'static str {
        match self {
            Family::Object => "object",
            Family::Table => "table",
            Family::Cell => "cell",
        }
    }

    /// The entry that stands for no style of its own.
    fn none(self) -> &'static str {
        match self {
            Family::Object => designcraft_doc::NO_OBJECT_STYLE,
            Family::Table => designcraft_doc::BASIC_TABLE,
            Family::Cell => designcraft_doc::NO_CELL_STYLE,
        }
    }

    fn names(self, styles: &designcraft_doc::Styles) -> Vec<String> {
        match self {
            Family::Object => styles.object.iter().map(|s| s.name.clone()).collect(),
            Family::Table => styles.table.iter().map(|s| s.name.clone()).collect(),
            Family::Cell => styles.cell.iter().map(|s| s.name.clone()).collect(),
        }
    }

    /// The caption when the selection has nothing of this family.
    fn nothing(self) -> &'static str {
        match self {
            Family::Object => "Nothing selected",
            Family::Table | Family::Cell => "No table selected",
        }
    }

    fn delete_label(self) -> &'static str {
        match self {
            Family::Object => "Delete Object Style",
            Family::Table => "Delete Table Style",
            Family::Cell => "Delete Cell Style",
        }
    }
}

pub fn object(app: &mut DesignApp, ui: &mut egui::Ui) {
    list(app, ui, Family::Object);
}

pub fn table(app: &mut DesignApp, ui: &mut egui::Ui) {
    list(app, ui, Family::Table);
}

pub fn cell(app: &mut DesignApp, ui: &mut egui::Ui) {
    list(app, ui, Family::Cell);
}

fn list(app: &mut DesignApp, ui: &mut egui::Ui, f: Family) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else { return };
    let names = f.names(&st.doc.styles);
    let lang = app.ui.language.clone();
    let current = app.session.execute("style.current", &json!({})).ok().map(|c| c[f.key()].clone()).filter(|c| c.is_object());
    let (cur, overridden) = match &current {
        Some(c) => (c["name"].as_str().map(str::to_string), c["overrides"] == true),
        None => (None, false),
    };
    crate::rtl::label(
        ui,
        egui::RichText::new(match &cur {
            Some(c) => format!("{}{}", crate::i18n::style_name(&lang, c), if overridden { "+" } else { "" }),
            None => crate::i18n::tr(&lang, f.nothing()).into(),
        })
        .size(11.0)
        .color(t.text_dim),
    );
    for n in &names {
        row(app, ui, f, n, cur.as_deref(), overridden, &t);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(crate::rtl::widget(
            ui,
            egui::RichText::new(crate::i18n::tr(
                &lang,
                if cfg!(target_os = "macos") { "⌥-click clears overrides" } else { "Alt-click clears overrides" },
            ))
            .size(10.5)
            .color(t.text_disabled),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, "plus", 20.0, false, crate::i18n::tr(&lang, "Create New Style from Selection")).clicked() {
                let _ = match f {
                    Family::Object => app.run("style.object.create", json!({"name": "Object Style 1", "fromSelection": true})),
                    Family::Table => app.run("style.table.create", json!({"name": "Table Style 1"})),
                    Family::Cell => app.run("style.cell.create", json!({"name": "Cell Style 1", "fromSelection": true})),
                };
            }
            crate::menus::menu_button(ui, "☰", |ui| {
                let (label, cmd) = match f {
                    Family::Object => ("Load Object Styles…", "app.loadObjectStyles"),
                    Family::Table | Family::Cell => ("Load Table and Cell Styles…", "app.loadTableAndCellStyles"),
                };
                if ui.button(crate::rtl::widget(ui, crate::i18n::tr(&lang, label))).clicked() {
                    let _ = app.run(cmd, json!({}));
                    ui.close();
                }
            });
        });
    });
}

/// Apply style `n` of `f` to the selection; `clear`: its overrides go too.
fn apply(app: &mut DesignApp, f: Family, n: &str, clear: bool) {
    match f {
        Family::Object => {
            if app.run("style.object.apply", json!({"name": n})).is_ok() && clear {
                let _ = app.run("style.object.clearOverrides", json!({}));
            }
        }
        Family::Table | Family::Cell => {
            let _ = app.run(&format!("style.{}.apply", f.key()), json!({"name": n, "clearOverrides": clear}));
        }
    }
}

/// One style: click applies (⌥ clears overrides); right-click: apply, clear overrides, redefine,
/// break link, delete.
fn row(app: &mut DesignApp, ui: &mut egui::Ui, f: Family, n: &str, current: Option<&str>, overridden: bool, t: &Tokens) {
    let lang = app.ui.language.clone();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let is_current = current == Some(n);
    if is_current {
        ui.painter().rect_filled(rect, 0.0, t.row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, t.hover);
    }
    let label = crate::i18n::style_name(&lang, n);
    let row_label = format!("{label}{}", if is_current && overridden { "+" } else { "" });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, is_current, &row_label));
    crate::rtl::paint(ui.painter(), rect.min + vec2(8.0, 11.0), egui::Align2::LEFT_CENTER, &row_label, egui::FontId::proportional(12.5), t.text);
    if resp.clicked() {
        let clear = ui.input(|i| i.modifiers.alt);
        apply(app, f, n, clear);
    }
    resp.context_menu(|ui| {
        if ui.button(crate::rtl::widget(ui, format!("{} \"{label}\"", crate::i18n::tr(&lang, "Apply")))).clicked() {
            apply(app, f, n, false);
            ui.close();
        }
        if ui.button(crate::rtl::widget(ui, crate::i18n::tr(&lang, "Apply \"{name}\", Clear Overrides").replace("{name}", label))).clicked() {
            apply(app, f, n, true);
            ui.close();
        }
        // [None] has nothing to redefine; [Basic Table] is the default table style, and can be.
        if is_current
            && (f == Family::Table || n != f.none())
            && ui.button(crate::rtl::widget(ui, crate::i18n::tr(&lang, "Redefine Style"))).clicked()
        {
            let _ = app.run(&format!("style.{}.redefine", f.key()), json!({"name": n}));
            ui.close();
        }
        if is_current && n != f.none() && ui.button(crate::rtl::widget(ui, crate::i18n::tr(&lang, "Break Link to Style"))).clicked() {
            let _ = app.run("style.breakLink", json!({"kind": f.key()}));
            ui.close();
        }
        if !n.starts_with('[') {
            ui.separator();
            if ui.button(crate::rtl::widget(ui, crate::i18n::tr(&lang, f.delete_label()))).clicked() {
                let _ = app.run(&format!("style.{}.delete", f.key()), json!({"name": n}));
                ui.close();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use designcraft_doc::{ItemId, StoryId};
    use egui_kittest::{Harness, kittest::Queryable};
    use serde_json::json;

    use super::*;

    fn harness(app: DesignApp, family: Family) -> Harness<'static, DesignApp> {
        Harness::builder().with_size(egui::vec2(320.0, 420.0)).build_ui_state(
            move |ui, app: &mut DesignApp| {
                // The semibold face is drawn from the frame after the fonts are installed.
                let key = egui::Id::new("test_fonts");
                if ui.data(|d| d.get_temp::<bool>(key)).is_none() {
                    crate::theme::install_fonts(ui.ctx(), "");
                    ui.data_mut(|d| d.insert_temp(key, true));
                    return;
                }
                match family {
                    Family::Object => object(app, ui),
                    Family::Table => table(app, ui),
                    Family::Cell => cell(app, ui),
                }
            },
            app,
        )
    }

    fn new_app() -> DesignApp {
        let mut app = DesignApp::new(designcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        app
    }

    /// Each panel's ☰ menu loads its family's styles from another file: Object Styles its object
    /// styles, Table and Cell Styles both.
    #[test]
    fn the_panel_menus_load_styles_from_another_file() {
        for (family, item, purpose) in [
            (Family::Object, "Load Object Styles…", "styles:object"),
            (Family::Table, "Load Table and Cell Styles…", "styles:tableCell"),
            (Family::Cell, "Load Table and Cell Styles…", "styles:tableCell"),
        ] {
            let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let seen = asked.clone();
            let services = crate::Services {
                inbox: Some(crate::Inbox::default()),
                open_async: Some(Box::new(move |request| seen.lock().unwrap().push(request))),
                ..Default::default()
            };
            let mut app = DesignApp::new(designcraft_engine::Session::new(), services);
            app.run("file.new", json!({})).unwrap();
            let mut h = harness(app, family);
            h.run_steps(3);
            h.get_by_label("☰").click();
            h.run_steps(2);
            h.get_by_label(item).click();
            h.run_steps(2);
            assert_eq!(asked.lock().unwrap().pop().map(|r| r.purpose), Some(purpose.to_string()), "{item}");
        }
    }

    /// Right-click the last element labelled `label` and click `item` in its menu.
    fn context(h: &mut Harness<'static, DesignApp>, label: &str, item: &str) {
        let at = h.get_all_by_label(label).next_back().unwrap().rect().center();
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.run_steps(2);
        }
        h.get_by_label(item).click();
        h.run_steps(3);
    }

    fn alt_click(h: &mut Harness<'static, DesignApp>, label: &str) {
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
        h.get_all_by_label(label).next_back().unwrap().click();
        h.run_steps(3);
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    }

    /// Object Styles: every style, the built-ins too; a click styles the selected object; once it
    /// differs from its style the row says so with +, and Alt-click clears that.
    #[test]
    fn the_object_styles_panel_applies_and_clears_overrides() {
        let mut app = new_app();
        app.run("style.object.create", json!({"name": "Callout", "fromSelection": false, "fill": "[Black]"})).unwrap();
        let id = ItemId(app.run("frame.create", json!({"rect": [36, 36, 200, 200]})).unwrap()["id"].as_u64().unwrap());
        app.run("selection.set", json!({"ids": [id.0]})).unwrap();
        let mut h = harness(app, Family::Object);
        h.run_steps(3);
        for n in ["[None]", "[Basic Text Frame]", "Callout"] {
            h.get_by_label(n);
        }
        h.get_by_label("Callout").click();
        h.run_steps(3);
        let item = |h: &Harness<'static, DesignApp>| h.state().session.doc().unwrap().doc.item(id).unwrap().clone();
        assert_eq!(item(&h).object_style, "Callout");
        h.state_mut().run("object.fill", json!({"ids": [id.0], "swatch": "[Paper]"})).unwrap();
        h.run_steps(3);
        assert!(h.get_all_by_label("Callout+").next().is_some(), "the override shows");
        alt_click(&mut h, "Callout+");
        assert_eq!(item(&h).fill, designcraft_doc::Fill::swatch("[Black]"), "Alt-click cleared it");
        // Break Link to Style: the look stays, the style goes.
        context(&mut h, "Callout", "Break Link to Style");
        assert_eq!(
            (item(&h).object_style.as_str(), item(&h).fill.clone()),
            (designcraft_doc::NO_OBJECT_STYLE, designcraft_doc::Fill::swatch("[Black]"))
        );
    }

    /// A 3×2 table (one header row) with table style "Data" (body cells "Body", Paper fill), the
    /// caret in it, and cell (1, 0) selected.
    fn with_table() -> (DesignApp, u64, u64) {
        let mut app = new_app();
        let r = app.run("frame.create", json!({"rect": [72, 72, 500, 500], "content": "text", "text": ""})).unwrap();
        let sid = r["story"].as_u64().unwrap();
        app.run("text.select", json!({"story": sid, "anchor": 0, "focus": 0})).unwrap();
        app.run("table.insert", json!({"rows": 3, "cols": 2, "headerRows": 1})).unwrap();
        app.run("style.cell.create", json!({"name": "Body", "fill": "[Paper]"})).unwrap();
        app.run("style.cell.create", json!({"name": "Total", "fill": "[Black]"})).unwrap();
        app.run("style.table.create", json!({"name": "Data", "body": "Body", "spaceBefore": 6})).unwrap();
        let tid = *app.session.doc().unwrap().doc.story(StoryId(sid)).unwrap().tables.keys().next().unwrap();
        app.run("table.select", json!({"story": sid, "table": tid, "rows": [1, 1], "cols": [0, 0]})).unwrap();
        (app, sid, tid)
    }

    fn table_of(h: &Harness<'static, DesignApp>, sid: u64) -> designcraft_doc::Table {
        (**h.state().session.doc().unwrap().doc.story(StoryId(sid)).unwrap().tables.values().next().unwrap()).clone()
    }

    /// Table Styles styles the table; Cell Styles styles only the selected cell, and Alt-click
    /// clears its overrides.
    #[test]
    fn the_table_and_cell_styles_panels_apply_to_the_selection() {
        let (app, sid, _) = with_table();
        let mut h = harness(app, Family::Table);
        h.run_steps(3);
        // The caption names the current style too.
        assert_eq!(h.get_all_by_label("[Basic Table]").count(), 2);
        h.get_by_label("Data").click();
        h.run_steps(3);
        assert_eq!(table_of(&h, sid).style, "Data");
        let mut h = harness(h.into_state(), Family::Cell);
        h.run_steps(3);
        h.get_all_by_label("Total").next_back().unwrap().click();
        h.run_steps(3);
        let t = table_of(&h, sid);
        assert_eq!((t.cell(1, 0).unwrap().style.as_str(), t.cell(2, 0).unwrap().style.as_str()), ("Total", "Body"), "only the selected cell");
        // Its own vertical justification (Total doesn't set it) and fill: a click keeps the first,
        // Alt-click clears both.
        h.state_mut().run("table.setCell", json!({"vj": "center"})).unwrap();
        h.state_mut().run("table.setCell", json!({"fill": "[Paper]"})).unwrap();
        h.run_steps(3);
        h.get_all_by_label("Total+").next_back().unwrap().click();
        h.run_steps(3);
        assert_eq!(table_of(&h, sid).cell(1, 0).unwrap().vj, designcraft_doc::VerticalJustification::Center, "a click keeps it");
        h.state_mut().run("table.setCell", json!({"fill": "[Paper]"})).unwrap();
        h.run_steps(3);
        alt_click(&mut h, "Total+");
        let c = table_of(&h, sid).cell(1, 0).unwrap().clone();
        assert_eq!((c.fill.as_str(), c.vj), ("[Black]", designcraft_doc::VerticalJustification::default()), "Alt-click cleared both");
    }

    /// Redefine Style from the context menu gives the cell style the cell's override.
    #[test]
    fn redefine_style_from_the_context_menu() {
        let (app, _, _) = with_table();
        let mut h = harness(app, Family::Cell);
        h.run_steps(3);
        h.get_all_by_label("Total").next_back().unwrap().click();
        h.run_steps(3);
        h.state_mut().run("table.setCell", json!({"fill": "[Registration]"})).unwrap();
        h.run_steps(3);
        context(&mut h, "Total+", "Redefine Style");
        let total = h.state().session.doc().unwrap().doc.styles.cell.iter().find(|c| c.name == "Total").unwrap().fill.clone();
        assert_eq!(total.as_deref(), Some("[Registration]"));
    }

    /// The Table panel's cell style list styles the selected cells, not the whole table.
    #[test]
    fn the_table_panels_cell_style_applies_to_the_selected_cells() {
        let (mut app, sid, _) = with_table();
        let info = app.session.execute("table.get", &json!({})).unwrap();
        let p = crate::panels::table::style_params("cell", "Total", &info);
        app.run("style.cell.apply", p).unwrap();
        let t = (**app.session.doc().unwrap().doc.story(StoryId(sid)).unwrap().tables.values().next().unwrap()).clone();
        assert_eq!((t.cell(1, 0).unwrap().style.as_str(), t.cell(2, 0).unwrap().style.as_str()), ("Total", ""));
    }

    /// F6.4's screenshots (`plan/shots/f6.4-{object,table,cell}.png`): each panel beside a document
    /// with a styled frame and a styled table.
    #[test]
    #[ignore]
    fn render_style_panels() {
        let shots = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan/shots");
        std::fs::create_dir_all(&shots).unwrap();
        let (mut app, sid, tid) = with_table();
        // The table style while a cell is selected (it applies to the selection's table).
        app.run("style.table.apply", json!({"name": "Data"})).unwrap();
        app.run("style.object.create", json!({"name": "Callout", "fromSelection": false, "fill": "[Black]"})).unwrap();
        app.run("style.object.create", json!({"name": "Sidebar", "fromSelection": false, "fill": "[Paper]"})).unwrap();
        let id = app.run("frame.create", json!({"rect": [300, 560, 500, 700]})).unwrap()["id"].as_u64().unwrap();
        app.run("style.object.apply", json!({"name": "Callout", "ids": [id]})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).wgpu().build_ui_state(
            |ui, app: &mut DesignApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(8);
        for (panel, file, select) in [
            ("objectStyles", "f6.4-object.png", json!({"ids": [id]})),
            ("tableStyles", "f6.4-table.png", json!(null)),
            ("cellStyles", "f6.4-cell.png", json!(null)),
        ] {
            if select.is_null() {
                h.state_mut().run("table.select", json!({"story": sid, "table": tid, "rows": [1, 1], "cols": [0, 0]})).unwrap();
            } else {
                h.state_mut().run("selection.set", select).unwrap();
            }
            h.state_mut().run("window.panel", json!({"panel": panel})).unwrap();
            h.run_steps(6);
            h.render().unwrap().save(shots.join(file)).unwrap();
            h.state_mut().run("window.panel", json!({"panel": panel})).unwrap();
            h.run_steps(3);
        }
    }
}
