//! Paths stored in documents (links, data sources, book chapters) are followed only where
//! [`crate::io::document_path`] allows: never to another computer, and resolved beside the
//! document. Every path the engine hands the file system is recorded by [`crate::io::touched`].

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Session;

const UNC: &str = r"\\192.0.2.1\share\x.png";
const FILE_URL: &str = "file://192.0.2.1/s/x.png";

fn png() -> Vec<u8> {
    designcraft_render::Rendered { width: 2, height: 2, pixels: vec![255; 16] }.to_png()
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dc-linked-paths-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A document saved as `doc` with one placed graphic per link, linked to it.
fn document_with_links(doc: &Path, links: &[&str]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    for (i, link) in links.iter().enumerate() {
        s.execute("file.place", &json!({"base64": crate::cmd::base64_encode(&png()), "name": format!("{i}.png"), "x": 72 * i, "y": 72})).unwrap();
        let link = link.to_string();
        s.edit(move |d, _| {
            let a = d.assets.values_mut().find(|a| a.link.is_none() && a.name == format!("{i}.png")).unwrap();
            std::sync::Arc::make_mut(a).link = Some(link);
            Ok(Value::Null)
        })
        .unwrap();
    }
    s.doc_mut().unwrap().path = Some(doc.to_string_lossy().into_owned());
    s
}

#[test]
fn links_to_other_computers_never_reach_the_file_system() {
    let dir = folder("remote");
    let doc = dir.join("doc.designcraft");
    let mut s = document_with_links(&doc, &[UNC, FILE_URL]);
    let (rows, seen) = crate::io::touched(|| {
        let rows = s.execute("links.list", &json!({})).unwrap();
        let issues = s.execute("preflight.run", &json!({})).unwrap();
        s.execute("links.relinkFolder", &json!({"extension": "jpg"})).unwrap();
        s.execute("links.update", &json!({})).unwrap();
        (rows, issues)
    });
    assert!(seen.is_empty(), "{seen:?}");
    let (rows, issues) = rows;
    for row in rows.as_array().unwrap() {
        assert_eq!(row["status"], "missing", "{row}");
        assert_eq!(row["refused"], "the path is on another computer", "{row}");
    }
    assert_eq!(issues.to_string().matches("Missing link").count(), 2, "{issues}");

    // Data sources and book chapters stored the same way.
    let mut d = s.doc().unwrap().doc.as_ref().clone();
    d.data_merge.sources.push(designcraft_doc::DataSource {
        id: 1,
        path: Some(UNC.replace("x.png", "data.csv")),
        relative_path: Some(FILE_URL.replace("x.png", "data.csv")),
        name: "data.csv".into(),
        delimiter: designcraft_doc::Delimiter::Comma,
        sheet: None,
        fields: vec![],
        rows: vec![],
        fingerprint: None,
        status: designcraft_doc::SourceStatus::Ok,
        warnings: vec![],
    });
    std::fs::write(&doc, designcraft_format::save(&d).unwrap()).unwrap();
    let mut opened = Session::new();
    let ((), seen) = crate::io::touched(|| {
        opened.execute("file.open", &json!({"path": doc.to_string_lossy()})).unwrap();
        opened.execute("links.list", &json!({})).unwrap();
    });
    // The document itself and the folders beside it, nothing else.
    assert!(seen.iter().all(|p| p.starts_with(&dir)), "{seen:?}");
    assert_eq!(opened.doc().unwrap().doc.data_merge.sources[0].status, designcraft_doc::SourceStatus::Missing);
    let book = dir.join("b.dcbook");
    std::fs::write(&book, json!({"documents": [UNC.replace("x.png", "c.designcraft"), FILE_URL.replace("x.png", "c.designcraft")]}).to_string())
        .unwrap();
    s.execute("book.open", &json!({"path": book.to_string_lossy()})).unwrap();
    let (list, seen) = crate::io::touched(|| s.execute("book.list", &json!({})).unwrap());
    assert!(seen.is_empty(), "{seen:?}");
    assert_eq!(list["documents"][0]["pages"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_relative_link_resolves_beside_the_document() {
    let dir = folder("relative");
    std::fs::write(dir.join("logo.png"), png()).unwrap();
    let mut s = document_with_links(&dir.join("doc.designcraft"), &["logo.png"]);
    let (rows, seen) = crate::io::touched(|| s.execute("links.list", &json!({})).unwrap());
    assert_eq!(rows[0]["status"], "ok", "{rows}");
    assert!(rows[0]["refused"].is_null());
    assert!(seen.iter().all(|p| p.starts_with(&dir)), "{seen:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Linked text stories (Flow) whose file is on another computer are missing too: listing,
/// checking and updating them, as a dry run or not, never reach the file system.
#[test]
fn text_links_to_other_computers_never_reach_the_file_system() {
    let dir = folder("remote-text");
    let doc = dir.join("doc.designcraft");
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let mut stories = Vec::new();
    for (i, link) in [UNC, FILE_URL].iter().enumerate() {
        let r =
            s.execute("frame.create", &json!({"rect": [72, 72 + 150 * i, 300, 200 + 150 * i], "content": "text", "text": "Revenue grew."})).unwrap();
        let sid = designcraft_doc::StoryId(r["story"].as_u64().unwrap());
        let link = link.to_string();
        s.edit(move |d, _| {
            d.story_mut(sid).unwrap().file_link = Some(designcraft_doc::TextFileLink { path: link, ..Default::default() });
            Ok(Value::Null)
        })
        .unwrap();
        stories.push(sid.0);
    }
    s.doc_mut().unwrap().path = Some(doc.to_string_lossy().into_owned());
    let (rows, seen) = crate::io::touched(|| {
        let rows = s.execute("links.list", &json!({})).unwrap();
        assert_eq!(s.execute("links.check", &json!({})).unwrap(), json!([]));
        s.execute("links.update", &json!({"dryRun": true})).unwrap();
        s.execute("links.update", &json!({})).unwrap();
        rows
    });
    assert!(seen.is_empty(), "{seen:?}");
    for sid in stories {
        let row = rows.as_array().unwrap().iter().find(|r| r["story"] == sid).unwrap();
        assert_eq!((row["status"].as_str(), row["refused"].as_str()), (Some("missing"), Some("the path is on another computer")), "{row}");
    }
}

/// A linked text story's relative link (as a package stores it) resolves beside the document
/// when it is listed, checked and updated.
#[test]
fn a_relative_text_link_resolves_beside_the_document() {
    let dir = folder("relative-text");
    let file = dir.join("report.txt");
    std::fs::write(&file, "Revenue grew.").unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("file.place", &json!({"path": file, "link": true, "rect": [72, 72, 300, 200]})).unwrap();
    s.edit(|d, _| {
        for st in d.stories.values_mut() {
            if let Some(l) = std::sync::Arc::make_mut(st).file_link.as_mut() {
                l.path = "report.txt".into();
            }
        }
        Ok(Value::Null)
    })
    .unwrap();
    s.doc_mut().unwrap().path = Some(dir.join("doc.designcraft").to_string_lossy().into_owned());
    std::fs::write(&file, "Revenue grew strongly.").unwrap();
    let rows = s.execute("links.list", &json!({})).unwrap();
    let row = rows.as_array().unwrap().iter().find(|r| r["story"].is_u64()).unwrap();
    assert_eq!(row["status"], "modified", "{row}");
    assert_eq!(s.execute("links.check", &json!({})).unwrap(), json!([]));
    assert_eq!(s.execute("links.update", &json!({})).unwrap()["updated"], 1);
    let text: Vec<String> = s.doc().unwrap().doc.stories.values().map(|st| st.text.clone()).collect();
    assert!(text.iter().any(|t| t.contains("strongly")), "{text:?}");
}
