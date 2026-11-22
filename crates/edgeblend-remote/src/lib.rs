//! Phone remote: a small LAN HTTP server that serves the phone page and a JSON API to
//! edit the shared [`Config`] live.
//!
//! Nothing listens until [`start`] is called; dropping the returned [`RemoteHandle`] stops it.
//! Every API call needs the session token (`?t=`), which is random per start and never stored.

mod apply;
mod net;
mod server;
mod state;
mod token;

use anyhow::Result;
use edgeblend_config::history::Service as HistoryService;
use edgeblend_config::Config;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

pub use apply::{
    apply, GeometryUndo, Handle, Op, CORNER_MAX, CORNER_MIN, CURVE_MAX, CURVE_MIN, GAMMA_MAX, GAMMA_MIN,
    MAX_PROJECTORS, NUDGE_MAX, UNDO_MAX,
};
pub use server::{Change, HistoryRow, OnChange, MAX_BODY};
pub use state::{snapshot, PatternInfo, State};

/// Port tried first; the next free one above it is used if it is busy.
pub const DEFAULT_PORT: u16 = 7878;

/// A running remote server. Drop (or [`RemoteHandle::stop`]) to stop listening.
pub struct RemoteHandle {
    /// Magic link for the QR code (first LAN address).
    pub url: String,
    /// Same link on every LAN address found (first = `url`).
    pub urls: Vec<String>,
    pub port: u16,
    _server: server::Server,
}

impl RemoteHandle {
    pub fn stop(self) {}
}

/// Start serving on `0.0.0.0:port` (or the next free port).
///
/// `rev` is bumped on every phone edit; the desktop bumps it on its own edits so the phone
/// can order responses. `on_change` runs after each phone edit with no lock held.
/// `history` enables the history endpoints (list, save milestone, restore).
pub fn start(
    cfg: Arc<Mutex<Config>>,
    rev: Arc<AtomicU64>,
    history: Option<Arc<HistoryService>>,
    on_change: OnChange,
    port: u16,
) -> Result<RemoteHandle> {
    let token = token::new_token()?;
    let ctx = server::Ctx { cfg, rev, undo: Mutex::new(Vec::new()), token: token.clone(), on_change, history };
    let server = server::Server::start(ctx, port)?;
    let port = server.port;
    let ips = net::lan_ips();
    let urls: Vec<String> = if ips.is_empty() {
        vec![format!("http://127.0.0.1:{port}/?t={token}")]
    } else {
        ips.iter().map(|ip| format!("http://{ip}:{port}/?t={token}")).collect()
    };
    Ok(RemoteHandle { url: urls[0].clone(), urls, port, _server: server })
}

/// QR modules as rows of booleans (`true` = dark), for the desktop panel to paint.
pub fn qr_matrix(text: &str) -> Vec<Vec<bool>> {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return Vec::new();
    };
    let w = code.width();
    (0..w).map(|y| (0..w).map(|x| code[(x, y)] == qrcode::Color::Dark).collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[test]
    fn qr_matrix_is_square_and_mixed() {
        let m = qr_matrix("http://192.168.1.10:7878/?t=0123456789abcdef0123456789abcdef");
        assert!(!m.is_empty());
        assert!(m.iter().all(|r| r.len() == m.len()));
        assert!(m.iter().flatten().any(|&b| b) && m.iter().flatten().any(|&b| !b));
    }

    fn http(port: u16, req: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
        let text = String::from_utf8_lossy(&buf).into_owned();
        let status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
        (status, body)
    }

    fn token_of(h: &RemoteHandle) -> String {
        h.url.rsplit("t=").next().unwrap().to_string()
    }

    #[test]
    fn serves_over_tcp_and_stops() {
        let cfg = Arc::new(Mutex::new(Config::default()));
        let rev = Arc::new(AtomicU64::new(0));
        let h = start(cfg.clone(), rev.clone(), None, Arc::new(|_| {}), 17931).expect("start");
        let port = h.port;
        let t = token_of(&h);
        assert_eq!(t.len(), 32);

        let (st, body) = http(port, "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        assert_eq!(st, 200);
        assert!(body.to_ascii_lowercase().contains("<!doctype html"));
        let (st, _) = http(port, "GET /api/state HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        assert_eq!(st, 403);

        let op = r#"{"op":"set_mute","projector":2,"muted":true}"#;
        let req = format!(
            "POST /api/edit?t={t} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{op}",
            op.len()
        );
        let (st, _) = http(port, &req);
        assert_eq!(st, 200);
        assert_eq!(cfg.lock().unwrap().mute_mask, 0b100);
        assert_eq!(rev.load(Ordering::SeqCst), 1);

        let big = "x".repeat(MAX_BODY + 1);
        let req = format!(
            "POST /api/edit?t={t} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{big}",
            big.len()
        );
        assert_eq!(http(port, &req).0, 413);

        h.stop();
        let closed = (0..20).any(|_| {
            std::thread::sleep(Duration::from_millis(100));
            TcpStream::connect(("127.0.0.1", port)).is_err()
        });
        assert!(closed, "port still open after stop");
    }

    #[test]
    fn busy_port_falls_back_to_next() {
        let cfg = Arc::new(Mutex::new(Config::default()));
        let rev = Arc::new(AtomicU64::new(0));
        let a = start(cfg.clone(), rev.clone(), None, Arc::new(|_| {}), 17951).expect("first");
        let b = start(cfg, rev, None, Arc::new(|_| {}), a.port).expect("second");
        assert_ne!(a.port, b.port);
        assert!(b.port > a.port);
        assert_ne!(token_of(&a), token_of(&b));
    }
}
