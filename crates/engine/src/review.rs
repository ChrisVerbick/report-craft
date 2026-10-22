//! The review service as the engine sees it: a transport the app provides (HTTP lives above the
//! engine, which must build for the web), a client for the service's owner API, the state a
//! published document keeps in the session, and an in-memory service for tests.

use std::io::Read;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use crate::{EngineError, Result};

pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub bearer: Option<String>,
    pub content_type: Option<&'static str>,
    pub body: Vec<u8>,
}

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Sends requests to the review service. The desktop app provides one; a build without one
/// says so when a review command runs.
pub trait Transport: Send + Sync {
    fn send(&self, req: HttpRequest) -> std::result::Result<HttpResponse, String>;
}

/// Where the document is published and the comments last synced.
#[derive(Clone, Debug, Default)]
pub struct ReviewState {
    pub server: String,
    /// `{id, name, client}`.
    pub project: Value,
    /// The service's id for this document.
    pub document: String,
    /// The last version published or seen: `{id, n, label, uploaded, pages}`.
    pub version: Option<Value>,
    pub comments: Vec<designcraft_review::Comment>,
    pub synced: i64,
}

/// The owner API, as the app uses it.
pub struct Client<'a> {
    pub server: &'a str,
    pub token: &'a str,
    pub transport: &'a dyn Transport,
}

impl Client<'_> {
    fn raw(&self, method: &'static str, path: &str, body: Option<(&'static str, Vec<u8>)>) -> Result<(u16, Value)> {
        let (content_type, body) = body.map_or((None, Vec::new()), |(c, b)| (Some(c), b));
        let req = HttpRequest {
            method,
            url: format!("{}{path}", self.server.trim_end_matches('/')),
            bearer: Some(self.token.to_string()),
            content_type,
            body,
        };
        let resp = self.transport.send(req).map_err(|e| EngineError::Other(format!("review service: {e}")))?;
        let v = if resp.body.is_empty() { Value::Null } else { serde_json::from_slice(&resp.body).unwrap_or(Value::Null) };
        Ok((resp.status, v))
    }

    fn ok(&self, method: &'static str, path: &str, body: Option<(&'static str, Vec<u8>)>) -> Result<Value> {
        let (status, v) = self.raw(method, path, body)?;
        if status >= 400 {
            let msg = v["error"].as_str().map_or_else(|| format!("the review service answered {status}"), str::to_string);
            return Err(EngineError::Other(format!("review service: {msg}")));
        }
        Ok(v)
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.ok("GET", path, None)
    }

    /// `GET` that may answer 404: `None` then.
    pub fn get_opt(&self, path: &str) -> Result<Option<Value>> {
        match self.raw("GET", path, None)? {
            (404, _) => Ok(None),
            (s, v) if s >= 400 => Err(EngineError::Other(format!("review service: {}", v["error"].as_str().unwrap_or("error")))),
            (_, v) => Ok(Some(v)),
        }
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.ok("POST", path, Some(("application/json", body.to_string().into_bytes())))
    }

    pub fn patch(&self, path: &str, body: &Value) -> Result<Value> {
        self.ok("PATCH", path, Some(("application/json", body.to_string().into_bytes())))
    }

    pub fn post_zip(&self, path: &str, zip: Vec<u8>) -> Result<Value> {
        self.ok("POST", path, Some(("application/zip", zip)))
    }
}

/// An in-memory review service with the owner API's routes, for tests: projects, document
/// bindings, uploads, and the comments a test puts there.
#[derive(Default)]
pub struct FakeService {
    inner: Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    projects: Vec<Value>,
    /// app document id → (project id, document id, title, versions)
    bindings: Vec<(String, String, String, String, i64)>,
    comments: Vec<Value>,
    log: Vec<(String, String)>,
    next: u32,
}

impl FakeService {
    pub fn with_project(name: &str) -> Arc<Self> {
        let s = Self::default();
        s.lock().projects.push(json!({"id": "p1", "name": name, "client": ""}));
        Arc::new(s)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A comment as a reviewer made it; what's left out is filled in (id, document, version,
    /// status, author).
    pub fn add_comment(&self, mut c: Value) {
        let mut st = self.lock();
        st.next += 1;
        let (doc, version) = st.bindings.first().map(|b| (b.2.clone(), format!("v{}", b.4))).unwrap_or_default();
        let o = c.as_object_mut().map(std::mem::take).unwrap_or_default();
        let mut filled = json!({"id": format!("c{}", st.next), "document": doc, "version": version, "status": "open", "created": 1_700_000_000, "modified": 1_700_000_000, "body": "", "author": {"id": "m1", "name": "Reviewer", "role": "reviewer"}});
        for (k, v) in o {
            filled[k] = v;
        }
        st.comments.push(filled);
    }

    pub fn comment(&self, id: &str) -> Value {
        self.lock().comments.iter().find(|c| c["id"] == id).cloned().unwrap_or(Value::Null)
    }

    pub fn comments(&self) -> Vec<Value> {
        self.lock().comments.clone()
    }

    pub fn binding(&self, app_document_id: &str) -> Option<Value> {
        let st = self.lock();
        let b = st.bindings.iter().find(|b| b.0 == app_document_id)?;
        let project = st.projects.iter().find(|p| p["id"] == b.1.as_str()).cloned().unwrap_or(Value::Null);
        Some(
            json!({"project": project, "document": {"id": b.2, "appDocumentId": b.0, "title": b.3}, "versions": (1..=b.4).map(|n| json!({"id": format!("v{n}"), "n": n, "label": format!("Review v{n}"), "uploaded": 0, "pages": 1})).collect::<Vec<_>>()}),
        )
    }

    pub fn uploads(&self) -> usize {
        self.lock().log.iter().filter(|(m, u)| m == "POST" && u.ends_with("/versions")).count()
    }

    /// Every request seen: (method, path).
    pub fn log(&self) -> Vec<(String, String)> {
        self.lock().log.clone()
    }

    fn route(&self, method: &str, path: &str, body: &[u8]) -> (u16, Value) {
        let mut st = self.lock();
        st.log.push((method.into(), path.into()));
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let json_body: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        match (method, parts.as_slice()) {
            ("GET", ["api", "projects"]) => (200, Value::Array(st.projects.clone())),
            ("POST", ["api", "projects"]) => {
                st.next += 1;
                let p = json!({"id": format!("p{}", st.next), "name": json_body["name"], "client": json_body["client"].as_str().unwrap_or("")});
                st.projects.push(p.clone());
                (201, p)
            }
            ("GET", ["api", "documents", app]) => {
                drop(st);
                self.binding(app).map_or((404, json!({"error": "no such document"})), |b| (200, b))
            }
            ("POST", ["api", "projects", p, "versions"]) => {
                let Some(manifest) = manifest_in(body) else { return (400, json!({"error": "review bundle: no manifest.json"})) };
                let app = manifest["documentId"].as_str().unwrap_or("").to_string();
                let title = manifest["title"].as_str().unwrap_or("Untitled").to_string();
                let pages = manifest["pages"].as_array().map_or(0, Vec::len);
                let p = p.to_string();
                if let Some(i) = st.bindings.iter().position(|b| b.0 == app) {
                    let bound = st.bindings[i].1.clone();
                    if bound != p {
                        let name = st.projects.iter().find(|x| x["id"] == bound.as_str()).and_then(|x| x["name"].as_str()).unwrap_or("?").to_string();
                        return (
                            409,
                            json!({"error": format!("this document belongs to the project “{name}”; detach it in the app (review.detach) to start a review of its own")}),
                        );
                    }
                    st.bindings[i].4 += 1;
                    let (n, doc) = (st.bindings[i].4, st.bindings[i].2.clone());
                    return (
                        201,
                        json!({"document": {"id": doc, "appDocumentId": app, "title": title}, "version": {"id": format!("v{n}"), "n": n, "label": format!("Review v{n}"), "uploaded": 0, "pages": pages}}),
                    );
                }
                st.next += 1;
                let doc = format!("d{}", st.next);
                st.bindings.push((app.clone(), p, doc.clone(), title.clone(), 1));
                (
                    201,
                    json!({"document": {"id": doc, "appDocumentId": app, "title": title}, "version": {"id": "v1", "n": 1, "label": "Review v1", "uploaded": 0, "pages": pages}}),
                )
            }
            ("GET", ["api", "projects", _, "comments.json"]) => (200, json!({"comments": st.comments})),
            ("PATCH", ["api", "comments", id]) => match st.comments.iter_mut().find(|c| c["id"] == *id) {
                Some(c) => {
                    c["status"] = json_body["status"].clone();
                    (200, c.clone())
                }
                None => (404, json!({"error": "no such comment"})),
            },
            ("POST", ["api", "comments", id, "replies"]) => {
                let Some(parent) = st.comments.iter().find(|c| c["id"] == *id).cloned() else { return (404, json!({"error": "no such comment"})) };
                st.next += 1;
                let r = json!({"id": format!("c{}", st.next), "document": parent["document"], "version": parent["version"], "inReplyTo": id, "kind": "reply", "page": parent["page"], "body": json_body["body"], "status": "open", "created": 1_700_000_001, "modified": 1_700_000_001, "author": {"id": "owner", "name": json_body["name"].as_str().unwrap_or("Owner"), "role": "owner"}});
                st.comments.push(r.clone());
                (201, r)
            }
            ("POST", ["api", "projects", _, "share-links"]) => {
                (201, json!({"id": "l1", "token": "tok", "url": "https://review.test/r/tok", "mode": json_body["mode"].as_str().unwrap_or("invite")}))
            }
            _ => (404, json!({"error": "no such route"})),
        }
    }
}

/// The manifest inside a bundle zip.
fn manifest_in(zip: &[u8]) -> Option<Value> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip)).ok()?;
    let mut f = z.by_name("manifest.json").ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    serde_json::from_str(&s).ok()
}

impl Transport for FakeService {
    fn send(&self, req: HttpRequest) -> std::result::Result<HttpResponse, String> {
        if req.bearer.as_deref().is_none_or(str::is_empty) {
            return Ok(HttpResponse { status: 401, body: br#"{"error":"the admin token"}"#.to_vec() });
        }
        let path = req.url.find("/api/").map(|i| &req.url[i..]).unwrap_or("/");
        let (status, body) = self.route(req.method, path, &req.body);
        Ok(HttpResponse { status, body: body.to_string().into_bytes() })
    }
}
