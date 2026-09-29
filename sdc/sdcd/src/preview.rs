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

/// What a live site answers for a page: its HTTP status, or `None` when it could not be asked (0.15.4).
///
/// The report: *"preview te sudu main domain load hoi… slug page load hoi nah"*. The preview did follow
/// the page, but the site is a built Next.js app, and a page whose source was just written is a 404 there
/// until the site is built and restarted. The window asks this to say so, instead of drawing a 404 and
/// leaving the person to wonder.
pub fn status(url: &str) -> Option<u16> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(6))
        .timeout_read(Duration::from_secs(8))
        .redirects(3)
        .build();

    match agent.get(url).call() {
        Ok(response) => Some(response.status()),
        Err(ureq::Error::Status(status, _)) => Some(status),
        Err(_) => None,
    }
}

/// The dev-server preview (0.15.4): the project's own `npm run dev`, started on the chat's host on a
/// loopback port and reached through the signed-in connection, so the page being written is on screen
/// as it is written - with its real styles and components, hot-reloaded - instead of the last build the
/// live site serves.
///
/// One POSIX-sh round trip per call, and every call is safe to repeat: the first starts the server,
/// the next ones report on it. It listens on `127.0.0.1` only, never on the host's public address.
pub fn dev_script(root_expr: &str, stop: bool) -> String {
    format!(
        r#"root={root_expr}
dir=""
for d in "$root" "$root"/*; do
  if [ -f "$d/package.json" ] && grep -q '"dev"[[:space:]]*:' "$d/package.json"; then dir="$d"; break; fi
done
if [ -z "$dir" ]; then echo "@@NODIR"; exit 0; fi
echo "@@DIR $dir"
run="$HOME/.sdc/run"; mkdir -p "$run"
key=$(printf %s "$dir" | cksum | cut -d' ' -f1)
pidf="$run/preview-$key.pid"; portf="$run/preview-$key.port"; log="$run/preview-$key.log"
alive=0
if [ -f "$pidf" ] && kill -0 "$(cat "$pidf")" 2>/dev/null; then alive=1; fi
if [ {stop} = 1 ]; then
  if [ $alive = 1 ]; then pid=$(cat "$pidf"); pkill -P "$pid" 2>/dev/null; kill -- -"$pid" 2>/dev/null || kill "$pid" 2>/dev/null; fi
  port=$(cat "$portf" 2>/dev/null)
  if [ -n "$port" ] && command -v fuser >/dev/null 2>&1; then fuser -k "$port/tcp" >/dev/null 2>&1; fi
  rm -f "$pidf" "$portf"; echo "@@STOPPED"; exit 0
fi
if [ $alive = 0 ]; then
  busy=$( (ss -ltnH 2>/dev/null || netstat -ltn 2>/dev/null) | awk '{{print $4}}')
  port=""
  p=3100
  while [ $p -le 3199 ]; do
    if ! printf '%s\n' "$busy" | grep -q ":$p\$"; then port=$p; break; fi
    p=$((p+1))
  done
  [ -z "$port" ] && port=3100
  extra=""
  if grep -q '"dev"[[:space:]]*:[[:space:]]*"[^"]*next dev' "$dir/package.json"; then extra="--hostname 127.0.0.1"; fi
  [ -s "$HOME/.nvm/nvm.sh" ] && . "$HOME/.nvm/nvm.sh" >/dev/null 2>&1
  cd "$dir" || exit 0
  if command -v setsid >/dev/null 2>&1; then
    setsid nohup npm run dev -- --port "$port" $extra >"$log" 2>&1 </dev/null &
  else
    nohup npm run dev -- --port "$port" $extra >"$log" 2>&1 </dev/null &
  fi
  echo $! >"$pidf"; echo "$port" >"$portf"
  echo "@@STARTED"
fi
port=$(cat "$portf" 2>/dev/null)
echo "@@PORT $port"
if kill -0 "$(cat "$pidf" 2>/dev/null)" 2>/dev/null; then echo "@@ALIVE 1"; else echo "@@ALIVE 0"; fi
code=$(curl -s -o /dev/null -m 4 -w '%{{http_code}}' "http://127.0.0.1:$port/" 2>/dev/null || wget -q -S -O /dev/null -T 4 "http://127.0.0.1:$port/" 2>&1 | awk '/HTTP\//{{print $2}}' | tail -n 1)
echo "@@CODE $code"
echo "@@LOG"
tail -n 12 "$log" 2>/dev/null
"#,
        stop = if stop { 1 } else { 0 }
    )
}

/// What [`dev_script`] reported.
#[derive(Debug, Default, PartialEq)]
pub struct DevReport {
    pub dir: Option<String>,
    pub port: Option<u16>,
    pub alive: bool,
    pub answering: bool,
    pub stopped: bool,
    pub log: String,
}

fn local_servers() -> &'static Mutex<HashMap<String, (std::process::Child, u16, std::path::PathBuf)>> {
    static SERVERS: OnceLock<Mutex<HashMap<String, (std::process::Child, u16, std::path::PathBuf)>>> = OnceLock::new();

    SERVERS.get_or_init(Default::default)
}

/// [`dev_script`] for a chat on this machine: the same folder rule, a child this daemon holds, and a
/// port asked of the OS. `localhost:<port>` is reachable here as it is.
pub fn dev_local(root: &std::path::Path, stop: bool) -> DevReport {
    let mut report = DevReport::default();
    let has_dev = |dir: &std::path::Path| {
        std::fs::read_to_string(dir.join("package.json")).is_ok_and(|text| {
            serde_json::from_str::<serde_json::Value>(&text).is_ok_and(|value| value["scripts"]["dev"].is_string())
        })
    };
    let mut dirs = vec![root.to_path_buf()];

    if let Ok(entries) = std::fs::read_dir(root) {
        let mut inner: Vec<_> = entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect();

        inner.sort();
        dirs.extend(inner);
    }

    let Some(dir) = dirs.into_iter().find(|dir| has_dev(dir)) else {
        return report;
    };
    let key = dir.to_string_lossy().to_string();
    let log = std::env::temp_dir().join(format!("sdc-preview-{:x}.log", key.bytes().fold(0u64, |hash, byte| hash.wrapping_mul(31).wrapping_add(byte as u64))));
    let Ok(mut servers) = local_servers().lock() else {
        return report;
    };

    report.dir = Some(key.clone());

    if stop {
        if let Some((child, _, _)) = servers.remove(&key) {
            if let Some(pid) = Some(child.id()) {
                crate::pty::kill_tree(pid);
            }
        }

        report.stopped = true;

        return report;
    }

    let running = servers.get_mut(&key).is_some_and(|(child, _, _)| matches!(child.try_wait(), Ok(None)));

    if !running {
        let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|listener| listener.local_addr()).map(|address| address.port()).unwrap_or(3100);
        let next = std::fs::read_to_string(dir.join("package.json")).is_ok_and(|text| text.contains("next dev"));
        let mut command = if cfg!(windows) {
            let mut command = std::process::Command::new("cmd");

            command.args(["/C", "npm", "run", "dev", "--", "--port", &port.to_string()]);
            command
        } else {
            let mut command = std::process::Command::new("npm");

            command.args(["run", "dev", "--", "--port", &port.to_string()]);
            command
        };

        if next {
            command.args(["--hostname", "127.0.0.1"]);
        }

        let file = std::fs::File::create(&log).ok();

        command
            .current_dir(&dir)
            .stdin(std::process::Stdio::null())
            .stdout(file.as_ref().and_then(|file| file.try_clone().ok()).map(std::process::Stdio::from).unwrap_or_else(std::process::Stdio::null))
            .stderr(file.map(std::process::Stdio::from).unwrap_or_else(std::process::Stdio::null));

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            command.creation_flags(0x0800_0000);
        }

        match command.spawn() {
            Ok(child) => {
                servers.insert(key.clone(), (child, port, log.clone()));
            }
            Err(error) => {
                report.log = format!("npm could not be started: {error}");

                return report;
            }
        }
    }

    if let Some((child, port, log)) = servers.get_mut(&key) {
        report.port = Some(*port);
        report.alive = matches!(child.try_wait(), Ok(None));
        report.answering = TcpStream::connect_timeout(&([127, 0, 0, 1], *port).into(), Duration::from_millis(400)).is_ok();

        let text = std::fs::read_to_string(log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();

        report.log = lines[lines.len().saturating_sub(12)..].join("\n");
    }

    report
}

pub fn parse_dev(stdout: &str) -> DevReport {
    let mut report = DevReport::default();
    let mut in_log = false;

    for line in stdout.lines() {
        if in_log {
            report.log.push_str(line);
            report.log.push('\n');
        } else if let Some(dir) = line.strip_prefix("@@DIR ") {
            report.dir = Some(dir.trim().to_string());
        } else if let Some(port) = line.strip_prefix("@@PORT ") {
            report.port = port.trim().parse().ok();
        } else if let Some(alive) = line.strip_prefix("@@ALIVE ") {
            report.alive = alive.trim() == "1";
        } else if let Some(code) = line.strip_prefix("@@CODE ") {
            /* Any HTTP answer at all - a 404 or a 500 included - is a server that is up. */
            report.answering = code.trim().parse::<u16>().is_ok_and(|code| code >= 100);
        } else if line == "@@STOPPED" {
            report.stopped = true;
        } else if line == "@@LOG" {
            in_log = true;
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_server_report_is_read_back() {
        let report = parse_dev("@@DIR /var/www/site/public_html\n@@STARTED\n@@PORT 3101\n@@ALIVE 1\n@@CODE 200\n@@LOG\n▲ Next.js 16.2.10\n- Local: http://127.0.0.1:3101\n");

        assert_eq!(report.dir.as_deref(), Some("/var/www/site/public_html"));
        assert_eq!(report.port, Some(3101));
        assert!(report.alive && report.answering);
        assert!(report.log.contains("Next.js 16.2.10"));

        let starting = parse_dev("@@DIR /srv/app\n@@PORT 3100\n@@ALIVE 1\n@@CODE 000\n@@LOG\n");

        assert!(starting.alive && !starting.answering);
        assert_eq!(parse_dev("@@NODIR\n"), DevReport::default());
    }

    #[test]
    fn the_dev_server_listens_on_loopback_only_and_one_call_can_stop_it() {
        let script = dev_script("'/var/www/site'", false);

        assert!(script.contains("--hostname 127.0.0.1"));
        assert!(script.contains("npm run dev -- --port"));
        assert!(dev_script("'/x'", true).contains("[ 1 = 1 ]"));
    }

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
