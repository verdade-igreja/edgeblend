//! HTTP plumbing. [`route`] holds all request logic and needs no socket (unit tested);
//! [`Server`] runs it on a tiny_http background thread.

use crate::apply::{self, GeometryUndo, Op};
use crate::state;
use crate::token::{query_param, token_eq};
use anyhow::{Context, Result};
use edgeblend_config::history::{self, Service as History, Source};
use edgeblend_config::Config;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The built phone app (single self-contained file, see `web/`).
const INDEX_HTML: &str = include_str!("../web/dist/index.html");
/// Edit ops are tiny JSON objects.
pub const MAX_BODY: usize = 4 * 1024;
/// How many ports above the requested one to try when it is busy.
const PORT_TRIES: u16 = 10;
const STOP_JOIN_TIMEOUT: Duration = Duration::from_secs(1);

/// What the phone changed in the live config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// An edit op was applied.
    Edit,
    /// A history version replaced the live config (already saved to disk by the history).
    /// `displays_changed`: source/output differ, so a running engine must be restarted.
    Restored { displays_changed: bool },
}

/// Called after every successful phone change, with no lock held (persist, repaint, ...).
pub type OnChange = Arc<dyn Fn(Change) + Send + Sync>;

pub struct Ctx {
    pub cfg: Arc<Mutex<Config>>,
    /// Shared with the desktop panel, which bumps it on its own edits.
    pub rev: Arc<AtomicU64>,
    pub undo: Mutex<Vec<GeometryUndo>>,
    pub token: String,
    pub on_change: OnChange,
    /// Config version history; `None` disables the history endpoints.
    pub history: Option<Arc<History>>,
}

/// One row of `GET /api/history` (newest first).
#[derive(Debug, Serialize)]
pub struct HistoryRow {
    pub id: String,
    /// RFC 3339 UTC; the phone shows it in its local time.
    pub created: String,
    pub created_ms: i64,
    pub milestone: bool,
    pub name: Option<String>,
    pub source: Source,
    /// Unreadable file: listed, but cannot be restored.
    pub broken: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilestoneReq {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreReq {
    id: String,
}

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub ctype: &'static str,
    pub body: Vec<u8>,
}

impl Reply {
    fn text(status: u16, msg: impl Into<String>) -> Self {
        Self { status, ctype: "text/plain; charset=utf-8", body: msg.into().into_bytes() }
    }

    fn json(value: &impl serde::Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => Self { status: 200, ctype: "application/json", body },
            Err(e) => Self::text(500, format!("encode: {e}")),
        }
    }
}

/// Why a request body could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyError {
    TooLarge,
    Unreadable,
}

/// Handle one request. `read_body` is only called for authenticated POSTs, so an
/// unauthenticated client never gets its body read.
pub fn route(
    ctx: &Ctx,
    method: &str,
    url: &str,
    read_body: impl FnOnce() -> Result<Vec<u8>, BodyError>,
) -> Reply {
    let (path, query) = match url.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (url, None),
    };
    if method == "GET" && (path == "/" || path == "/index.html") {
        // Static page, no secrets in it; it reads the token from its own URL.
        return Reply { status: 200, ctype: "text/html; charset=utf-8", body: INDEX_HTML.as_bytes().to_vec() };
    }
    if !path.starts_with("/api/") {
        return Reply::text(404, "not found");
    }
    if !query_param(query, "t").is_some_and(|t| token_eq(t, &ctx.token)) {
        return Reply::text(403, "forbidden");
    }
    match (method, path) {
        ("GET", "/api/history") => match &ctx.history {
            Some(h) => Reply::json(&history_rows(h)),
            None => Reply::text(404, "history not available"),
        },
        ("POST", "/api/milestone") => {
            let Some(h) = &ctx.history else { return Reply::text(404, "history not available") };
            let body = match body_reply(read_body()) {
                Ok(b) => b,
                Err(r) => return r,
            };
            let req: MilestoneReq = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => return Reply::text(400, format!("bad request: {e}")),
            };
            match h.save_milestone(req.name, Source::Remote) {
                Ok(id) => Reply::json(&serde_json::json!({ "id": id })),
                Err(e) => {
                    eprintln!("remote: milestone failed: {e:#}");
                    Reply::text(500, "could not save the milestone")
                }
            }
        }
        ("POST", "/api/restore") => {
            let Some(h) = &ctx.history else { return Reply::text(404, "history not available") };
            let body = match body_reply(read_body()) {
                Ok(b) => b,
                Err(r) => return r,
            };
            let req: RestoreReq = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => return Reply::text(400, format!("bad request: {e}")),
            };
            if !history::valid_id(&req.id) {
                return Reply::text(400, "invalid version id");
            }
            match h.list().into_iter().find(|e| e.id == req.id) {
                None => return Reply::text(404, "no such version"),
                Some(e) if e.broken.is_some() => return Reply::text(409, "that version file is unreadable"),
                Some(_) => {}
            }
            let displays = |c: &Config| (c.source.clone(), c.output.clone());
            let before = displays(&ctx.cfg.lock().unwrap());
            // Service::restore locks the config itself and writes blend.toml: no lock held here.
            if let Err(e) = h.restore(&req.id) {
                eprintln!("remote: restore {} failed: {e:#}", req.id);
                return Reply::text(400, "could not restore that version");
            }
            let snap = {
                let cfg = ctx.cfg.lock().unwrap();
                let displays_changed = displays(&cfg) != before;
                let rev = ctx.rev.fetch_add(1, Ordering::SeqCst) + 1;
                (state::snapshot(&cfg, rev), displays_changed)
            };
            ctx.undo.lock().unwrap().clear(); // geometry undo no longer matches
            (ctx.on_change)(Change::Restored { displays_changed: snap.1 });
            Reply::json(&snap.0)
        }
        ("GET", "/api/state") => {
            let cfg = ctx.cfg.lock().unwrap();
            let snap = state::snapshot(&cfg, ctx.rev.load(Ordering::SeqCst));
            drop(cfg);
            Reply::json(&snap)
        }
        ("POST", "/api/edit") => {
            let body = match body_reply(read_body()) {
                Ok(b) => b,
                Err(r) => return r,
            };
            let op: Op = match serde_json::from_slice(&body) {
                Ok(op) => op,
                Err(e) => return Reply::text(400, format!("bad op: {e}")),
            };
            let snap = {
                // Lock order: cfg, then undo (the only place both are taken).
                let mut cfg = ctx.cfg.lock().unwrap();
                let mut undo = ctx.undo.lock().unwrap();
                if let Err(e) = apply::apply(&mut cfg, &op, &mut undo) {
                    return Reply::text(400, format!("{e:#}"));
                }
                let rev = ctx.rev.fetch_add(1, Ordering::SeqCst) + 1;
                state::snapshot(&cfg, rev)
            };
            if let Some(h) = &ctx.history {
                h.hint_source(Source::Remote); // label the next auto version
            }
            (ctx.on_change)(Change::Edit);
            Reply::json(&snap)
        }
        (_, "/api/state" | "/api/edit" | "/api/history" | "/api/milestone" | "/api/restore") => Reply::text(405, "method not allowed"),
        _ => Reply::text(404, "not found"),
    }
}

fn body_reply(body: Result<Vec<u8>, BodyError>) -> Result<Vec<u8>, Reply> {
    body.map_err(|e| match e {
        BodyError::TooLarge => Reply::text(413, "payload too large"),
        BodyError::Unreadable => Reply::text(400, "bad body"),
    })
}

fn history_rows(h: &History) -> Vec<HistoryRow> {
    h.list()
        .into_iter()
        .map(|e| HistoryRow {
            id: e.id,
            created: e.created,
            created_ms: e.created_ms,
            milestone: e.kind == history::Kind::Milestone,
            name: e.name,
            source: e.source,
            broken: e.broken.is_some(),
        })
        .collect()
}

fn read_limited(req: &mut tiny_http::Request) -> Result<Vec<u8>, BodyError> {
    if req.body_length().is_some_and(|n| n > MAX_BODY) {
        return Err(BodyError::TooLarge);
    }
    let mut body = Vec::new();
    req.as_reader()
        .take(MAX_BODY as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|_| BodyError::Unreadable)?;
    if body.len() > MAX_BODY {
        return Err(BodyError::TooLarge);
    }
    Ok(body)
}

fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
    img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn serve(ctx: &Ctx, mut req: tiny_http::Request) {
    let method = req.method().as_str().to_ascii_uppercase();
    let url = req.url().to_string();
    let reply = route(ctx, &method, &url, || read_limited(&mut req));
    let resp = tiny_http::Response::from_data(reply.body)
        .with_status_code(reply.status)
        .with_header(header("Content-Type", reply.ctype))
        .with_header(header("Cache-Control", "no-store"))
        // The token is in the page URL: never leak it through Referer.
        .with_header(header("Referrer-Policy", "no-referrer"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("Content-Security-Policy", CSP));
    let _ = req.respond(resp);
}

/// Running listener. Dropping it stops the thread and closes the port.
pub struct Server {
    pub port: u16,
    stop: Arc<AtomicBool>,
    http: Arc<tiny_http::Server>,
    join: Option<JoinHandle<()>>,
}

impl Server {
    /// Bind `0.0.0.0:port` (or the next free port above it) and serve on a background thread.
    pub fn start(ctx: Ctx, port: u16) -> Result<Self> {
        let (listener, port) = bind_first_free(port)?;
        let http = Arc::new(
            tiny_http::Server::from_listener(listener, None).map_err(|e| anyhow::anyhow!("http server: {e}"))?,
        );
        let stop = Arc::new(AtomicBool::new(false));
        let (stop_t, http_t) = (stop.clone(), http.clone());
        let join = thread::Builder::new()
            .name("edgeblend-remote".into())
            .spawn(move || {
                while !stop_t.load(Ordering::SeqCst) {
                    match http_t.recv_timeout(Duration::from_millis(200)) {
                        Ok(Some(req)) if !stop_t.load(Ordering::SeqCst) => serve(&ctx, req),
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            })
            .context("spawn remote thread")?;
        Ok(Self { port, stop, http, join: Some(join) })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.http.unblock();
        // Wake the accept loop so tiny_http's listener thread notices the shutdown.
        let _ = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], self.port)), Duration::from_millis(100));
        if let Some(j) = self.join.take() {
            // Bounded wait: a stalled client must not hang Quit.
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = j.join();
                let _ = tx.send(());
            });
            let _ = rx.recv_timeout(STOP_JOIN_TIMEOUT);
        }
    }
}

fn bind_first_free(port: u16) -> Result<(TcpListener, u16)> {
    let mut last = None;
    for p in port..=port.saturating_add(PORT_TRIES) {
        match TcpListener::bind(("0.0.0.0", p)) {
            Ok(l) => return Ok((l, p)),
            Err(e) => last = Some(e),
        }
    }
    Err(anyhow::anyhow!("no free port in {port}..={}: {}", port.saturating_add(PORT_TRIES), last.map(|e| e.to_string()).unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const T: &str = "0123456789abcdef0123456789abcdef";

    fn ctx() -> (Ctx, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let ctx = Ctx {
            cfg: Arc::new(Mutex::new(Config::default())),
            rev: Arc::new(AtomicU64::new(0)),
            undo: Mutex::new(Vec::new()),
            token: T.into(),
            on_change: Arc::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
            }),
            history: None,
        };
        (ctx, calls)
    }

    /// Temp dir with a blend.toml path whose sibling `history/` the Service uses.
    struct Tmp(std::path::PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ctx_with_history() -> (Ctx, Arc<std::sync::Mutex<Vec<Change>>>, Tmp) {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "edgeblend-remote-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (mut c, _) = ctx();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        c.on_change = Arc::new(move |ch| s.lock().unwrap().push(ch));
        c.history = Some(History::start(c.cfg.clone(), dir.join("blend.toml")));
        (c, seen, Tmp(dir))
    }

    #[test]
    fn history_endpoints_need_token() {
        let (c, _, _tmp) = ctx_with_history();
        assert_eq!(route(&c, "GET", "/api/history", no_body).status, 403);
        assert_eq!(route(&c, "POST", "/api/milestone?t=bad", no_body).status, 403);
        assert_eq!(route(&c, "POST", "/api/restore", no_body).status, 403);
    }

    #[test]
    fn history_disabled_is_404() {
        let (c, _) = ctx();
        assert_eq!(route(&c, "GET", &format!("/api/history?t={T}"), no_body).status, 404);
        assert_eq!(route(&c, "POST", &format!("/api/milestone?t={T}"), no_body).status, 404);
    }

    #[test]
    fn milestone_then_restore() {
        let (c, seen, _tmp) = ctx_with_history();
        let h = c.history.clone().unwrap();
        c.cfg.lock().unwrap().gamma = 1.5;
        let r = route(&c, "POST", &format!("/api/milestone?t={T}"), || Ok(br#"{"name":"  good one "}"#.to_vec()));
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let id = json(&r)["id"].as_str().unwrap().to_string();

        let r = route(&c, "GET", &format!("/api/history?t={T}"), no_body);
        assert_eq!(r.status, 200);
        let rows = json(&r);
        let row = rows.as_array().unwrap().iter().find(|e| e["id"] == id.as_str()).expect("listed");
        assert_eq!(row["milestone"], true);
        assert_eq!(row["name"], "good one");
        assert_eq!(row["source"], "remote");
        assert_eq!(row["broken"], false);

        // Unnamed milestone is fine too.
        let r = route(&c, "POST", &format!("/api/milestone?t={T}"), || Ok(b"{}".to_vec()));
        assert_eq!(r.status, 200);

        c.cfg.lock().unwrap().gamma = 2.8;
        let body = format!(r#"{{"id":"{id}"}}"#).into_bytes();
        let r = route(&c, "POST", &format!("/api/restore?t={T}"), || Ok(body));
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        assert!((json(&r)["gamma"].as_f64().unwrap() - 1.5).abs() < 1e-6);
        assert!((c.cfg.lock().unwrap().gamma - 1.5).abs() < 1e-6);
        assert_eq!(c.rev.load(Ordering::SeqCst), 1);
        assert_eq!(*seen.lock().unwrap(), vec![Change::Restored { displays_changed: false }]);
        drop(h);
    }

    #[test]
    fn restore_rejects_bad_ids() {
        let (c, seen, _tmp) = ctx_with_history();
        let url = format!("/api/restore?t={T}");
        let cases: [(&[u8], u16); 6] = [
            (br#"{"id":"../../blend"}"#, 400),
            (br#"{"id":"C:\x"}"#, 400),
            (br#"{"id":""}"#, 400),
            (br#"{"id":"20990101-000000-000"}"#, 404),
            (br#"{"nope":1}"#, 400),
            (b"[]", 400),
        ];
        for (body, status) in cases {
            let r = route(&c, "POST", &url, || Ok(body.to_vec()));
            assert_eq!(r.status, status, "{}", String::from_utf8_lossy(body));
        }
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(c.rev.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn restore_reports_display_change() {
        let (c, seen, _tmp) = ctx_with_history();
        c.cfg.lock().unwrap().output = r"\\.\DISPLAY3".into();
        let r = route(&c, "POST", &format!("/api/milestone?t={T}"), || Ok(b"{}".to_vec()));
        let id = json(&r)["id"].as_str().unwrap().to_string();
        c.cfg.lock().unwrap().output = r"\\.\DISPLAY2".into();
        let body = format!(r#"{{"id":"{id}"}}"#).into_bytes();
        assert_eq!(route(&c, "POST", &format!("/api/restore?t={T}"), || Ok(body)).status, 200);
        assert_eq!(*seen.lock().unwrap(), vec![Change::Restored { displays_changed: true }]);
    }

    fn no_body() -> Result<Vec<u8>, BodyError> {
        panic!("body must not be read")
    }

    fn json(r: &Reply) -> serde_json::Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    #[test]
    fn page_is_public() {
        let (c, _) = ctx();
        let r = route(&c, "GET", "/", no_body);
        assert_eq!(r.status, 200);
        assert!(r.ctype.starts_with("text/html"));
    }

    #[test]
    fn api_requires_exact_token() {
        let (c, _) = ctx();
        for url in ["/api/state", "/api/state?t=", "/api/state?t=nope", "/api/state?x=1"] {
            assert_eq!(route(&c, "GET", url, no_body).status, 403, "{url}");
        }
        let wrong = &T[..31];
        assert_eq!(route(&c, "GET", &format!("/api/state?t={wrong}"), no_body).status, 403);
        // Unauthenticated edit: 403 without reading the body.
        assert_eq!(route(&c, "POST", "/api/edit", no_body).status, 403);
        assert_eq!(route(&c, "GET", &format!("/api/state?t={T}"), no_body).status, 200);
    }

    #[test]
    fn state_lists_patterns_and_mutes() {
        let (c, _) = ctx();
        c.cfg.lock().unwrap().mute_mask = 0b10;
        c.rev.store(5, Ordering::SeqCst);
        let r = route(&c, "GET", &format!("/api/state?t={T}"), no_body);
        let v = json(&r);
        assert_eq!(v["rev"], 5);
        assert_eq!(v["muted"], serde_json::json!([false, true, false]));
        assert_eq!(v["patterns"].as_array().unwrap().len(), edgeblend_config::Pattern::ALL.len());
    }

    #[test]
    fn edit_applies_bumps_rev_and_notifies() {
        let (c, calls) = ctx();
        let url = format!("/api/edit?t={T}");
        let r = route(&c, "POST", &url, || Ok(br#"{"op":"set_mute","projector":0,"muted":true}"#.to_vec()));
        assert_eq!(r.status, 200);
        assert_eq!(json(&r)["rev"], 1);
        assert_eq!(json(&r)["muted"][0], true);
        assert_eq!(c.cfg.lock().unwrap().mute_mask, 1);

        let r = route(&c, "POST", &url, || Ok(br#"{"op":"pattern","value":"alignment"}"#.to_vec()));
        assert_eq!(json(&r)["pattern"], "alignment");
        assert_eq!(json(&r)["rev"], 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn bad_edits_change_nothing() {
        let (c, calls) = ctx();
        let url = format!("/api/edit?t={T}");
        let before = c.cfg.lock().unwrap().clone();
        let cases: [(&[u8], u16); 5] = [
            (br#"{"op":"set_mute","projector":3,"muted":true}"#, 400),
            (br#"{"op":"pattern","value":"<img src=x>"}"#, 400),
            (b"not json", 400),
            (br#"{"op":"gamma","value":1e40}"#, 400),
            (b"\xff\xfe", 400),
        ];
        for (body, status) in cases {
            let r = route(&c, "POST", &url, || Ok(body.to_vec()));
            assert_eq!(r.status, status, "{}", String::from_utf8_lossy(body));
        }
        assert_eq!(route(&c, "POST", &url, || Err(BodyError::TooLarge)).status, 413);
        assert_eq!(route(&c, "POST", &url, || Err(BodyError::Unreadable)).status, 400);
        assert_eq!(*c.cfg.lock().unwrap(), before);
        assert_eq!(c.rev.load(Ordering::SeqCst), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn wrong_method_and_unknown_paths() {
        let (c, _) = ctx();
        assert_eq!(route(&c, "GET", &format!("/api/edit?t={T}"), no_body).status, 405);
        assert_eq!(route(&c, "POST", &format!("/api/state?t={T}"), no_body).status, 405);
        assert_eq!(route(&c, "GET", &format!("/api/other?t={T}"), no_body).status, 404);
        assert_eq!(route(&c, "GET", "/etc/passwd", no_body).status, 404);
    }
}
