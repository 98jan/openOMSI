//! The administration's web API and who may reach it (see SERVER.md, "The web panel and the
//! API"): `POST /api/admin/login` for a token, then with `Authorization: Bearer <token>` the
//! session, the server's state, its players, the operations and what became of them. The
//! web gateway's threads answer from the hub alone: they read what the server's loop
//! published and queue requests for it, and never touch the session or the world.
//!
//! Who may reach it (`WebCfg`): by default only this machine, directly - not through a tunnel
//! or a proxy on it, the server's own Cloudflare tunnel included. `admin_web = remote` opens it
//! to the proxies the operator names (`admin_trusted_proxies`, this machine by default), and
//! only for requests they say came over HTTPS; the address they say the request came from is
//! believed of them alone. No cookie is used (so no other site's page can act with an admin's
//! session); a request that changes something must be JSON and, when the browser names its
//! origin, from the server's own address or one of `admin_web_origins`. No answer carries
//! `Access-Control-Allow-Origin`: another site's page cannot read it.

use crate::auth::{LoginError, SessionInfo};
use crate::hub::{Actor, Hub, Origin};
use crate::op::Op;
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

/// Who may reach the panel and the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebMode {
    /// Nobody: they answer 404.
    Off,
    /// This machine, directly (the default).
    Local,
    /// This machine, and HTTPS through the trusted proxies.
    Remote,
}

impl WebMode {
    pub fn parse(s: &str) -> Option<WebMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "no" => Some(WebMode::Off),
            "" | "local" => Some(WebMode::Local),
            "remote" => Some(WebMode::Remote),
            _ => None,
        }
    }
}

/// The header in which the trusted proxy puts the client's address (`admin_proxy_header`):
/// only that one is believed - the others may be the client's own, passed through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyHeader {
    /// `X-Forwarded-For`, its last address (the one the proxy added): Caddy, nginx, Cloudflare.
    ForwardedFor,
    /// `X-Real-IP`, for a proxy set to write it.
    RealIp,
    /// `CF-Connecting-IP`: Cloudflare (its tunnel too).
    CfConnectingIp,
}

impl ProxyHeader {
    pub fn parse(s: &str) -> Option<ProxyHeader> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "x-forwarded-for" => Some(ProxyHeader::ForwardedFor),
            "x-real-ip" => Some(ProxyHeader::RealIp),
            "cf-connecting-ip" => Some(ProxyHeader::CfConnectingIp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebCfg {
    pub mode: WebMode,
    /// The origins (`https://admin.example.org`) a page may act from besides the server's own.
    pub origins: Vec<String>,
    /// The proxies whose word about a request's source and scheme is believed.
    pub trusted_proxies: Vec<IpAddr>,
    /// Where they say it.
    pub proxy_header: ProxyHeader,
    /// The highest traffic density (for the operations' checks).
    pub traffic_max: usize,
}

impl Default for WebCfg {
    fn default() -> WebCfg {
        WebCfg { mode: WebMode::Local, origins: Vec::new(), trusted_proxies: vec![IpAddr::from([127, 0, 0, 1]), IpAddr::from(std::net::Ipv6Addr::LOCALHOST)], proxy_header: ProxyHeader::ForwardedFor, traffic_max: 120 }
    }
}

/// A request as the gateway read it.
pub struct Request<'a> {
    pub method: &'a str,
    /// The path without the query.
    pub path: &'a str,
    pub query: &'a str,
    /// The header lines (`Name: value`, one a line).
    pub headers: &'a str,
    pub body: &'a [u8],
    /// The TCP peer.
    pub peer: Option<SocketAddr>,
}

impl Request<'_> {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
    }

    /// A header's last line (a proxy adds its own after the client's).
    fn last_header(&self, name: &str) -> Option<&str> {
        self.headers.lines().filter_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())).last()
    }

    fn param(&self, name: &str) -> Option<String> {
        self.query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| url_decode(v)))
    }
}

/// `%xx` and `+` of a query value (a broken escape kept as it is).
fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hexv = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match (hexv(b[i + 1]), hexv(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push(h * 16 + l);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: &'static str,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// Besides the gateway's own (`Cache-Control: no-store`, `Connection: close`).
    pub headers: Vec<(&'static str, String)>,
}

/// The headers every answer of the administration carries.
fn guarded(mut r: Response) -> Response {
    r.headers.push(("X-Content-Type-Options", "nosniff".into()));
    r.headers.push(("Referrer-Policy", "no-referrer".into()));
    r.headers.push(("X-Frame-Options", "DENY".into()));
    r
}

fn json_answer(status: &'static str, v: Value) -> Response {
    guarded(Response { status, content_type: "application/json", body: v.to_string().into_bytes(), headers: Vec::new() })
}

fn error(status: &'static str, msg: impl Into<String>) -> Response {
    json_answer(status, json!({ "error": msg.into() }))
}

/// Where a request may come from, and from whom it says it comes: the source to count wrong
/// passwords by and to name in the records - or the answer that turns it away.
fn admitted(req: &Request, cfg: &WebCfg) -> Result<IpAddr, Response> {
    let Some(peer) = req.peer else { return Err(error("403 Forbidden", "no peer address")) };
    let proxied = ["cf-connecting-ip", "cf-ray", "x-forwarded-for", "forwarded", "x-real-ip", "x-forwarded-proto"].iter().any(|h| req.header(h).is_some());
    match cfg.mode {
        WebMode::Off => Err(error("404 Not Found", "the web administration is off on this server (admin_web = off)")),
        // (a page of another site whose name was turned to this machine - DNS rebinding -
        // still names its own site)
        _ if peer.ip().is_loopback() && !proxied && !local_host(req.header("host").unwrap_or("")) => Err(error("403 Forbidden", "reach the administration as 127.0.0.1 or localhost on this machine")),
        _ if peer.ip().is_loopback() && !proxied => Ok(peer.ip()),
        WebMode::Local => Err(error("403 Forbidden", "the administration is reached from the server's own machine only (admin_web = local), not through a tunnel or a proxy")),
        WebMode::Remote => {
            if !cfg.trusted_proxies.contains(&peer.ip()) {
                return Err(error("403 Forbidden", "only through the HTTPS proxy the server trusts (admin_trusted_proxies)"));
            }
            let https = req.header("x-forwarded-proto").is_some_and(|p| p.eq_ignore_ascii_case("https")) || req.header("cf-visitor").is_some_and(|v| v.replace(' ', "").contains("\"scheme\":\"https\""));
            if !https {
                return Err(error("403 Forbidden", "the administration is reached over HTTPS only"));
            }
            // (only the header the proxy writes: another one may be the client's own)
            let said = match cfg.proxy_header {
                ProxyHeader::ForwardedFor => req.last_header("x-forwarded-for").and_then(|v| v.rsplit(',').next()),
                ProxyHeader::RealIp => req.last_header("x-real-ip"),
                ProxyHeader::CfConnectingIp => req.last_header("cf-connecting-ip"),
            };
            said.and_then(|v| v.trim().parse::<IpAddr>().ok()).ok_or_else(|| error("403 Forbidden", "the proxy did not say whose request it is (admin_proxy_header)"))
        }
    }
}

/// The machine's own names, with or without a port.
fn local_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    let name = if h.starts_with('[') { h.split(']').next().map(|n| format!("{n}]")).unwrap_or_default() } else { h.split(':').next().unwrap_or("").to_string() };
    matches!(name.as_str(), "127.0.0.1" | "localhost" | "[::1]")
}

/// A request that changes something comes from a page of the server's own address (or one
/// allowed), as JSON - which a page of another site cannot send without asking first.
fn same_origin(req: &Request, cfg: &WebCfg) -> Result<(), Response> {
    let json_body = req.header("content-type").is_some_and(|c| c.to_ascii_lowercase().starts_with("application/json"));
    if !json_body {
        return Err(error("415 Unsupported Media Type", "send JSON (Content-Type: application/json)"));
    }
    let Some(origin) = req.header("origin") else { return Ok(()) };
    let host = req.header("x-forwarded-host").filter(|_| cfg.mode == WebMode::Remote).or_else(|| req.header("host")).unwrap_or("");
    let own = ["http://", "https://"].iter().any(|s| origin.eq_ignore_ascii_case(&format!("{s}{host}")));
    if own || cfg.origins.iter().any(|o| o.trim_end_matches('/').eq_ignore_ascii_case(origin)) {
        Ok(())
    } else {
        Err(error("403 Forbidden", "a request from another site's page"))
    }
}

fn bearer<'a>(req: &'a Request) -> Option<&'a str> {
    let h = req.header("authorization")?;
    let (scheme, token) = h.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// The session of the request's token, or the answer that turns it away.
fn session(hub: &Hub, req: &Request) -> Result<SessionInfo, Response> {
    let token = bearer(req).ok_or_else(|| error("401 Unauthorized", "log in first"))?;
    hub.auth().check(token, Instant::now()).ok_or_else(|| error("401 Unauthorized", "the session has ended: log in again"))
}

fn age_ms(at: Option<Instant>) -> Value {
    at.map(|t| json!(t.elapsed().as_millis() as u64)).unwrap_or(Value::Null)
}

/// Answer an administration request (None: a path that is not the administration's).
pub fn handle(hub: &Hub, cfg: &WebCfg, req: &Request) -> Option<Response> {
    if !req.path.starts_with("/api/admin/") && req.path != "/api/admin" {
        return None;
    }
    let source = match admitted(req, cfg) {
        Ok(s) => s,
        Err(r) => return Some(r),
    };
    let post = req.method == "POST";
    if post {
        if let Err(r) = same_origin(req, cfg) {
            return Some(r);
        }
    }
    let body: Value = if post { serde_json::from_slice(req.body).unwrap_or(Value::Null) } else { Value::Null };
    Some(match (req.method, req.path) {
        ("POST", "/api/admin/login") => login(hub, source, &body),
        ("POST", "/api/admin/logout") => {
            if let Some(t) = bearer(req) {
                let info = hub.auth().check(t, Instant::now());
                if hub.auth().logout(t) {
                    hub.note(&info.as_ref().map(|i| i.actor.clone()).unwrap_or_default(), info.map(|i| i.role), "logout", "ok", "");
                }
            }
            json_answer("200 OK", json!({ "ok": true }))
        }
        ("GET", path) => match session(hub, req) {
            Err(r) => r,
            Ok(s) => get(hub, req, &s, path),
        },
        ("POST", "/api/admin/ops") => match session(hub, req) {
            Err(r) => r,
            Ok(s) => submit(hub, cfg, &s, &body),
        },
        ("POST", _) => error("404 Not Found", "no such request"),
        _ => error("405 Method Not Allowed", "GET or POST"),
    })
}

fn login(hub: &Hub, source: IpAddr, body: &Value) -> Response {
    let password = body.get("password").and_then(Value::as_str).unwrap_or("");
    let r = hub.auth().login(password, source, Instant::now());
    match r {
        Ok((token, info)) => {
            hub.note(&info.actor, Some(info.role), "login", "ok", "web");
            json_answer("200 OK", json!({ "token": token, "session": info }))
        }
        Err(e) => {
            // (a wrong password noted each, a lock once: knocking on it fills nothing; no
            // password set, nothing to note)
            if matches!(e, LoginError::Wrong | LoginError::Full | LoginError::Locked { first: true, .. }) {
                hub.note(&format!("web ({source})"), None, "login", "refused", &e.message());
            }
            match e {
                LoginError::Disabled => error("404 Not Found", e.message()),
                LoginError::Locked { wait, .. } => json_answer("429 Too Many Requests", json!({ "error": e.message(), "retry_after": wait })),
                LoginError::Wrong => error("401 Unauthorized", e.message()),
                LoginError::Full => error("503 Service Unavailable", e.message()),
            }
        }
    }
}

fn get(hub: &Hub, req: &Request, s: &SessionInfo, path: &str) -> Response {
    let snap = hub.snapshot();
    match path {
        "/api/admin/session" => json_answer("200 OK", json!({ "session": s })),
        "/api/admin/state" => json_answer("200 OK", json!({ "session": s, "server": snap.server, "age_ms": age_ms(snap.at) })),
        "/api/admin/players" => {
            let q = req.param("q").unwrap_or_default();
            let found: Vec<&crate::players::Player> = crate::players::search(&snap.players, &q);
            json_answer("200 OK", json!({ "players": found, "total": snap.players.len(), "age_ms": age_ms(snap.at) }))
        }
        "/api/admin/events" => {
            let after = req.param("after").and_then(|v| v.parse().ok()).unwrap_or(0);
            let audit_after = req.param("audit_after").and_then(|v| v.parse().ok()).unwrap_or(0);
            json_answer("200 OK", json!({ "ops": hub.recent(50), "audit": hub.audit_since(audit_after, 100), "after": after }))
        }
        p if p.starts_with("/api/admin/ops/") => match p["/api/admin/ops/".len()..].parse::<u64>().ok().and_then(|id| hub.record(id)) {
            Some(r) => json_answer("200 OK", json!({ "op": r })),
            None => error("404 Not Found", "no such operation (or no longer kept: see the audit log)"),
        },
        _ => error("404 Not Found", "no such request"),
    }
}

fn submit(hub: &Hub, cfg: &WebCfg, s: &SessionInfo, body: &Value) -> Response {
    let op = match Op::from_json(body, cfg.traffic_max) {
        Ok(op) => op,
        Err(e) => return error("400 Bad Request", e),
    };
    // (the role is the session's, never the request's)
    let actor = Actor { label: s.actor.clone(), role: s.role, origin: Origin::Web };
    match hub.submit(actor, op) {
        Ok(rec) => json_answer("202 Accepted", json!({ "op": rec })),
        Err(e) if e.contains("may not") => error("403 Forbidden", e),
        Err(e) => error("503 Service Unavailable", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::Audit;
    use crate::auth::{Auth, AuthCfg, Passwords};
    use crate::hub::{Outcome, Snapshot, Status};
    use crate::players::{Player, PlayerState};

    fn hub() -> Hub {
        let h = Hub::new(Auth::new(Passwords { admin: "admin secret".into(), moderator: "mod secret".into() }, AuthCfg::default()), Audit::new(None, 0));
        let p = |id: u32, conn: u64, name: &str| Player { id, conn, name: name.into(), state: PlayerState::Driving, vehicle: "Vehicles/A/a.bus".into(), line: String::new(), tour: String::new(), aboard: None, online_secs: 60, admin: None };
        h.publish(Snapshot { players: vec![p(2, 5, "Pichau"), p(3, 6, "pichau_fan"), p(4, 7, "Joao")], at: Some(Instant::now()), ..Default::default() });
        h
    }

    const HERE: &str = "127.0.0.1:50000";

    struct R {
        method: &'static str,
        path: String,
        query: String,
        headers: String,
        body: Vec<u8>,
        peer: SocketAddr,
    }

    impl R {
        fn get(path: &str) -> R {
            let (p, q) = path.split_once('?').unwrap_or((path, ""));
            R { method: "GET", path: p.into(), query: q.into(), headers: "Host: 127.0.0.1:27025\r\n".into(), body: Vec::new(), peer: HERE.parse().unwrap() }
        }
        fn post(path: &str, body: Value) -> R {
            R { method: "POST", path: path.into(), query: String::new(), headers: "Host: 127.0.0.1:27025\r\nContent-Type: application/json\r\n".into(), body: body.to_string().into_bytes(), peer: HERE.parse().unwrap() }
        }
        fn with(mut self, h: &str) -> R {
            self.headers.push_str(h);
            self.headers.push_str("\r\n");
            self
        }
        fn token(self, t: &str) -> R {
            self.with(&format!("Authorization: Bearer {t}"))
        }
        fn from(mut self, peer: &str) -> R {
            self.peer = peer.parse().unwrap();
            self
        }
        fn send(&self, hub: &Hub, cfg: &WebCfg) -> (String, Value) {
            let req = Request { method: self.method, path: &self.path, query: &self.query, headers: &self.headers, body: &self.body, peer: Some(self.peer) };
            let r = handle(hub, cfg, &req).expect("an administration page");
            assert!(!r.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("Access-Control-Allow-Origin")));
            (r.status.to_string(), serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        }
    }

    fn login(hub: &Hub, cfg: &WebCfg, pw: &str) -> String {
        let (st, v) = R::post("/api/admin/login", json!({ "password": pw })).send(hub, cfg);
        assert_eq!(st, "200 OK", "{v}");
        v["token"].as_str().unwrap().to_string()
    }

    #[test]
    fn nothing_without_a_session() {
        let (h, cfg) = (hub(), WebCfg::default());
        for path in ["/api/admin/state", "/api/admin/players", "/api/admin/events", "/api/admin/session", "/api/admin/ops/1"] {
            assert_eq!(R::get(path).send(&h, &cfg).0, "401 Unauthorized", "{path}");
            assert_eq!(R::get(path).token("not-a-token").send(&h, &cfg).0, "401 Unauthorized", "{path}");
        }
        assert_eq!(R::post("/api/admin/ops", json!({"op": "say", "text": "x"})).send(&h, &cfg).0, "401 Unauthorized");
        // the password is no token, and a token in the address is no session
        assert_eq!(R::get("/api/admin/state").token("admin secret").send(&h, &cfg).0, "401 Unauthorized");
        let t = login(&h, &cfg, "admin secret");
        assert_eq!(R::get(&format!("/api/admin/state?token={t}")).send(&h, &cfg).0, "401 Unauthorized");
        assert_eq!(R::get("/api/admin/state").token(&t).send(&h, &cfg).0, "200 OK");
        // not the administration's
        assert!(handle(&h, &cfg, &Request { method: "GET", path: "/status", query: "", headers: "", body: &[], peer: Some(HERE.parse().unwrap()) }).is_none());
    }

    #[test]
    fn a_login_its_lock_and_its_end() {
        let (h, cfg) = (hub(), WebCfg::default());
        for _ in 0..5 {
            assert_eq!(R::post("/api/admin/login", json!({ "password": "nope" })).send(&h, &cfg).0, "401 Unauthorized");
        }
        let (st, v) = R::post("/api/admin/login", json!({ "password": "admin secret" })).send(&h, &cfg);
        assert_eq!(st, "429 Too Many Requests");
        assert!(v["retry_after"].as_u64().unwrap() > 0);
        // (the lock is per source: another way in still works - here a remote one through the proxy)
        let remote = WebCfg { mode: WebMode::Remote, ..WebCfg::default() };
        let (st, v) = R::post("/api/admin/login", json!({ "password": "mod secret" })).with("X-Forwarded-Proto: https").with("X-Forwarded-For: 203.0.113.9").send(&h, &remote);
        assert_eq!(st, "200 OK", "{v}");
        assert_eq!(v["session"]["role"], "moderator");
        assert!(v["session"]["actor"].as_str().unwrap().contains("203.0.113.9"));
        let t = v["token"].as_str().unwrap().to_string();
        assert_eq!(R::post("/api/admin/logout", json!({})).token(&t).with("X-Forwarded-Proto: https").with("X-Forwarded-For: 203.0.113.9").send(&h, &remote).0, "200 OK");
        assert_eq!(R::get("/api/admin/session").token(&t).with("X-Forwarded-Proto: https").with("X-Forwarded-For: 203.0.113.9").send(&h, &remote).0, "401 Unauthorized");
        // the log has the logins and the refusals, and no password
        let audit = h.audit_since(0, 100);
        assert!(audit.iter().any(|e| e.action == "login" && e.result == "refused"));
        assert!(audit.iter().any(|e| e.action == "logout"));
        assert!(!format!("{audit:?}").contains("secret"));
    }

    /// The default: this machine only, never through a tunnel or a proxy on it.
    #[test]
    fn local_by_default() {
        let (h, cfg) = (hub(), WebCfg::default());
        assert_eq!(R::post("/api/admin/login", json!({ "password": "admin secret" })).from("192.0.2.7:4000").send(&h, &cfg).0, "403 Forbidden");
        for proxy in ["CF-Connecting-IP: 203.0.113.9", "X-Forwarded-For: 203.0.113.9", "X-Real-IP: 203.0.113.9", "Forwarded: for=203.0.113.9", "X-Forwarded-Proto: https"] {
            assert_eq!(R::post("/api/admin/login", json!({ "password": "admin secret" })).with(proxy).send(&h, &cfg).0, "403 Forbidden", "{proxy}");
        }
        let off = WebCfg { mode: WebMode::Off, ..WebCfg::default() };
        assert_eq!(R::post("/api/admin/login", json!({ "password": "admin secret" })).send(&h, &off).0, "404 Not Found");
    }

    /// Remote: HTTPS through a trusted proxy only; its word about the source believed of it alone.
    #[test]
    fn remote_through_the_trusted_proxy_over_https() {
        let h = hub();
        let cfg = WebCfg { mode: WebMode::Remote, trusted_proxies: vec!["10.0.0.1".parse().unwrap()], ..WebCfg::default() };
        let pw = json!({ "password": "admin secret" });
        // not the proxy (even saying https), or the proxy without https
        assert_eq!(R::post("/api/admin/login", pw.clone()).from("192.0.2.7:1").with("X-Forwarded-Proto: https").send(&h, &cfg).0, "403 Forbidden");
        assert_eq!(R::post("/api/admin/login", pw.clone()).from("10.0.0.1:1").send(&h, &cfg).0, "403 Forbidden");
        assert_eq!(R::post("/api/admin/login", pw.clone()).from("10.0.0.1:1").with("X-Forwarded-Proto: http").send(&h, &cfg).0, "403 Forbidden");
        // this machine directly, still
        assert_eq!(R::post("/api/admin/login", pw.clone()).send(&h, &cfg).0, "200 OK");
        // the proxy, over https (Cloudflare's way of saying so too), saying whose request it is
        assert_eq!(R::post("/api/admin/login", pw.clone()).from("10.0.0.1:1").with("X-Forwarded-Proto: https").with("X-Forwarded-For: 203.0.113.9").send(&h, &cfg).0, "200 OK");
        assert_eq!(R::post("/api/admin/login", pw.clone()).from("10.0.0.1:1").with("CF-Visitor: {\"scheme\":\"https\"}").with("X-Forwarded-For: 203.0.113.9").send(&h, &cfg).0, "200 OK");
        // a proxy that does not say whose: turned away (not taken for the proxy itself)
        assert_eq!(R::post("/api/admin/login", pw).from("10.0.0.1:1").with("X-Forwarded-Proto: https").send(&h, &cfg).0, "403 Forbidden");
    }

    /// Only the header the proxy writes names the source: what the client sends through it
    /// (a CF-Connecting-IP behind Caddy, the first X-Forwarded-For addresses) is not believed -
    /// a new one on each try would get around the lock and lie in the records.
    #[test]
    fn the_source_is_the_proxys_word_alone() {
        let h = hub();
        let cfg = WebCfg { mode: WebMode::Remote, trusted_proxies: vec!["10.0.0.1".parse().unwrap()], ..WebCfg::default() };
        let wrong = |n: u8| R::post("/api/admin/login", json!({ "password": "nope" })).from("10.0.0.1:1").with("X-Forwarded-Proto: https").with(&format!("CF-Connecting-IP: 198.51.100.{n}")).with(&format!("X-Forwarded-For: 198.51.100.{n}, 203.0.113.9"));
        for n in 0..5 {
            assert_eq!(wrong(n).send(&h, &cfg).0, "401 Unauthorized");
        }
        // (all from 203.0.113.9, whatever else they said: locked)
        assert_eq!(wrong(9).send(&h, &cfg).0, "429 Too Many Requests");
        assert!(h.audit_since(0, 100).iter().all(|e| e.actor.contains("203.0.113.9")), "{:?}", h.audit_since(0, 100));
        // a proxy set to write X-Real-IP (or CF-Connecting-IP) is believed of that one
        let real = WebCfg { proxy_header: ProxyHeader::RealIp, ..cfg.clone() };
        let (st, v) = R::post("/api/admin/login", json!({ "password": "admin secret" })).from("10.0.0.1:1").with("X-Forwarded-Proto: https").with("X-Forwarded-For: 203.0.113.9").with("X-Real-IP: 192.0.2.44").send(&h, &real);
        assert_eq!(st, "200 OK", "{v}");
        assert!(v["session"]["actor"].as_str().unwrap().contains("192.0.2.44"), "{v}");
        assert_eq!(ProxyHeader::parse("CF-Connecting-IP"), Some(ProxyHeader::CfConnectingIp));
        assert_eq!(ProxyHeader::parse(""), Some(ProxyHeader::ForwardedFor));
        assert_eq!(ProxyHeader::parse("forwarded"), None);
    }

    /// A page of another site whose name was turned to this machine (DNS rebinding) still
    /// names that site: this machine's own names only.
    #[test]
    fn this_machine_by_its_own_names() {
        let (h, cfg) = (hub(), WebCfg::default());
        let pw = || R::post("/api/admin/login", json!({ "password": "admin secret" }));
        let as_host = |host: &str| {
            let mut r = pw();
            r.headers = r.headers.replace("Host: 127.0.0.1:27025", &format!("Host: {host}"));
            r
        };
        for host in ["127.0.0.1:27025", "localhost:27025", "LOCALHOST", "[::1]:27025", "127.0.0.1"] {
            assert_eq!(as_host(host).send(&h, &cfg).0, "200 OK", "{host}");
        }
        for host in ["evil.example:27025", "127.0.0.1.evil.example", "", "localhost.evil.example:27025"] {
            assert_eq!(as_host(host).send(&h, &cfg).0, "403 Forbidden", "{host}");
        }
    }

    /// Knocking on a lock is noted once, not each time: a flood of logins does not push the
    /// real history out of the audit log.
    #[test]
    fn a_lock_is_noted_once() {
        let (h, cfg) = (hub(), WebCfg::default());
        for _ in 0..200 {
            let _ = R::post("/api/admin/login", json!({ "password": "nope" })).send(&h, &cfg);
        }
        let refused = h.audit_since(0, 1000).into_iter().filter(|e| e.action == "login" && e.result == "refused").count();
        // (the 5 wrong passwords, then the lock once)
        assert_eq!(refused, 6);
    }

    /// A page of another site cannot act: JSON only, and its origin refused.
    #[test]
    fn no_request_from_another_sites_page() {
        let (h, cfg) = (hub(), WebCfg::default());
        let t = login(&h, &cfg, "admin secret");
        let say = || R::post("/api/admin/ops", json!({"op": "say", "text": "hi"})).token(&t);
        assert_eq!(say().with("Origin: https://evil.example").send(&h, &cfg).0, "403 Forbidden");
        assert_eq!(say().with("Origin: http://127.0.0.1:27025").send(&h, &cfg).0, "202 Accepted");
        let mut form = say();
        form.headers = form.headers.replace("application/json", "application/x-www-form-urlencoded");
        assert_eq!(form.send(&h, &cfg).0, "415 Unsupported Media Type");
        let allowed = WebCfg { origins: vec!["https://admin.example.org/".into()], ..WebCfg::default() };
        assert_eq!(say().with("Origin: https://admin.example.org").send(&h, &allowed).0, "202 Accepted");
    }

    /// The role decides on the server, whatever a page shows or sends; an operation is checked
    /// and queued, and its record tells what became of it.
    #[test]
    fn operations_by_role_and_their_records() {
        let (h, cfg) = (hub(), WebCfg::default());
        let m = login(&h, &cfg, "mod secret");
        let (st, v) = R::post("/api/admin/ops", json!({"op": "clock", "seconds": 30600})).token(&m).send(&h, &cfg);
        assert_eq!(st, "403 Forbidden", "{v}");
        // a role in the request is nothing
        let (st, _) = R::post("/api/admin/ops", json!({"op": "clock", "seconds": 30600, "role": "admin"})).token(&m).send(&h, &cfg);
        assert_eq!(st, "403 Forbidden");
        let (st, _) = R::post("/api/admin/ops", json!({"op": "promote"})).token(&m).send(&h, &cfg);
        assert_eq!(st, "400 Bad Request");
        assert!(h.take(10).is_empty());
        let (st, v) = R::post("/api/admin/ops", json!({"op": "kick", "target": {"id": 2, "conn": 5}, "reason": "<b>spam</b>"})).token(&m).send(&h, &cfg);
        assert_eq!(st, "202 Accepted");
        assert_eq!(v["op"]["status"], "queued");
        assert_eq!(v["op"]["target_name"], "Pichau (#2)");
        let id = v["op"]["id"].as_u64().unwrap();
        // the loop takes it and says what became of it
        let job = h.take(10).remove(0);
        assert_eq!(job.id, id);
        h.finish(job.id, &job.actor, &Outcome::applied("Pichau (#2) sent away"));
        let (st, v) = R::get(&format!("/api/admin/ops/{id}")).token(&m).send(&h, &cfg);
        assert_eq!((st.as_str(), v["op"]["status"].as_str()), ("200 OK", Some(Status::Applied.name())));
        let (_, ev) = R::get("/api/admin/events").token(&m).send(&h, &cfg);
        assert!(ev["ops"].as_array().unwrap().iter().any(|o| o["id"] == id && o["status"] == "applied"));
        assert_eq!(R::get("/api/admin/ops/999").token(&m).send(&h, &cfg).0, "404 Not Found");
    }

    /// The admins' list needs no share_positions, gives no position or address, and searches.
    #[test]
    fn the_players_for_the_admins() {
        let (h, cfg) = (hub(), WebCfg::default());
        let t = login(&h, &cfg, "mod secret");
        let (st, v) = R::get("/api/admin/players?q=PICHAU").token(&t).send(&h, &cfg);
        assert_eq!(st, "200 OK");
        let names: Vec<&str> = v["players"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["Pichau", "pichau_fan"]);
        assert_eq!(v["total"], 3);
        let p = &v["players"][0];
        assert_eq!((p["id"].as_u64(), p["conn"].as_u64()), (Some(2), Some(5)));
        assert!(p.get("x").is_none() && p.get("addr").is_none() && p.get("ip").is_none());
        let (_, v) = R::get("/api/admin/players?q=pichau%5Ffan").token(&t).send(&h, &cfg);
        assert_eq!(v["players"].as_array().unwrap().len(), 1);
        let (_, v) = R::get("/api/admin/players?q=.%2A").token(&t).send(&h, &cfg);
        assert_eq!(v["players"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_query_value_is_decoded() {
        assert_eq!(url_decode("a+b%20c%2Fd"), "a b c/d");
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz%4"), "%zz%4");
        assert_eq!(url_decode("%C3%A9"), "é");
    }
}
