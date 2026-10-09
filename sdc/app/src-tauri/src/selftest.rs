//! The window self-test (0.22) - how the frameless window is checked on Linux and macOS, where nobody on the team
//! has a machine: CI starts the real app with `SDC_WINDOW_SELFTEST=<report.json>` and reads the report.
//!
//! The app then drives its own window through the same calls the title-bar buttons make - maximise, restore,
//! minimise, resize - checks there is no native frame, asks the page what its title bar holds (the three window
//! buttons, the drag regions), writes everything to the report and quits. Without the variable none of this runs.

use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

/// What the page reported about its title bar, once it has.
static DOM: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

/// The page's answer (`selftest_dom`), sent from the script [`start`] evaluates in it.
#[tauri::command]
pub fn selftest_dom(facts: Value) {
    if let Ok(mut dom) = DOM.lock() {
        *dom = Some(facts);
    }
}

/// Starts the self-test when `SDC_WINDOW_SELFTEST` names a report file; does nothing otherwise.
pub fn start(handle: &AppHandle) {
    let Some(report) = std::env::var_os("SDC_WINDOW_SELFTEST").filter(|path| !path.is_empty()) else {
        return;
    };
    let handle = handle.clone();

    std::thread::spawn(move || {
        let result = run(&handle);
        let text = serde_json::to_string_pretty(&result).unwrap_or_default();

        let _ = std::fs::write(&report, &text);
        println!("SDC_WINDOW_SELFTEST {text}");

        /* Long enough for a screenshot of the restored window, then out. */
        std::thread::sleep(Duration::from_secs(4));
        handle.exit(0);
    });
}

fn run(handle: &AppHandle) -> Value {
    let pause = |seconds: u64| std::thread::sleep(Duration::from_millis(seconds * 1000));

    /* The page loads and the daemon answers first - and on Linux, CI drives the window with a real mouse in this
       time (SDC_WINDOW_SELFTEST_WAIT, seconds) before the self-test touches it. */
    pause(std::env::var("SDC_WINDOW_SELFTEST_WAIT").ok().and_then(|value| value.parse().ok()).unwrap_or(10));

    let Some(window) = handle.get_webview_window("main") else {
        return json!({ "ok": false, "error": "no main window" });
    };
    let mut checks: Vec<Value> = Vec::new();
    let mut check = |name: &str, passed: bool, detail: Value| checks.push(json!({ "name": name, "ok": passed, "detail": detail }));

    let decorated = window.is_decorated().unwrap_or(true);

    check("no native frame (decorations off)", !decorated, json!(decorated));

    let before = window.inner_size().ok();

    let _ = window.eval(
        "(() => { const facts = { controls: !!document.querySelector('.win-controls'), buttons: document.querySelectorAll('.win-controls .win-btn').length, \
         closeButton: !!document.querySelector('.win-btn.win-close'), dragRegions: document.querySelectorAll('[data-tauri-drag-region]').length, \
         topbar: (document.querySelector('.topbar')?.getBoundingClientRect().height ?? 0), background: getComputedStyle(document.body).backgroundColor, \
         title: document.title, userAgent: navigator.userAgent }; \
         window.__TAURI_INTERNALS__.invoke('selftest_dom', { facts }); })()",
    );
    pause(2);

    let dom = DOM.lock().ok().and_then(|dom| dom.clone()).unwrap_or(Value::Null);

    check("the title bar draws minimise, maximise and close", dom["buttons"].as_u64() == Some(3) && dom["closeButton"] == true, dom.clone());
    check("the title bar can drag the window", dom["dragRegions"].as_u64().unwrap_or(0) >= 2, dom["dragRegions"].clone());

    let _ = window.maximize();
    pause(2);
    let maximized = window.is_maximized().unwrap_or(false);

    check("maximise", maximized, json!(window.inner_size().ok().map(|size| [size.width, size.height])));

    let _ = window.unmaximize();
    pause(2);
    check("restore", !window.is_maximized().unwrap_or(true), json!(window.inner_size().ok().map(|size| [size.width, size.height])));

    let _ = window.set_size(tauri::LogicalSize::new(1100.0, 720.0));
    pause(1);
    let resized = window.inner_size().ok();

    check("resize", resized != before && resized.is_some(), json!(resized.map(|size| [size.width, size.height])));

    let _ = window.minimize();
    pause(2);
    let minimized = window.is_minimized().unwrap_or(false);

    check("minimise", minimized, json!(minimized));

    let _ = window.unminimize();
    let _ = window.set_focus();
    pause(2);
    check("bring back from minimised", !window.is_minimized().unwrap_or(true), Value::Null);

    let ok = checks.iter().all(|check| check["ok"] == true);

    json!({ "ok": ok, "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "checks": checks, "dom": dom })
}
