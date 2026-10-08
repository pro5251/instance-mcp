//! Switchboard mode: dial an openab-sb `GET /vm/attach` and serve MCP there under one
//! profile, for as long as the process runs. Contract: openab-sb
//! `docs/SOUTHBOUND-CONTRACT.md`. Mirrors the Swift `Switchboard` (URL rule, per-profile
//! instructions) and the southbound reconnect table.
//!
//! ```text
//!   Connect / PTY agent ──MCP──► openab-sb ◄── WS (we dial) ── this node (MCP server)
//! ```
//! The switchboard applies its own per-caller allowlist on top; the profile here is this
//! computer's own ceiling and the one that matters if the switchboard is misconfigured.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tungstenite::http::Request;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

use crate::mcp::answer_with;

/// Accept `wss://…/vm/attach`, or `ws://` to a loopback host only (the secret must not
/// cross a network in clear). Returns the URL unchanged on success.
pub(crate) fn validate_url(text: &str) -> Result<String, String> {
    let bad =
        || format!("--switchboard wants a ws:// or wss:// URL ending in /vm/attach, got {text}");
    let (scheme, rest) = text.split_once("://").ok_or_else(bad)?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "ws" && scheme != "wss" {
        return Err(bad());
    }
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(authority);
    if host.is_empty() || !path_ends_with_vm_attach(path) {
        return Err(bad());
    }
    if scheme == "ws" && !is_loopback(host) {
        return Err(format!(
            "--switchboard: ws:// is only allowed to loopback; use wss:// for {host} \
             (the secret would cross the network in clear)"
        ));
    }
    Ok(text.to_string())
}

fn path_ends_with_vm_attach(path: &str) -> bool {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    path.trim_end_matches('/').ends_with("/vm/attach") || path == "/vm/attach"
}

fn is_loopback(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
    h == "localhost" || h == "::1" || h.starts_with("127.")
}

/// What to do when the switchboard closes the socket (southbound §5).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Close {
    /// Stop dialling for good (another daemon won, or the secret was revoked).
    Stop(&'static str),
    /// Redial with backoff.
    Redial,
}

pub(crate) fn disposition_close(code: u16) -> Close {
    match code {
        4002 => Close::Stop("replaced"),
        4003 => Close::Stop("secret_revoked"),
        // 4005 (bad/late initialize), 1001 (restarting), 1000 / abnormal / timeout: redial.
        _ => Close::Redial,
    }
}

/// What to do about the dial's HTTP handshake status (southbound §2).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Handshake {
    Attached,
    /// Wrong/missing secret: retry at most every ~5 minutes.
    RetrySlow,
    /// Switchboard down / restarting: redial with backoff.
    Redial,
}

pub(crate) fn disposition_handshake(status: u16) -> Handshake {
    match status {
        101 => Handshake::Attached,
        401 => Handshake::RetrySlow,
        _ => Handshake::Redial,
    }
}

/// Backoff: 1 s doubling to a 60 s cap, ±20 % jitter; reset to 1 s after a connection that
/// stayed up ≥ 60 s (southbound §5).
pub(crate) struct Backoff {
    base: u64,
}

impl Backoff {
    fn new() -> Self {
        Backoff { base: 1 }
    }
    fn reset(&mut self) {
        self.base = 1;
    }
    /// Next sleep in milliseconds, then grow the base.
    fn next_ms(&mut self) -> u64 {
        let base = self.base;
        self.base = (self.base * 2).min(60);
        jitter_ms(base)
    }
}

/// `secs` seconds ±20 %, in milliseconds. Jitter seed is the clock (no rand crate).
fn jitter_ms(secs: u64) -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let frac = (nanos % 1000) as f64 / 1000.0; // 0.0..1.0
    let factor = 0.8 + frac * 0.4; // 0.8..1.2
    ((secs as f64) * 1000.0 * factor) as u64
}

const RETRY_SLOW_SECS: u64 = 300;

/// Per-profile initialize instructions, prepended to the platform's base, telling the
/// agent it arrived through the switchboard (mirrors the Swift `Switchboard.instructions`).
fn instructions(profile: &str, base: Option<&str>) -> Option<String> {
    let head = match profile {
        "observe" => {
            "You reached this computer through OpenAB Switchboard under the `observe` profile: \
             you may look, not act. Only `sys_info` and `screenshot` are available."
        }
        "desktop" | "owner" => {
            "You reached this computer through OpenAB Switchboard. Calls are relayed over the \
             network, so expect a second or more per call; a human may be watching the screen."
        }
        _ => return base.map(|s| s.to_string()),
    };
    Some(match base {
        Some(b) => format!("{head}\n\n{b}"),
        None => head.to_string(),
    })
}

fn sleep_cancellable(ms: u64, cancelled: &AtomicBool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        if cancelled.load(Ordering::Acquire) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100).min(deadline - Instant::now()));
    }
    !cancelled.load(Ordering::Acquire)
}

fn build_request(url: &str, secret: &str) -> Result<Request<()>, String> {
    use tungstenite::handshake::client::generate_key;
    let rest = url
        .strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .ok_or_else(|| "switchboard url missing ws scheme".to_string())?;
    let authority = rest.split('/').next().unwrap_or(rest);
    Request::builder()
        .method("GET")
        .uri(url)
        .header("Host", authority)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key())
        .header("Authorization", format!("Bearer {secret}"))
        .body(())
        .map_err(|e| format!("failed to build request: {e}"))
}

fn handshake_status(err: &tungstenite::Error) -> Option<u16> {
    match err {
        tungstenite::Error::Http(resp) => Some(resp.status().as_u16()),
        _ => None,
    }
}

/// The switchboard dial loop. Re-reads `secret_file` on every dial (rotation), serves MCP
/// under `profile`, and reconnects per the southbound table until `cancelled`.
pub(crate) fn run(url: String, secret_file: String, profile: String, cancelled: Arc<AtomicBool>) {
    let mut backoff = Backoff::new();
    let instr = instructions(&profile, crate::platform::SERVER_INSTRUCTIONS);
    loop {
        if cancelled.load(Ordering::Acquire) {
            return;
        }
        let secret = match std::fs::read_to_string(&secret_file) {
            Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => {
                eprintln!("switchboard: cannot read secret file {secret_file}; retrying");
                if !sleep_cancellable(backoff.next_ms(), &cancelled) {
                    return;
                }
                continue;
            }
        };
        let req = match build_request(&url, &secret) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("switchboard: {e}");
                return;
            }
        };
        eprintln!("switchboard: dialling {url} as {profile}");
        match tungstenite::connect(req) {
            Ok((mut socket, _resp)) => {
                match socket.get_mut() {
                    MaybeTlsStream::Plain(s) => {
                        let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
                    }
                    MaybeTlsStream::Rustls(s) => {
                        let _ = s.get_mut().set_read_timeout(Some(Duration::from_secs(1)));
                    }
                    _ => {}
                }
                let up_since = Instant::now();
                eprintln!("switchboard: attached");
                let close = serve(&mut socket, &profile, instr.as_deref(), &cancelled);
                if cancelled.load(Ordering::Acquire) {
                    let _ = socket.close(None);
                    return;
                }
                // Reset backoff only after a connection that stayed up long enough.
                if up_since.elapsed() >= Duration::from_secs(60) {
                    backoff.reset();
                }
                match close {
                    Close::Stop(reason) => {
                        eprintln!("switchboard: stopped ({reason})");
                        return;
                    }
                    Close::Redial => {}
                }
            }
            Err(e) => match handshake_status(&e).map(disposition_handshake) {
                Some(Handshake::RetrySlow) => {
                    eprintln!("switchboard: 401 (bad secret); retrying in 5 min");
                    if !sleep_cancellable(RETRY_SLOW_SECS * 1000, &cancelled) {
                        return;
                    }
                    continue;
                }
                _ => eprintln!("switchboard: dial failed: {e}; backing off"),
            },
        }
        if !sleep_cancellable(backoff.next_ms(), &cancelled) {
            return;
        }
    }
}

/// Serve MCP on an attached socket until it closes; returns what to do next.
fn serve(
    socket: &mut tungstenite::WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    profile: &str,
    instructions: Option<&str>,
    cancelled: &AtomicBool,
) -> Close {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Close::Redial;
        }
        match socket.read() {
            Ok(Message::Text(t)) => {
                if let Some(reply) = answer_with(t.as_str(), profile, instructions) {
                    if socket.send(Message::Text(reply)).is_err() {
                        return Close::Redial;
                    }
                }
            }
            Ok(Message::Binary(b)) => {
                let t = String::from_utf8_lossy(&b).to_string();
                if let Some(reply) = answer_with(&t, profile, instructions) {
                    if socket.send(Message::Text(reply)).is_err() {
                        return Close::Redial;
                    }
                }
            }
            Ok(Message::Ping(p)) => {
                let _ = socket.send(Message::Pong(p));
            }
            Ok(Message::Close(frame)) => {
                let _ = socket.flush();
                let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(1000);
                return disposition_close(code);
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(ref e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return Close::Redial,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn vectors() {
        // Same repo-root conformance dir as the reverse-attach vectors.
        const VECTORS: &str = include_str!("../../../conformance/switchboard_vectors.json");
        let v: Value = serde_json::from_str(VECTORS).expect("parse vectors");

        for case in v["validate_url"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            let ok = validate_url(input).is_ok();
            assert_eq!(ok, case["ok"].as_bool().unwrap(), "validate_url({input})");
        }
        for case in v["close"].as_array().unwrap() {
            let code = case["code"].as_u64().unwrap() as u16;
            let stop = matches!(disposition_close(code), Close::Stop(_));
            assert_eq!(stop, case["stop"].as_bool().unwrap(), "close {code}");
        }
        for case in v["handshake"].as_array().unwrap() {
            let status = case["status"].as_u64().unwrap() as u16;
            let got = match disposition_handshake(status) {
                Handshake::Attached => "attached",
                Handshake::RetrySlow => "retry_slow",
                Handshake::Redial => "redial",
            };
            assert_eq!(
                got,
                case["disposition"].as_str().unwrap(),
                "handshake {status}"
            );
        }
    }

    #[test]
    fn backoff_grows_and_caps() {
        let mut b = Backoff::new();
        let a = b.next_ms(); // base 1
        let c = b.next_ms(); // base 2
        assert!((800..=1200).contains(&a), "{a}");
        assert!((1600..=2400).contains(&c), "{c}");
        for _ in 0..10 {
            b.next_ms();
        }
        let capped = b.next_ms();
        assert!(capped <= 72_000, "capped ~60s ±20%: {capped}");
    }
}
