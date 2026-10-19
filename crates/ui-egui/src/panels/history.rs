//! History panel (Stage 5): the document's versions, newest first. Pick one to restore it,
//! compare it with the document (a redline) or export it as a file of its own.

use egui::RichText;
use serde_json::{Value, json};

use crate::DesignApp;
use crate::theme::{Tokens, semibold};

/// What the buttons under the list do with the version picked.
enum Act {
    Restore,
    Compare,
    Export,
}

pub fn show(app: &mut DesignApp, ui: &mut egui::Ui) {
    if app.session.active().is_none() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language.clone();
    let list = app.session.execute("version.list", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let sel_id = egui::Id::new("history_panel_sel");
    // A version no longer listed (another document's, or pruned) isn't picked.
    let mut sel = ui.data(|d| d.get_temp::<Option<u64>>(sel_id)).flatten().filter(|s| list.iter().any(|v| v["id"].as_u64() == Some(*s)));
    let mut act = None;
    // Whether this document's saved file keeps them: its own setting, else the preference.
    let keeps = app.session.active().is_some_and(|st| st.doc.settings.keep_versions.unwrap_or(app.session.prefs.keep_versions));
    let mut on = keeps;
    let hint = crate::i18n::tr(&lang, "Anyone you send the file to can restore them. File › Save a Copy without History… leaves them out.");
    if ui.checkbox(&mut on, crate::rtl::widget(ui, crate::i18n::tr(&lang, "Keep Earlier Versions in This File"))).on_hover_text(hint).changed() {
        let _ = app.run("version.keep", json!({"on": on}));
    }
    if !keeps {
        crate::rtl::label(
            ui,
            RichText::new(crate::i18n::tr(&lang, "Off: saving drops this file's versions, and copies carry none.")).size(11.0).color(t.text_dim),
        );
    }
    if list.is_empty() {
        crate::rtl::label(
            ui,
            RichText::new(crate::i18n::tr(&lang, "No versions yet. Saving a changed document keeps one.")).size(11.0).color(t.text_dim),
        );
    } else {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for v in list.iter().rev() {
                let Some(id) = v["id"].as_u64() else { continue };
                let name = title(&lang, v);
                let text = if v["kind"] == "manual" { RichText::new(name).font(semibold(12.0)) } else { RichText::new(name).size(12.0) };
                ui.horizontal(|ui| {
                    if ui.selectable_label(sel == Some(id), crate::rtl::widget(ui, text)).clicked() {
                        sel = Some(id);
                    }
                    // Who and when, at the right: the exact time (UTC: the app has no time zone) on hover.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ts = v["ts"].as_i64().unwrap_or(0);
                        let when = crate::rtl::label(ui, RichText::new(crate::panels::flow::ago(ts, &lang)).size(11.0).color(t.text_dim));
                        if ts > 0 {
                            when.on_hover_text(format!("{} UTC", designcraft_doc::vars::format_date(ts, "yyyy-MM-dd HH:mm")));
                        }
                        if let Some(author) = v["author"].as_str().filter(|a| !a.is_empty()) {
                            crate::rtl::label(ui, RichText::new(author).size(11.0).color(t.text_dim));
                        }
                    });
                });
            }
        });
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            for (a, label) in [(Act::Restore, "Restore"), (Act::Compare, "Compare with Current"), (Act::Export, "Export…")] {
                if ui.add_enabled(sel.is_some(), egui::Button::new(crate::rtl::widget(ui, crate::i18n::tr(&lang, label)))).clicked() {
                    act = Some(a);
                }
            }
        });
    }
    ui.horizontal(|ui| {
        let n = list.len();
        if n > 0 {
            let count = if n == 1 {
                crate::i18n::tr(&lang, "1 version").to_string()
            } else {
                crate::i18n::tr(&lang, "{n} versions").replace("{n}", &n.to_string())
            };
            crate::rtl::label(ui, RichText::new(count).size(11.0).color(t.text_dim));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(crate::rtl::widget(ui, crate::i18n::tr(&lang, "Save a Version…"))).clicked() {
                let _ = app.run("app.saveVersionDialog", json!({}));
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(sel_id, sel));
    if let (Some(a), Some(id)) = (act, sel) {
        match a {
            Act::Restore => restore(app, id),
            Act::Compare => compare(app, id),
            Act::Export => export(app, id),
        }
    }
}

/// What a version is called: its label, or what kept it. The app labels some saves itself ("As
/// opened", "Before Flow update") and restores ("Restored version 7").
fn title(lang: &str, v: &Value) -> String {
    let label = v["label"].as_str().unwrap_or_default();
    match v["kind"].as_str().unwrap_or_default() {
        "restore" => {
            let from = label.rsplit(' ').next().and_then(|n| n.parse::<u64>().ok()).or(v["parent"].as_u64()).unwrap_or(0);
            crate::i18n::tr(lang, "Restored version {id}").replace("{id}", &from.to_string())
        }
        "linkUpdate" => crate::i18n::tr(lang, "Flow update").to_string(),
        "save" if label.is_empty() => crate::i18n::tr(lang, "Saved").to_string(),
        "save" => crate::i18n::tr(lang, label).to_string(),
        _ if label.is_empty() => crate::i18n::tr(lang, "Version").to_string(),
        _ => label.to_string(),
    }
}

fn restore(app: &mut DesignApp, id: u64) {
    if app.run("version.restore", json!({"id": id})).is_ok() {
        let s = crate::i18n::tr(&app.ui.language, "Restored version {id}").replace("{id}", &id.to_string());
        app.status(s);
    }
}

fn compare(app: &mut DesignApp, id: u64) {
    if let Ok(r) = app.run("compare.versions", json!({"a": id})) {
        let s = crate::menus::redline_status(&app.ui.language, &r);
        app.status(s);
    }
}

fn export(app: &mut DesignApp, id: u64) {
    let name = app.session.active().map(|d| format!("{} (version {id}).designcraft", d.title())).unwrap_or_default();
    let Some(path) = app.services.pick_save.as_mut().and_then(|f| f(&name)) else { return };
    if app.run("version.export", json!({"id": id, "path": path})).is_ok() {
        let s = crate::i18n::tr(&app.ui.language, "Exported version {id}").replace("{id}", &id.to_string());
        app.status(s);
    }
}

#[cfg(test)]
mod tests {
    use designcraft_doc::StoryId;
    use egui_kittest::{Harness, kittest::Queryable};
    use serde_json::{Value, json};

    use super::*;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dc-history-panel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A new document with one story saying `text`.
    fn app_with(text: &str) -> (DesignApp, StoryId) {
        let mut app = DesignApp::new(designcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        let r = app.run("frame.create", json!({"rect": [72, 72, 400, 300], "content": "text", "text": text})).unwrap();
        (app, StoryId(r["story"].as_u64().unwrap()))
    }

    fn write(app: &mut DesignApp, sid: StoryId, text: &str) {
        app.session
            .edit(|d, _| {
                d.set_story_text(sid, text).unwrap();
                Ok(())
            })
            .unwrap();
    }

    /// Saved as two labelled versions: "First" says "One.", "Second" says "Two.".
    fn app_with_versions(dir: &std::path::Path) -> (DesignApp, StoryId) {
        let (mut app, sid) = app_with("One.");
        app.run("version.commit", json!({"label": "First", "path": dir.join("doc.designcraft")})).unwrap();
        write(&mut app, sid, "Two.");
        app.run("version.commit", json!({"label": "Second"})).unwrap();
        (app, sid)
    }

    /// The panel, and any dialog it opens.
    fn harness(app: DesignApp) -> Harness<'static, DesignApp> {
        Harness::builder().with_size(egui::vec2(320.0, 480.0)).build_ui_state(
            |ui, app: &mut DesignApp| {
                // The semibold face is drawn from the frame after the fonts are installed.
                let key = egui::Id::new("test_fonts");
                if ui.data(|d| d.get_temp::<bool>(key)).is_none() {
                    crate::theme::install_fonts(ui.ctx(), "");
                    ui.data_mut(|d| d.insert_temp(key, true));
                    return;
                }
                show(app, ui);
                crate::dialogs::show(app, &ui.ctx().clone());
            },
            app,
        )
    }

    fn text(app: &DesignApp, sid: StoryId) -> String {
        app.session.active().unwrap().doc.story(sid).unwrap().text.clone()
    }

    fn versions(app: &mut DesignApp) -> Vec<Value> {
        app.session.execute("version.list", &json!({})).unwrap().as_array().unwrap().clone()
    }

    /// How many versions the file at `path` keeps.
    fn versions_in(path: &std::path::Path) -> usize {
        let mut s = designcraft_engine::Session::new();
        s.execute("file.open", &json!({"path": path})).unwrap();
        s.execute("version.list", &json!({})).unwrap().as_array().unwrap().len()
    }

    /// Picking a version and Restore make the document that version again, keep a version
    /// saying so (listed first) and say so in the status line.
    #[test]
    fn restoring_a_version_from_the_panel() {
        let dir = folder("restore");
        let (app, sid) = app_with_versions(&dir);
        let mut h = harness(app);
        h.run_steps(3);
        h.get_by_label("First").click();
        h.run_steps(2);
        h.get_by_label("Restore").click();
        h.run_steps(3);
        assert_eq!(text(h.state(), sid), "One.");
        let v = versions(h.state_mut());
        assert_eq!((v.len(), v[2]["kind"].as_str()), (3, Some("restore")));
        assert!(h.state().ui.status.contains("Restored version 1"), "{}", h.state().ui.status);
        h.get_by_label("Restored version 1");
        // Undoable.
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(text(h.state(), sid), "Two.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Export… writes the version picked as a file of its own, without history; Compare with
    /// Current opens its redline against the document.
    #[test]
    fn exporting_and_comparing_a_version_from_the_panel() {
        let dir = folder("compare");
        let (mut app, sid) = app_with_versions(&dir);
        let out = dir.join("first.designcraft");
        let to = out.to_string_lossy().to_string();
        app.services.pick_save = Some(Box::new(move |_| Some(to.clone())));
        let mut h = harness(app);
        h.run_steps(3);
        h.get_by_label("First").click();
        h.run_steps(2);
        h.get_by_label("Export…").click();
        h.run_steps(3);
        assert_eq!(versions_in(&out), 0);
        let mut s = designcraft_engine::Session::new();
        s.execute("file.open", &json!({"path": out})).unwrap();
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "One.");
        h.get_by_label("Compare with Current").click();
        h.run_steps(3);
        let title = h.state().session.active().unwrap().doc.title.clone();
        assert!(title.contains("compared with version 1"), "{title}");
        assert!(h.state().ui.status.contains("change"), "{}", h.state().ui.status);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Save a Version… (the panel's button, as File › Save a Version…) asks for a label, then
    /// keeps a manual version as it saves; a document never saved picks its file first. File ›
    /// Save a Copy without History… writes a copy with none of them.
    #[test]
    fn save_a_version_and_a_copy_without_history() {
        let dir = folder("menu");
        let (mut app, _) = app_with("One.");
        let path = dir.join("doc.designcraft");
        let to = path.to_string_lossy().to_string();
        app.services.pick_save = Some(Box::new(move |_| Some(to.clone())));
        let mut h = harness(app);
        h.run_steps(3);
        h.get_by_label("No versions yet. Saving a changed document keeps one.");
        h.get_by_label("Save a Version…").click();
        h.run_steps(3);
        h.get_by_label("Label").type_text("Draft for review");
        h.run_steps(2);
        h.get_by_label("  OK  ").click();
        h.run_steps(3);
        let v = versions(h.state_mut());
        assert_eq!(
            v.iter().map(|v| (v["kind"].as_str().unwrap(), v["label"].as_str().unwrap())).collect::<Vec<_>>(),
            [("manual", "Draft for review")]
        );
        assert!(!h.state().session.active().unwrap().is_dirty());
        assert_eq!(versions_in(&path), 1);
        h.get_by_label("Draft for review");
        let copy = dir.join("copy.designcraft");
        let to = copy.to_string_lossy().to_string();
        h.state_mut().services.pick_save = Some(Box::new(move |_| Some(to.clone())));
        h.state_mut().run("app.saveCopyWithoutHistoryDialog", json!({})).unwrap();
        assert_eq!((versions_in(&copy), versions_in(&path)), (0, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The panel's switch is this document's own: it turns its versions off, with a note saying
    /// what that means, and on again; the preference for other documents stays as it was.
    #[test]
    fn the_panel_switch_turns_versions_off_and_on() {
        let dir = folder("switch");
        let (app, _) = app_with_versions(&dir);
        let mut h = harness(app);
        h.run_steps(3);
        let own = |app: &DesignApp| app.session.active().unwrap().doc.settings.keep_versions;
        h.get_by_label("Keep Earlier Versions in This File").click();
        h.run_steps(3);
        assert_eq!(own(h.state()), Some(false));
        assert!(h.state().session.prefs.keep_versions, "the preference, for other documents, stays on");
        h.get_by_label("Off: saving drops this file's versions, and copies carry none.");
        h.get_by_label("Keep Earlier Versions in This File").click();
        h.run_steps(3);
        assert_eq!(own(h.state()), Some(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Preferences › File Handling opens with the switch on and saves it off.
    #[test]
    fn preferences_save_the_switch() {
        let (mut app, _) = app_with("One.");
        app.run("app.preferences", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.fields["keepVersions"], true);
        d.fields.insert("keepVersions".into(), json!(false));
        d.fields.insert("section".into(), json!("files"));
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            |ui, app: &mut DesignApp| {
                let key = egui::Id::new("test_fonts");
                if ui.data(|d| d.get_temp::<bool>(key)).is_none() {
                    crate::theme::install_fonts(ui.ctx(), "");
                    ui.data_mut(|d| d.insert_temp(key, true));
                    return;
                }
                crate::dialogs::show(app, &ui.ctx().clone());
            },
            app,
        );
        h.run_steps(3);
        h.get_by_label("Keep Earlier Versions in Saved Files");
        crate::dialogs::confirm(h.state_mut()).unwrap();
        assert!(!h.state().session.prefs.keep_versions);
    }

    /// F5.3's screenshot (`plan/shots/f5.3-history.png`): the History panel beside a document
    /// with two labelled versions, a save between them and a restore.
    #[test]
    #[ignore]
    fn render_history_panel() {
        let shots = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan/shots");
        std::fs::create_dir_all(&shots).unwrap();
        let dir = folder("shot");
        let (mut app, sid) = app_with("Quarterly report\nRevenue grew in every region. Costs were flat.");
        app.run("prefs.set", json!({"userName": "Chris"})).unwrap();
        app.run("version.commit", json!({"label": "First draft", "path": dir.join("report.designcraft")})).unwrap();
        write(&mut app, sid, "Quarterly report\nRevenue grew in every region except the north. Costs were flat.");
        app.run("file.save", json!({})).unwrap();
        write(&mut app, sid, "Quarterly report\nRevenue grew in every region except the north. Costs fell by two percent.");
        app.run("version.commit", json!({"label": "Sent for review"})).unwrap();
        app.run("version.restore", json!({"id": 1})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).wgpu().build_ui_state(
            |ui, app: &mut DesignApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(8);
        h.state_mut().run("window.panel", json!({"panel": "history"})).unwrap();
        h.run_steps(4);
        h.get_by_label("Sent for review").click();
        h.run_steps(4);
        h.render().unwrap().save(shots.join("f5.3-history.png")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
