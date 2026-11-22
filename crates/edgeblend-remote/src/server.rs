//! HTTP plumbing. [`route`] holds all request logic and needs no socket (unit tested);
//! [`Server`] runs it on a tiny_http background thread.

use crate::apply::{self, GeometryUndo, Op};
use crate::state;
use crate::token::{query_param, token_eq};
use anyhow::{Context, Result};
use edgeblend_config::Config;
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

/// Called after every successful phone edit, with no lock held (persist, repaint).
pub type OnChange = Arc<dyn Fn() + Send + Sync>;

pub struct Ctx {
    pub cfg: Arc<Mutex<Config>>,
    /// Shared with the desktop panel, which bumps it on its own edits.
    pub rev: Arc<AtomicU64>,
    pub undo: Mutex<Vec<GeometryUndo>>,
    pub token: String,
    pub on_change: OnChange,
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

/// Handle one request. `read_body` is only called for authenticated edits, so an
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
        ("GET", "/api/state") => {
            let cfg = ctx.cfg.lock().unwrap();
            let snap = state::snapshot(&cfg, ctx.rev.load(Ordering::SeqCst));
            drop(cfg);
            Reply::json(&snap)
        }
        ("POST", "/api/edit") => {
            let body = match read_body() {
                Ok(b) => b,
                Err(BodyError::TooLarge) => return Reply::text(413, "payload too large"),
                Err(BodyError::Unreadable) => return Reply::text(400, "bad body"),
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
            (ctx.on_change)();
            Reply::json(&snap)
        }
        (_, "/api/state" | "/api/edit") => Reply::text(405, "method not allowed"),
        _ => Reply::text(404, "not found"),
    }
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
            on_change: Arc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        };
        (ctx, calls)
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
