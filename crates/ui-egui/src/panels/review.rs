//! Review panel (Stage 10): where the document is published for review, the reviewers'
//! comments, and what to do with each: go to its text, add it as a note, apply a suggestion as
//! a tracked change, resolve it, answer it.

use egui::RichText;
use serde_json::{Value, json};

use crate::DesignApp;
use crate::theme::{Tokens, semibold};

/// What a click asks for, run once the panel is drawn.
enum Act {
    Refresh,
    Publish(String),
    Sync,
    Link,
    Detach,
    GoTo(String),
    Note(String),
    Apply(String),
    Resolve(String, bool),
    Reply(String, String),
}

pub fn show(app: &mut DesignApp, ui: &mut egui::Ui) {
    if app.session.active().is_none() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language.clone();
    let tr = |s: &str| crate::i18n::tr(&lang, s).to_string();
    let mut act = None;
    let (projects_id, project_id, link_id, filter_id, who_id) = (
        egui::Id::new("review_panel_projects"),
        egui::Id::new("review_panel_project"),
        egui::Id::new("review_panel_link"),
        egui::Id::new("review_panel_filter"),
        egui::Id::new("review_panel_who"),
    );

    // The service: the preferences, kept on this computer.
    // The host passes its edition's service in; the hint shows it.
    let edition_server = app.session.review_server_default.clone().unwrap_or_else(|| "https://review.example.com".into());
    let unset =
        (app.session.prefs.review_server.is_empty() && app.session.review_server_default.is_none()) || app.session.prefs.review_token.is_empty();
    egui::CollapsingHeader::new(crate::rtl::widget(ui, tr("Review service"))).default_open(unset).show(ui, |ui| {
        let mut server = app.session.prefs.review_server.clone();
        let mut token = app.session.prefs.review_token.clone();
        crate::rtl::label(ui, RichText::new(tr("Server")).size(11.0).color(t.text_dim));
        if ui.add(egui::TextEdit::singleline(&mut server).hint_text(edition_server.as_str())).changed() {
            let _ = app.run("prefs.set", json!({"reviewServer": server.trim()}));
        }
        crate::rtl::label(ui, RichText::new(tr("Owner token")).size(11.0).color(t.text_dim));
        if ui.add(egui::TextEdit::singleline(&mut token).password(true)).changed() {
            let _ = app.run("prefs.set", json!({"reviewToken": token.trim()}));
        }
        crate::rtl::label(ui, RichText::new(tr("Kept in the preferences file on this computer.")).size(11.0).color(t.text_dim));
    });

    let state = app.session.active().and_then(|st| st.review.clone());
    match &state {
        None => {
            crate::rtl::label(ui, RichText::new(tr("Not published for review yet.")).font(semibold(12.0)));
            let mut projects: Vec<Value> = ui.data(|d| d.get_temp(projects_id)).unwrap_or_default();
            if projects.is_empty() && !unset && ui.data(|d| d.get_temp::<bool>(projects_id.with("tried"))).is_none() {
                ui.data_mut(|d| d.insert_temp(projects_id.with("tried"), true));
                act = Some(Act::Refresh);
            }
            let mut chosen: String = ui.data(|d| d.get_temp(project_id)).unwrap_or_default();
            if !projects.iter().any(|p| p["id"] == chosen.as_str()) {
                chosen = projects.first().and_then(|p| p["id"].as_str()).unwrap_or_default().to_string();
            }
            let name_of = |id: &str| projects.iter().find(|p| p["id"] == id).and_then(|p| p["name"].as_str()).unwrap_or("").to_string();
            ui.horizontal(|ui| {
                crate::rtl::label(ui, tr("Project"));
                egui::ComboBox::from_id_salt("review_panel_project_box").selected_text(name_of(&chosen)).show_ui(ui, |ui| {
                    for p in &projects {
                        let (id, name) = (p["id"].as_str().unwrap_or(""), p["name"].as_str().unwrap_or(""));
                        ui.selectable_value(&mut chosen, id.to_string(), name);
                    }
                });
                if ui.button(crate::rtl::widget(ui, tr("Refresh"))).on_hover_text(tr("The service's projects")).clicked() {
                    act = Some(Act::Refresh);
                }
            });
            ui.data_mut(|d| d.insert_temp(project_id, chosen.clone()));
            ui.horizontal(|ui| {
                if ui.add_enabled(!chosen.is_empty() && !unset, egui::Button::new(crate::rtl::widget(ui, tr("Publish")))).clicked() {
                    act = Some(Act::Publish(chosen.clone()));
                }
                if ui
                    .add_enabled(!unset, egui::Button::new(crate::rtl::widget(ui, tr("Check"))))
                    .on_hover_text(tr("Whether this document is published already"))
                    .clicked()
                {
                    act = Some(Act::Sync);
                }
            });
            projects.clear();
        }
        Some(st) => {
            let project = st.project["name"].as_str().unwrap_or("?");
            let version = st.version.as_ref().and_then(|v| v["label"].as_str()).unwrap_or("");
            crate::rtl::label(ui, RichText::new(format!("{project} · {version}")).font(semibold(12.0)));
            ui.horizontal_wrapped(|ui| {
                if ui.button(crate::rtl::widget(ui, tr("Publish"))).on_hover_text(tr("A new version of this document")).clicked() {
                    act = Some(Act::Publish(String::new()));
                }
                if ui.button(crate::rtl::widget(ui, tr("Sync"))).on_hover_text(tr("Fetch the reviewers' comments")).clicked() {
                    act = Some(Act::Sync);
                }
                if ui.button(crate::rtl::widget(ui, tr("Share link"))).clicked() {
                    act = Some(Act::Link);
                }
                if ui
                    .button(crate::rtl::widget(ui, tr("Detach")))
                    .on_hover_text(tr("Forget the review: a copy for another client starts its own"))
                    .clicked()
                {
                    act = Some(Act::Detach);
                }
            });
            if let Some(url) = ui.data(|d| d.get_temp::<String>(link_id)) {
                ui.horizontal(|ui| {
                    let mut shown = url.clone();
                    ui.add(egui::TextEdit::singleline(&mut shown).desired_width(220.0));
                    if ui.button(crate::rtl::widget(ui, tr("Copy"))).clicked() {
                        ui.ctx().copy_text(url.clone());
                    }
                });
                crate::rtl::label(ui, RichText::new(tr("Shown once: the service keeps only its hash.")).size(11.0).color(t.text_dim));
            }
            ui.separator();
            // Filters.
            let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_else(|| "open".to_string());
            let mut who: String = ui.data(|d| d.get_temp(who_id)).unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                for (f, label) in [("open", "Open"), ("resolved", "Resolved"), ("applied", "Applied"), ("all", "All")] {
                    if ui.selectable_label(filter == f, crate::rtl::widget(ui, tr(label))).clicked() {
                        filter = f.to_string();
                    }
                }
                let mut names: Vec<String> = st.comments.iter().map(|c| c.author.name.clone()).filter(|n| !n.is_empty()).collect();
                names.sort();
                names.dedup();
                egui::ComboBox::from_id_salt("review_panel_who_box")
                    .selected_text(if who.is_empty() { tr("Everyone") } else { who.clone() })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut who, String::new(), tr("Everyone"));
                        for n in names {
                            ui.selectable_value(&mut who, n.clone(), n);
                        }
                    });
            });
            ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
            ui.data_mut(|d| d.insert_temp(who_id, who.clone()));
            // The comments, with where their text is now (cached per document state and sync).
            let list = comments(app, ui, st.synced);
            let shown: Vec<&Value> = list
                .iter()
                .filter(|c| c["inReplyTo"].is_null())
                .filter(|c| match filter.as_str() {
                    "open" => c["status"] == "open",
                    "resolved" => c["status"] == "resolved",
                    "applied" => c["status"] == "applied" || c["status"] == "orphaned",
                    _ => true,
                })
                .filter(|c| who.is_empty() || c["author"]["name"] == who.as_str())
                .collect();
            if shown.is_empty() {
                crate::rtl::label(ui, RichText::new(tr("No comments here. Sync fetches new ones.")).size(11.0).color(t.text_dim));
            }
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for c in shown {
                    let id = c["id"].as_str().unwrap_or("").to_string();
                    ui.push_id(&id, |ui| {
                        comment_row(ui, &t, &tr, c, &list, &mut act);
                    });
                    ui.separator();
                }
            });
        }
    }

    if let Some(a) = act {
        let ctx = ui.ctx().clone();
        run(app, &ctx, a);
    }
}

/// The comments with their anchors, computed again only when the document or the sync changed.
fn comments(app: &mut DesignApp, ui: &egui::Ui, synced: i64) -> Vec<Value> {
    let key = app.session.active().map(|st| (st.uid, st.revision, synced));
    let id = egui::Id::new("review_panel_comments");
    if let Some((k, list)) = ui.data(|d| d.get_temp::<(Option<(u64, u64, i64)>, Vec<Value>)>(id))
        && k == key
    {
        return list;
    }
    let list = app.session.execute("review.comments", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    ui.data_mut(|d| d.insert_temp(id, (key, list.clone())));
    list
}

#[allow(clippy::too_many_arguments)]
fn comment_row(ui: &mut egui::Ui, t: &Tokens, tr: &dyn Fn(&str) -> String, c: &Value, all: &[Value], act: &mut Option<Act>) {
    let id = c["id"].as_str().unwrap_or("").to_string();
    let (kind, status) = (c["kind"].as_str().unwrap_or(""), c["status"].as_str().unwrap_or(""));
    let anchored = c["anchored"]["status"].as_str().unwrap_or("");
    ui.horizontal_wrapped(|ui| {
        crate::rtl::label(ui, RichText::new(c["author"]["name"].as_str().unwrap_or("?")).font(semibold(12.0)));
        crate::rtl::label(ui, RichText::new(tr(kind)).size(10.0).color(t.text_dim));
        crate::rtl::label(
            ui,
            RichText::new(tr("p. {n}").replace("{n}", &(c["page"].as_i64().unwrap_or(0) + 1).to_string())).size(11.0).color(t.text_dim),
        );
        crate::rtl::label(ui, RichText::new(tr(status)).size(11.0).color(t.text_dim));
        if anchored == "moved" {
            crate::rtl::label(ui, RichText::new(tr("text moved")).size(11.0).color(t.text_dim));
        } else if anchored == "orphaned" {
            crate::rtl::label(ui, RichText::new(tr("text not found")).size(11.0).color(t.accent_strong));
        }
    });
    if let Some(q) = c["quote"]["exact"].as_str().filter(|q| !q.is_empty()) {
        crate::rtl::label(ui, RichText::new(format!("“{q}”")).italics().size(12.0).color(t.text_dim));
    }
    if let Some(b) = c["body"].as_str().filter(|b| !b.is_empty()) {
        crate::rtl::label(ui, RichText::new(b).size(12.0));
    }
    if let Some(r) = c["replacement"].as_str() {
        crate::rtl::label(ui, RichText::new(format!("→ {r}")).font(semibold(12.0)));
    }
    for r in all.iter().filter(|r| r["inReplyTo"] == id.as_str()) {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(12.0);
            crate::rtl::label(ui, RichText::new(r["author"]["name"].as_str().unwrap_or("?")).font(semibold(11.0)));
            crate::rtl::label(ui, RichText::new(r["body"].as_str().unwrap_or("")).size(11.0));
        });
    }
    ui.horizontal_wrapped(|ui| {
        let has_text = c["story"].is_u64();
        if ui.add_enabled(has_text && anchored != "orphaned", egui::Button::new(crate::rtl::widget(ui, tr("Go To")))).clicked() {
            *act = Some(Act::GoTo(id.clone()));
        }
        if ui.add_enabled(has_text, egui::Button::new(crate::rtl::widget(ui, tr("Add as Note")))).clicked() {
            *act = Some(Act::Note(id.clone()));
        }
        let suggestion = (kind == "replace" || kind == "strike") && anchored != "orphaned" && status != "applied";
        if suggestion
            && ui.button(crate::rtl::widget(ui, tr("Apply"))).on_hover_text(tr("As a tracked change: accept or reject it in Track Changes")).clicked()
        {
            *act = Some(Act::Apply(id.clone()));
        }
        if status == "open" || status == "resolved" {
            let label = if status == "open" { "Resolve" } else { "Reopen" };
            if ui.button(crate::rtl::widget(ui, tr(label))).clicked() {
                *act = Some(Act::Resolve(id.clone(), status == "open"));
            }
        }
    });
    let reply_id = egui::Id::new("review_panel_reply").with(&id);
    let mut reply: String = ui.data(|d| d.get_temp(reply_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        let edit = ui.add(egui::TextEdit::singleline(&mut reply).hint_text(tr("Reply…")).desired_width(180.0));
        let send = ui.add_enabled(!reply.trim().is_empty(), egui::Button::new(crate::rtl::widget(ui, tr("Reply")))).clicked()
            || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !reply.trim().is_empty());
        if send {
            *act = Some(Act::Reply(id.clone(), reply.trim().to_string()));
            reply.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(reply_id, reply));
}

fn run(app: &mut DesignApp, ctx: &egui::Context, a: Act) {
    let lang = app.ui.language.clone();
    let tr = |s: &str| crate::i18n::tr(&lang, s).to_string();
    let r = match a {
        Act::Refresh => app.run("review.projects", json!({})).map(|v| {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("review_panel_projects"), v.as_array().cloned().unwrap_or_default()));
            Value::Null
        }),
        Act::Publish(project) => {
            let p = if project.is_empty() { json!({}) } else { json!({"project": project}) };
            app.run("review.publish", p).map(|v| {
                app.status(tr("Published {version}").replace("{version}", v["version"]["label"].as_str().unwrap_or("")));
                v
            })
        }
        Act::Sync => app.run("review.sync", json!({})).map(|v| {
            app.status(tr("{n} comments synced").replace("{n}", &v["comments"].as_u64().unwrap_or(0).to_string()));
            v
        }),
        Act::Link => app.run("review.link", json!({})).map(|v| {
            if let Some(url) = v["url"].as_str() {
                ctx.data_mut(|d| d.insert_temp(egui::Id::new("review_panel_link"), url.to_string()));
            }
            v
        }),
        Act::Detach => app.run("review.detach", json!({})),
        Act::GoTo(id) => app.run("review.goTo", json!({"id": id})),
        Act::Note(id) => app.run("review.addNote", json!({"id": id})).inspect(|_| app.status(tr("Added as a note"))),
        Act::Apply(id) => app
            .run("review.apply", json!({"id": id}))
            .inspect(|_| app.status(tr("Applied as a tracked change: accept or reject it in Track Changes"))),
        Act::Resolve(id, on) => app.run("review.resolve", json!({"id": id, "on": on})),
        Act::Reply(id, body) => app.run("review.reply", json!({"id": id, "body": body})),
    };
    if let Err(e) = r {
        app.status(e);
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::{Harness, kittest::Queryable};
    use serde_json::json;

    use crate::{DesignApp, Services};

    fn harness(app: DesignApp) -> Harness<'static, DesignApp> {
        let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut DesignApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        h
    }

    /// Publish from the panel, a reviewer's suggestion arrives, Sync shows it, Apply makes the
    /// tracked change that Track Changes then accepts.
    #[test]
    fn the_panel_publishes_syncs_and_applies_a_suggestion() {
        let mut app = DesignApp::new(designcraft_engine::Session::new(), Services::default());
        let fake = designcraft_engine::review::FakeService::with_project("Acme");
        app.session.transport = Some(fake.clone());
        app.run("prefs.set", json!({"reviewServer": "https://review.test", "reviewToken": "t"})).unwrap();
        app.run("file.new", json!({})).unwrap();
        let sid = app.run("frame.create", json!({"rect": [72, 72, 400, 500], "content": "text", "text": "Hello review world"})).unwrap()["story"]
            .as_u64()
            .unwrap();
        app.ui.open_panel = Some("review".into());
        let mut h = harness(app);
        h.get_by_label("Publish").click();
        h.run_steps(3);
        assert_eq!(fake.uploads(), 1, "the panel published to the project it listed");
        assert!(h.state().session.active().unwrap().review.is_some());
        fake.add_comment(json!({"id": "c9", "kind": "replace", "page": 0, "story": sid, "start": 0, "end": 5, "quote": {"exact": "Hello"}, "replacement": "Hi", "body": "Say hi", "author": {"name": "Ann"}}));
        h.get_by_label("Sync").click();
        h.run_steps(3);
        h.get_by_label("Say hi");
        h.get_by_label("Apply").click();
        h.run_steps(3);
        let changes = h.state_mut().run("changes.list", json!({})).unwrap();
        assert!(changes.as_array().unwrap().iter().any(|c| c["kind"] == "inserted" && c["text"] == "Hi"), "{changes}");
        assert_eq!(fake.comment("c9")["status"], "applied");
        h.state_mut().run("changes.acceptAll", json!({})).unwrap();
        let text = h.state().session.active().unwrap().doc.story(designcraft_doc::StoryId(sid)).unwrap().text.clone();
        assert_eq!(text, "Hi review world");
    }
}
