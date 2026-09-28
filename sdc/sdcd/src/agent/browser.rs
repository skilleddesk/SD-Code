//! `screenshot` (0.13): the agent looks at the page it built.
//!
//! A model that writes a web page and never sees it ships overlapping buttons and white text on white.
//! With a vision model, SDC renders the page in the machine's own Chrome or Edge, headless, and hands
//! the picture back to the model - so "make it look like the mock-up" can be checked by the one doing it.
//! The dev server a background process started (`start_process`) is exactly what it points at.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

/// Renders `url` at `width`×`height` and answers the PNG's bytes.
pub fn screenshot(url: &str, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let lowered = url.to_ascii_lowercase();

    if !(lowered.starts_with("http://") || lowered.starts_with("https://") || lowered.starts_with("file:///")) {
        return Err("screenshot takes an http(s):// address or a file:/// path".to_string());
    }

    let browser = find_browser().ok_or_else(|| "no Chrome, Edge or Chromium was found on this machine to render the page".to_string())?;
    let dir = std::env::temp_dir().join(format!("sdc-shot-{}-{}", std::process::id(), rand_suffix()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("page.png");
    let mut command = Command::new(&browser);

    command
        .args([
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--virtual-time-budget=6000",
            &format!("--user-data-dir={}", dir.join("profile").display()),
            &format!("--window-size={width},{height}"),
            &format!("--screenshot={}", file.display()),
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(0x0800_0000);
    }

    let mut child = command.spawn().map_err(|error| format!("could not start {}: {error}", browser.display()))?;
    let deadline = Instant::now() + Duration::from_secs(45);

    loop {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }

        if Instant::now() > deadline {
            crate::pty::kill_tree(child.id());
            let _ = child.kill();

            return Err(format!("the page at {url} did not finish rendering in 45 s"));
        }

        std::thread::sleep(Duration::from_millis(200));
    }

    let bytes = std::fs::read(&file).map_err(|_| format!("the browser rendered nothing for {url} - is the page (or the dev server) up?"));
    let _ = std::fs::remove_dir_all(&dir);

    bytes
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0)
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

    #[test]
    fn a_local_page_is_rendered_when_a_browser_is_here() {
        if find_browser().is_none() {
            return;
        }

        let dir = std::env::temp_dir().join(format!("sdc-shot-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page = dir.join("index.html");

        std::fs::write(&page, "<html><body style='background:#c00'><h1>Hello</h1></body></html>").unwrap();

        let url = format!("file:///{}", page.display().to_string().replace('\\', "/").trim_start_matches('/'));
        let png = screenshot(&url, 400, 300).unwrap();

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
