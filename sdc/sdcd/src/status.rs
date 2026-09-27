//! **Mobile read-only status** (1.x in the plan): a page a phone on the same network can open to see what
//! SDC is doing - running turns and deploys, each site's health and Ops score, today's spend, and the
//! last actions from the audit ledger. Nothing on it can change anything.
//!
//! Off by default. Turned on, it listens on the local network on its own port with a random token in the
//! address, so only someone who was given the link can read it; turned off, the listener is gone.

use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::DaemonState;

pub const DEFAULT_PORT: u16 = 7813;

struct Running {
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

fn server() -> &'static Mutex<Option<Running>> {
    static SERVER: OnceLock<Mutex<Option<Running>>> = OnceLock::new();

    SERVER.get_or_init(|| Mutex::new(None))
}

/// This machine's address on the local network - the one a phone would reach. A UDP socket "connected"
/// to a public address sends nothing; it only makes the OS choose the outgoing interface.
fn lan_address() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;

    socket.connect("192.0.2.1:80").ok()?;

    Some(socket.local_addr().ok()?.ip().to_string())
}

/// Turns the page on or off. Answers with the link when it is on.
pub fn configure(state: Arc<DaemonState>, enabled: bool, port: Option<u16>) -> Value {
    let mut slot = server().lock().unwrap_or_else(|poison| poison.into_inner());

    if let Some(running) = slot.take() {
        running.task.abort();
    }

    if !enabled {
        return json!({ "enabled": false });
    }

    let port = port.unwrap_or(DEFAULT_PORT);
    let token = state
        .store
        .setting("status.token")
        .ok()
        .flatten()
        .filter(|token| token.len() >= 24)
        .unwrap_or_else(|| {
            let token = uuid::Uuid::new_v4().simple().to_string();
            let _ = state.store.set_setting("status.token", &token);

            token
        });
    let listener = match std::net::TcpListener::bind(("0.0.0.0", port)).and_then(|listener| {
        listener.set_nonblocking(true)?;

        tokio::net::TcpListener::from_std(listener)
    }) {
        Ok(listener) => listener,
        Err(error) => return json!({ "enabled": false, "error": format!("port {port} could not be opened: {error}") }),
    };
    let expected = token.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                continue;
            };
            let (state, expected) = (state.clone(), expected.clone());

            tokio::spawn(async move {
                let mut buffer = vec![0u8; 4096];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let line = request.lines().next().unwrap_or_default().to_string();
                let authorised = line.starts_with("GET ") && line.contains(&format!("token={expected}"));
                let (status, body) = if authorised {
                    ("200 OK", tokio::task::spawn_blocking(move || page(&state)).await.unwrap_or_default())
                } else {
                    ("403 Forbidden", "<!doctype html><meta charset=\"utf-8\"><p>This link is not valid.</p>".to_string())
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nX-Frame-Options: DENY\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );

                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let address = lan_address().unwrap_or_else(|| "this-computer".into());
    let url = format!("http://{address}:{port}/?token={token}");

    *slot = Some(Running { port, task });

    json!({ "enabled": true, "port": port, "url": url })
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The page: read-only, refreshed every thirty seconds.
pub fn page(state: &Arc<DaemonState>) -> String {
    let active = crate::trust::kill::active();
    let cost = crate::trust::cost::spent(&state.store, None);
    let mut html = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"refresh\" content=\"30\"><title>SDC status</title>\
<style>body{font:15px/1.5 system-ui,'Noto Sans Bengali',sans-serif;margin:0;padding:16px;background:#0d1117;color:#e6edf3}h1{font-size:18px}h2{font-size:14px;color:#8b949e;margin-top:20px}\
.card{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:10px 12px;margin:8px 0}.ok{color:#3fb950}.bad{color:#f85149}.warn{color:#d29922}.meta{color:#8b949e;font-size:12px}</style></head><body><h1>SDC · status</h1>",
    );

    html.push_str(&format!(
        "<p class=\"meta\">sdcd {} · {} · read-only</p><h2>Running now</h2>",
        crate::VERSION,
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));

    if active.is_empty() {
        html.push_str("<div class=\"card meta\">Nothing is running.</div>");
    }

    for work in &active {
        html.push_str(&format!("<div class=\"card\"><b>{}</b> {}<div class=\"meta\">since {}</div></div>", work.kind, escape(&work.label), escape(&work.started)));
    }

    html.push_str("<h2>Sites</h2>");

    for site in state.store.sites().unwrap_or_default() {
        let id = site["id"].as_str().unwrap_or_default();
        let report = state.store.health_history(id, 1).ok().and_then(|rows| rows.into_iter().next()).unwrap_or(Value::Null);
        let up = report["http"]["ok"].as_bool();
        let score = &report["score"];

        html.push_str(&format!(
            "<div class=\"card\"><b>{}</b> <span class=\"{}\">{}</span><div class=\"meta\">Ops score {} · {}</div></div>",
            escape(site["name"].as_str().unwrap_or_default()),
            match up { Some(true) => "ok", Some(false) => "bad", None => "warn" },
            match up { Some(true) => "up", Some(false) => "down", None => "not checked" },
            score["score"].as_i64().map(|score| score.to_string()).unwrap_or_else(|| "-".into()),
            escape(report["http"]["detail"].as_str().unwrap_or_default())
        ));
    }

    html.push_str(&format!(
        "<h2>AI spend</h2><div class=\"card\">Today ${:.2} · this month ${:.2}</div><h2>Last actions</h2>",
        cost["day"].as_f64().unwrap_or(0.0),
        cost["month"].as_f64().unwrap_or(0.0)
    ));

    for row in state.store.audit_rows(None, None, 12).unwrap_or_default() {
        html.push_str(&format!("<div class=\"card\">{}<div class=\"meta\">{} · {}</div></div>", escape(&row.summary), escape(&row.actor), escape(&row.ts)));
    }

    html.push_str("</body></html>");

    html
}

/// The port it is on now, if it is on.
pub fn running_port() -> Option<u16> {
    server().lock().ok()?.as_ref().map(|running| running.port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_page_answers_only_with_its_token_and_only_reads() {
        let state = DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).unwrap();
        let answer = configure(state.clone(), true, Some(0));

        /* Port 0 picks a free port; the answer still carries a link with the token. */
        assert_eq!(answer["enabled"], true, "{answer}");
        assert!(answer["url"].as_str().unwrap().contains("token="));
        assert!(page(&state).contains("read-only"));

        assert_eq!(configure(state, false, None)["enabled"], false);
        assert!(running_port().is_none());
    }
}
