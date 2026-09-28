//! The agent's browser (0.13 `screenshot`, 0.14 `browser`): open a page, read it, click, type, press keys,
//! and look at it - in the machine's own Chrome or Edge, headless, driven over the DevTools protocol.
//!
//! A model that writes a web page and never uses it ships a form whose button does nothing and a menu that
//! never opens. 0.13 could only take a picture (and, measured on the macOS CI runner, not even that: Chrome's
//! `--screenshot` flag wrote nothing for a `file:///` page). 0.14 keeps one browser per chat, talks to it the
//! way DevTools does, and gives the agent what a person testing the page does: go there, read what is on
//! it, click a button, fill a field, press Enter, and look.
//!
//! `read` works for every model - the page's text and a numbered list of what can be clicked or filled -
//! so a model that cannot see images can still use the page; the picture is for the models that can.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// A Chromium on this machine: Chrome, Edge, Chromium or Brave, where each installs itself.
pub fn find_browser() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if cfg!(windows) {
        for base in [std::env::var("ProgramFiles").ok(), std::env::var("ProgramFiles(x86)").ok(), std::env::var("LOCALAPPDATA").ok()].into_iter().flatten() {
            candidates.push(PathBuf::from(&base).join("Google/Chrome/Application/chrome.exe"));
            candidates.push(PathBuf::from(&base).join("Microsoft/Edge/Application/msedge.exe"));
            candidates.push(PathBuf::from(&base).join("BraveSoftware/Brave-Browser/Application/brave.exe"));
            candidates.push(PathBuf::from(&base).join("Chromium/Application/chrome.exe"));
        }
    } else if cfg!(target_os = "macos") {
        for app in ["Google Chrome.app/Contents/MacOS/Google Chrome", "Microsoft Edge.app/Contents/MacOS/Microsoft Edge", "Chromium.app/Contents/MacOS/Chromium", "Brave Browser.app/Contents/MacOS/Brave Browser"] {
            candidates.push(PathBuf::from("/Applications").join(app));
        }
    } else {
        for name in ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge", "brave-browser"] {
            for dir in ["/usr/bin", "/usr/local/bin", "/snap/bin"] {
                candidates.push(PathBuf::from(dir).join(name));
            }
        }
    }

    candidates.into_iter().find(|path| path.is_file())
}

/// Whether a model can look at images - the tools that hand it one exist only for those that can.
/// Conservative: a model that cannot read an image rejects the whole request, which ends the turn.
pub fn vision(anthropic: bool, model: &str) -> bool {
    if anthropic {
        return true;
    }

    let model = model.to_ascii_lowercase();

    ["gpt-4o", "gpt-4.1", "gpt-5", "o3", "o4", "-vl", "vl-", "vision", "gemini", "qwen3.5", "qwen3-max", "qwen-max", "qwen-plus", "llava", "pixtral", "kimi-k2.5", "glm-4.5v", "glm-4.6v", "llama-4", "llama4", "grok-4", "mistral-medium", "claude"]
        .iter()
        .any(|marker| model.contains(marker))
        && !model.contains("deepseek")
}

fn stamp() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0)
}

/// One headless browser and the page it shows, over the DevTools protocol.
pub struct Browser {
    child: Child,
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next: u64,
    profile: PathBuf,
    last_used: Instant,
}

impl Browser {
    /// Starts a browser with a page of `width`×`height`.
    pub fn launch(width: u32, height: u32) -> Result<Self, String> {
        let program = find_browser().ok_or_else(|| "no Chrome, Edge or Chromium was found on this machine".to_string())?;
        let profile = std::env::temp_dir().join(format!("sdc-browser-{}-{}", std::process::id(), stamp()));
        let _ = std::fs::create_dir_all(&profile);
        let mut command = Command::new(&program);

        command
            .args([
                "--headless=new",
                "--disable-gpu",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "--disable-background-networking",
                "--hide-scrollbars",
                "--remote-debugging-port=0",
                "--remote-allow-origins=*",
                &format!("--user-data-dir={}", profile.display()),
                &format!("--window-size={width},{height}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        /* A Linux runner or container runs as root, where Chrome refuses to start with its sandbox. */
        if cfg!(target_os = "linux") {
            command.arg("--no-sandbox");
        }

        command.arg("about:blank");

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            command.creation_flags(0x0800_0000);
        }

        let child = command.spawn().map_err(|error| format!("could not start {}: {error}", program.display()))?;

        /* The browser writes the port it chose into its profile once it is listening. */
        let port_file = profile.join("DevToolsActivePort");
        let deadline = Instant::now() + Duration::from_secs(30);
        let port = loop {
            if let Some(port) = std::fs::read_to_string(&port_file).ok().and_then(|text| text.lines().next().and_then(|line| line.trim().parse::<u16>().ok())) {
                break port;
            }

            if Instant::now() > deadline {
                let mut child = child;

                crate::pty::kill_tree(child.id());
                let _ = child.kill();

                return Err(format!("{} started but never opened its DevTools port", program.display()));
            }

            std::thread::sleep(Duration::from_millis(100));
        };

        let pages: Value = ureq::get(&format!("http://127.0.0.1:{port}/json/list"))
            .timeout(Duration::from_secs(10))
            .call()
            .map_err(|error| format!("the browser's page list: {error}"))?
            .into_string()
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        let url = pages
            .as_array()
            .and_then(|pages| pages.iter().find(|page| page["type"] == "page"))
            .and_then(|page| page["webSocketDebuggerUrl"].as_str())
            .ok_or("the browser has no page to drive")?
            .to_string();
        let (socket, _) = tungstenite::connect(url.as_str()).map_err(|error| format!("connecting to the page: {error}"))?;

        if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(45)));
        }

        let mut browser = Self { child, socket, next: 1, profile, last_used: Instant::now() };

        browser.call("Page.enable", json!({}))?;
        browser.call("Emulation.setDeviceMetricsOverride", json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": width < 600 }))?;

        Ok(browser)
    }

    /// One DevTools command and its answer; the events that arrive meanwhile are skipped.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next;

        self.next += 1;
        self.last_used = Instant::now();
        self.socket
            .send(Message::Text(json!({ "id": id, "method": method, "params": params }).to_string()))
            .map_err(|error| format!("{method}: {error}"))?;

        loop {
            let message = match self.socket.read() {
                Ok(message) => message,
                Err(tungstenite::Error::Io(error)) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    return Err(format!("{method}: the browser did not answer in 45 s"));
                }
                Err(error) => return Err(format!("{method}: {error}")),
            };

            let Message::Text(text) = message else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };

            if value["id"].as_u64() == Some(id) {
                if let Some(error) = value.get("error") {
                    return Err(format!("{method}: {}", error["message"].as_str().unwrap_or("error")));
                }

                return Ok(value["result"].clone());
            }
        }
    }

    /// A JavaScript expression's value.
    fn eval(&mut self, expression: &str) -> Result<Value, String> {
        let result = self.call("Runtime.evaluate", json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }))?;

        if let Some(exception) = result.get("exceptionDetails") {
            return Err(format!("the page's script failed: {}", exception["exception"]["description"].as_str().or_else(|| exception["text"].as_str()).unwrap_or("error")));
        }

        Ok(result["result"]["value"].clone())
    }

    /// Waits until the page has loaded and gone quiet for a moment (a navigation after a click included).
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);

        std::thread::sleep(Duration::from_millis(250));

        while Instant::now() < deadline {
            if self.eval("document.readyState").ok().and_then(|state| state.as_str().map(str::to_string)).as_deref() == Some("complete") {
                break;
            }

            std::thread::sleep(Duration::from_millis(200));
        }

        std::thread::sleep(Duration::from_millis(400));
    }

    pub fn open(&mut self, url: &str) -> Result<(), String> {
        let lowered = url.to_ascii_lowercase();

        if !(lowered.starts_with("http://") || lowered.starts_with("https://") || lowered.starts_with("file:///")) {
            return Err("the browser opens http(s):// addresses and file:/// paths".to_string());
        }

        let answer = self.call("Page.navigate", json!({ "url": url }))?;

        if let Some(error) = answer["errorText"].as_str().filter(|error| !error.is_empty()) {
            return Err(format!("{url} could not be opened: {error} - is the page (or its dev server) up?"));
        }

        self.settle();

        Ok(())
    }

    /// The page as text: where it is, its text, and a numbered list of what can be clicked or filled -
    /// each number is also written on its element, so `click 7` finds it again.
    pub fn read(&mut self) -> Result<String, String> {
        let value = self.eval(READ_SCRIPT)?;

        Ok(value.as_str().unwrap_or_default().to_string())
    }

    /// Where an element is, by its number from `read`, a CSS selector, or the words on it.
    fn locate(&mut self, target: &str) -> Result<(f64, f64, String), String> {
        let script = format!("({LOCATE_SCRIPT})({})", serde_json::to_string(target).unwrap_or_default());
        let found = self.eval(&script)?;

        if found.is_null() {
            return Err(format!("nothing on the page matches `{target}` - read the page again for the current numbers"));
        }

        Ok((found["x"].as_f64().unwrap_or(0.0), found["y"].as_f64().unwrap_or(0.0), found["what"].as_str().unwrap_or_default().to_string()))
    }

    /// A real mouse click in the middle of the element, as a person's.
    pub fn click(&mut self, target: &str) -> Result<String, String> {
        let (x, y, what) = self.locate(target)?;

        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            self.call("Input.dispatchMouseEvent", json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 }))?;
        }

        self.settle();

        Ok(what)
    }

    /// Clicks into a field, empties it, and types - as keystrokes the page's own code sees.
    pub fn type_text(&mut self, target: &str, text: &str) -> Result<String, String> {
        let what = self.click(target)?;

        self.eval("(() => { const e = document.activeElement; if (e && 'value' in e) { e.select && e.select(); } return true; })()")?;
        self.call("Input.dispatchKeyEvent", json!({ "type": "keyDown", "key": "Backspace", "code": "Backspace", "windowsVirtualKeyCode": 8 }))?;
        self.call("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": "Backspace", "code": "Backspace", "windowsVirtualKeyCode": 8 }))?;
        self.call("Input.insertText", json!({ "text": text }))?;
        std::thread::sleep(Duration::from_millis(200));

        Ok(what)
    }

    /// One key: Enter, Tab, Escape, Backspace, the arrows.
    pub fn press(&mut self, key: &str) -> Result<(), String> {
        let (name, code) = match key.to_ascii_lowercase().as_str() {
            "enter" | "return" => ("Enter", 13),
            "tab" => ("Tab", 9),
            "escape" | "esc" => ("Escape", 27),
            "backspace" => ("Backspace", 8),
            "arrowdown" | "down" => ("ArrowDown", 40),
            "arrowup" | "up" => ("ArrowUp", 38),
            "arrowleft" | "left" => ("ArrowLeft", 37),
            "arrowright" | "right" => ("ArrowRight", 39),
            "space" => (" ", 32),
            other => return Err(format!("`{other}` is not a key the browser tool presses (Enter, Tab, Escape, Backspace, arrows, Space)")),
        };
        let text = if name == "Enter" { "\r" } else if name == " " { " " } else { "" };

        self.call("Input.dispatchKeyEvent", json!({ "type": "keyDown", "key": name, "code": name, "windowsVirtualKeyCode": code, "text": text }))?;
        self.call("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": name, "code": name, "windowsVirtualKeyCode": code }))?;
        self.settle();

        Ok(())
    }

    /// The page as a PNG.
    pub fn screenshot(&mut self) -> Result<Vec<u8>, String> {
        use base64::Engine as _;

        let shot = self.call("Page.captureScreenshot", json!({ "format": "png" }))?;

        base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().unwrap_or_default())
            .map_err(|error| format!("the screenshot could not be read: {error}"))
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
        crate::pty::kill_tree(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();

        let profile = self.profile.clone();

        /* The profile is still locked for a moment after the browser exits. */
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            let _ = std::fs::remove_dir_all(profile);
        });
    }
}

/// `read`: title, address, the text, and the interactive elements, each numbered.
const READ_SCRIPT: &str = r#"(() => {
  const visible = (e) => { const r = e.getBoundingClientRect(); const s = getComputedStyle(e); return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none'; };
  document.querySelectorAll('[data-sdc]').forEach((e) => e.removeAttribute('data-sdc'));
  const items = [...document.querySelectorAll('a[href], button, input, textarea, select, [role=button], [role=link], [role=tab], [role=checkbox], [onclick], summary')].filter(visible).slice(0, 120);
  const lines = items.map((e, i) => {
    e.setAttribute('data-sdc', String(i + 1));
    const tag = e.tagName.toLowerCase();
    const type = e.getAttribute('type') ? ` type=${e.getAttribute('type')}` : '';
    const name = (e.innerText || e.value || e.getAttribute('aria-label') || e.getAttribute('placeholder') || e.getAttribute('title') || e.getAttribute('name') || '').trim().replace(/\s+/g, ' ').slice(0, 80);
    const href = tag === 'a' ? ` -> ${e.getAttribute('href')}` : '';
    const value = (tag === 'input' || tag === 'textarea') && e.value ? ` value="${String(e.value).slice(0, 40)}"` : '';
    return `[${i + 1}] ${tag}${type} "${name}"${value}${href}`;
  });
  const text = (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n').slice(0, 9000);
  return `Title: ${document.title}\nAddress: ${location.href}\n\n--- text ---\n${text}\n\n--- what can be clicked or filled (use the number) ---\n${lines.join('\n') || '(nothing)'}`;
})()"#;

/// `locate`: a number from `read`, a CSS selector, or the words on an element -> its centre on screen.
const LOCATE_SCRIPT: &str = r#"(t) => {
  let e = null;
  const s = String(t).trim().replace(/^\[|\]$/g, '');
  if (/^\d+$/.test(s)) e = document.querySelector(`[data-sdc="${s}"]`);
  if (!e) { try { e = document.querySelector(s); } catch (_) {} }
  if (!e) {
    const words = s.toLowerCase();
    e = [...document.querySelectorAll('a, button, input, textarea, select, label, summary, [role=button], [role=link], [role=tab]')]
      .find((x) => (x.innerText || x.value || x.getAttribute('aria-label') || x.getAttribute('placeholder') || '').trim().toLowerCase().includes(words));
  }
  if (!e) return null;
  e.scrollIntoView({ block: 'center', inline: 'center' });
  const r = e.getBoundingClientRect();
  const name = (e.innerText || e.value || e.getAttribute('aria-label') || e.getAttribute('placeholder') || '').trim().replace(/\s+/g, ' ').slice(0, 60);
  return { x: r.left + r.width / 2, y: r.top + r.height / 2, what: `${e.tagName.toLowerCase()} "${name}"` };
}"#;

/// One browser per chat, kept between the agent's steps and turns; one left alone for 15 minutes closes.
fn sessions() -> &'static Mutex<HashMap<String, Browser>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Browser>>> = OnceLock::new();

    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Runs `work` on the chat's browser, starting one (at `width`×`height`) when it has none.
pub fn with_session<T>(session_id: &str, width: u32, height: u32, work: impl FnOnce(&mut Browser) -> Result<T, String>) -> Result<T, String> {
    let mut sessions = sessions().lock().map_err(|_| "the browser list is poisoned".to_string())?;

    sessions.retain(|_, browser| browser.last_used.elapsed() < Duration::from_secs(15 * 60));

    if !sessions.contains_key(session_id) {
        sessions.insert(session_id.to_string(), Browser::launch(width, height)?);
    }

    let browser = sessions.get_mut(session_id).expect("inserted above");
    let result = work(browser);

    /* A browser whose connection broke is not kept: the next call starts a fresh one. */
    if let Err(reason) = &result {
        if reason.contains("did not answer") || reason.contains("Connection") || reason.contains("closed") {
            sessions.remove(session_id);
        }
    }

    result
}

/// Closes the chat's browser. `false` when it had none.
pub fn close(session_id: &str) -> bool {
    sessions().lock().map(|mut sessions| sessions.remove(session_id).is_some()).unwrap_or(false)
}

/// Renders `url` at `width`×`height` in a browser of its own and answers the PNG's bytes (`screenshot`).
pub fn screenshot(url: &str, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut browser = Browser::launch(width, height)?;

    browser.open(url)?;
    browser.screenshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_is_claimed_only_for_models_that_see() {
        assert!(vision(true, "claude-opus-5-5"));
        assert!(vision(false, "gpt-5"));
        assert!(vision(false, "qwen3-vl-plus"));
        assert!(!vision(false, "deepseek-v4-pro"));
        assert!(!vision(false, "llama3.2:3b"));
    }

    /// A real page, driven like a person: read it, fill the field, press the button, read the result, look.
    #[test]
    fn the_browser_reads_clicks_types_and_looks() {
        if find_browser().is_none() {
            return;
        }

        let dir = std::env::temp_dir().join(format!("sdc-browser-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page = dir.join("index.html");

        std::fs::write(
            &page,
            "<html><body style='background:#c00'><h1>Shop</h1><input id='q' placeholder='Your name'>\
             <button onclick=\"document.getElementById('out').textContent='Hello ' + document.getElementById('q').value\">Greet</button>\
             <p id='out'></p></body></html>",
        )
        .unwrap();

        let url = format!("file:///{}", page.display().to_string().replace('\\', "/").trim_start_matches('/'));
        let mut browser = Browser::launch(800, 600).unwrap_or_else(|reason| panic!("launch: {reason}"));

        browser.open(&url).unwrap();

        let read = browser.read().unwrap();

        assert!(read.contains("Shop") && read.contains("button \"Greet\""), "{read}");

        browser.type_text("Your name", "Rongdhonu").unwrap();
        browser.click("Greet").unwrap();

        assert!(browser.read().unwrap().contains("Hello Rongdhonu"));

        let png = browser.screenshot().unwrap();

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        drop(browser);
        std::thread::sleep(Duration::from_millis(300));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
