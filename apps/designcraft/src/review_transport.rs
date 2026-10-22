//! The review service over HTTP, for the engine's review commands (the engine has no networking
//! of its own: it must build for the web). TLS comes from the operating system.

use designcraft_engine::review::{HttpRequest, HttpResponse, Transport};

pub struct UreqTransport(ureq::Agent);

impl UreqTransport {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(180)))
            .user_agent(concat!("DesignCraft/", env!("CARGO_PKG_VERSION")))
            .build();
        Self(ureq::Agent::new_with_config(config))
    }
}

impl Transport for UreqTransport {
    fn send(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        let bearer = req.bearer.map(|t| format!("Bearer {t}"));
        let result = match req.method {
            "GET" => {
                let mut b = self.0.get(&req.url);
                if let Some(a) = &bearer {
                    b = b.header("Authorization", a);
                }
                b.call()
            }
            "POST" | "PATCH" | "PUT" => {
                let mut b = match req.method {
                    "PATCH" => self.0.patch(&req.url),
                    "PUT" => self.0.put(&req.url),
                    _ => self.0.post(&req.url),
                };
                if let Some(a) = &bearer {
                    b = b.header("Authorization", a);
                }
                if let Some(ct) = req.content_type {
                    b = b.header("Content-Type", ct);
                }
                b.send(&req.body[..])
            }
            m => return Err(format!("unsupported method {m}")),
        };
        let mut resp = result.map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(64 << 20).read_to_vec().map_err(|e| e.to_string())?;
        Ok(HttpResponse { status, body })
    }
}
