//! The live preview's window onto a real site (0.14.4).
//!
//! A site that says `X-Frame-Options: DENY` or `Content-Security-Policy: frame-ancestors 'none'` cannot
//! be shown in the Preview tab's frame - measured on the user's own site, which sends both, and which the
//! frame drew as a "blocked" icon however right its address was. Those headers protect a site from being
//! framed by a stranger's page; here the frame is the site owner's own editor, on their own machine.
//!
//! So the preview can go through this: a loopback-only proxy, one port per site, that fetches each
//! request from the site and hands back the answer with the headers that forbid framing taken out. Nothing
//! else is changed: the page, its cookies and its scripts are the site's own. It listens on `127.0.0.1`
//! only, and only for a site the window asked about.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::sdcp::envelope::ErrorObject;

/// Headers never passed on: the ones that forbid framing, the ones this proxy re-frames itself, and the
/// connection-level ones.
const DROPPED: &[&str] = &[
    "x-frame-options",
    "content-security-policy",
    "content-security-policy-report-only",
    "strict-transport-security",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "content-encoding",
];

fn proxies() -> &'static Mutex<HashMap<String, u16>> {
    static PROXIES: OnceLock<Mutex<HashMap<String, u16>>> = OnceLock::new();

    PROXIES.get_or_init(Default::default)
}

/// `https://example.com/pricing` → `(https://example.com, /pricing)`.
pub fn split_origin(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;

    if scheme != "https" && scheme != "http" {
        return None;
    }

    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };

    if authority.is_empty() {
        return None;
    }

    Some((format!("{scheme}://{authority}"), path.to_string()))
}

/// The proxied address for `url`: `http://127.0.0.1:<port><path>`, starting the proxy for its site the
/// first time.
pub fn open(url: &str) -> Result<String, ErrorObject> {
    let (origin, path) = split_origin(url).ok_or_else(|| ErrorObject::bad_request(format!("`{url}` is not an http(s) address")))?;
    let mut known = proxies().lock().map_err(|_| ErrorObject::internal("the preview proxy is unavailable"))?;

    if let Some(port) = known.get(&origin) {
        return Ok(format!("http://127.0.0.1:{port}{path}"));
    }

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| ErrorObject::internal(error.to_string()))?;
    let port = listener.local_addr().map_err(|error| ErrorObject::internal(error.to_string()))?.port();
    let serving = origin.clone();

    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let origin = serving.clone();

            std::thread::spawn(move || {
                let _ = serve(stream, &origin, port);
            });
        }
    });

    known.insert(origin, port);

    Ok(format!("http://127.0.0.1:{port}{path}"))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .redirects(0)
        .build()
}

/// One request: read it, ask the site, answer with the site's answer minus [`DROPPED`].
fn serve(mut stream: TcpStream, origin: &str, port: u16) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();

    reader.read_line(&mut line)?;

    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next().map(str::to_string), parts.next().map(str::to_string)) else {
        return Ok(());
    };

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut length = 0usize;

    loop {
        let mut header = String::new();

        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }

        if let Some((name, value)) = header.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();

            if name == "content-length" {
                length = value.parse().unwrap_or(0);
            }

            /* The site sees a request for itself: its own host, no loopback origin, and a body it does
               not have to decompress for us (this client does not decompress). */
            if !matches!(name.as_str(), "host" | "origin" | "referer" | "accept-encoding" | "connection" | "content-length") {
                headers.push((name, value));
            }
        }
    }

    let mut body = vec![0u8; length.min(32 * 1024 * 1024)];

    reader.read_exact(&mut body)?;

    let mut request = agent().request(&method, &format!("{origin}{target}"));

    for (name, value) in &headers {
        request = request.set(name, value);
    }

    let response = match if body.is_empty() { request.call() } else { request.send_bytes(&body) } {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(error) => {
            let text = format!("The preview could not reach {origin}: {error}");

            write!(
                stream,
                "HTTP/1.1 502 Bad Gateway\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            )?;

            return Ok(());
        }
    };

    let status = response.status();
    let status_text = response.status_text().to_string();
    let mut head = format!("HTTP/1.1 {status} {status_text}\r\n");
    let local = format!("http://127.0.0.1:{port}");

    for name in response.headers_names() {
        if DROPPED.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }

        for value in response.all(&name) {
            /* A redirect within the site stays inside the preview. */
            let value = if name.eq_ignore_ascii_case("location") { value.replace(origin, &local) } else { value.to_string() };
            /* A `Secure` cookie is not kept on http://127.0.0.1; without it the site's session still works. */
            let value = if name.eq_ignore_ascii_case("set-cookie") { value.replace("; Secure", "").replace(";Secure", "") } else { value };

            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }

    let mut content = Vec::new();

    response.into_reader().take(64 * 1024 * 1024).read_to_end(&mut content)?;

    head.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", content.len()));
    stream.write_all(head.as_bytes())?;
    stream.write_all(&content)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_splits_into_its_site_and_its_page() {
        assert_eq!(
            split_origin("https://example.com/pricing?x=1"),
            Some(("https://example.com".to_string(), "/pricing?x=1".to_string()))
        );
        assert_eq!(split_origin("https://example.com"), Some(("https://example.com".to_string(), "/".to_string())));
        assert_eq!(split_origin("file:///etc/passwd"), None);
    }

    /// The whole point: a site that forbids framing comes back without the headers that forbid it, and
    /// with everything else it said.
    #[test]
    fn the_headers_that_forbid_framing_are_taken_out() {
        let site = TcpListener::bind("127.0.0.1:0").unwrap();
        let site_port = site.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for mut stream in site.incoming().flatten() {
                let mut buffer = [0u8; 2048];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: frame-ancestors 'none'\r\nX-Kept: yes\r\nContent-Length: 5\r\n\r\nhello",
                );
            }
        });

        let proxied = open(&format!("http://127.0.0.1:{site_port}/page")).unwrap();
        let answer = agent().get(&proxied).call().unwrap();

        assert_eq!(answer.header("x-frame-options"), None);
        assert_eq!(answer.header("content-security-policy"), None);
        assert_eq!(answer.header("x-kept"), Some("yes"));
        assert_eq!(answer.into_string().unwrap(), "hello");
    }
}
