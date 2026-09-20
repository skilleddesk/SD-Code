//! SDC desktop shell (Tauri 2).
//!
//! The window, the plugins the capability set grants, and the **SDCP bridge** (`src/sdcp.rs`).
//!
//! The frontend never opens a socket: it calls `sdcp_status`, `sdcp_connect` and `sdcp_call`, and
//! listens for `sdcp://event` for everything the daemon pushes. `app/src/lib/transport.ts` picks
//! between this bridge, a `VITE_SDCP_URL` websocket and the in-process demo daemon, in that order.

mod sdcp;

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

/// Runs the desktop application.
///
/// The `mobile_entry_point` attribute is inert on desktop; it is kept so the same entry point can
/// serve the mobile targets without a rewrite.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // What the frontend is allowed to call is decided by `capabilities/default.json`,
        // not here: registering a plugin does not grant its permissions.
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(sdcp::SdcpBridge::new())
        .invoke_handler(tauri::generate_handler![
            sdcp_status,
            sdcp_connect,
            sdcp_call,
            sdcp_subscribe,
            sdcp_stop_daemon
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the SDC window");
}

/// Whether the daemon is reachable, and on what address.
#[tauri::command]
fn sdcp_status(bridge: State<'_, Arc<sdcp::SdcpBridge>>) -> Value {
    sdcp::status_json(&bridge)
}

/// Connects to the daemon, starting one first if nothing is listening on the loopback port.
#[tauri::command]
async fn sdcp_connect(
    app: tauri::AppHandle,
    bridge: State<'_, Arc<sdcp::SdcpBridge>>,
) -> Result<Value, String> {
    sdcp::connect(app, bridge.inner().clone()).await
}

/// One SDCP request, exactly as `TauriTransport` writes it: `{ method, params, id }`. The answer is
/// the daemon's whole response envelope, so the frontend correlates it by `id`.
#[tauri::command]
async fn sdcp_call(
    app: tauri::AppHandle,
    bridge: State<'_, Arc<sdcp::SdcpBridge>>,
    method: String,
    params: Option<Value>,
    id: Option<String>,
) -> Result<Value, String> {
    sdcp::call(app, bridge.inner().clone(), method, params.unwrap_or(Value::Null), id).await
}

/// Starts the notification reader: every event the daemon pushes becomes `sdcp://event`.
#[tauri::command]
async fn sdcp_subscribe(
    app: tauri::AppHandle,
    bridge: State<'_, Arc<sdcp::SdcpBridge>>,
) -> Result<(), String> {
    sdcp::subscribe(app, bridge.inner().clone()).await
}

/// Stops a daemon this bridge started; a user-started daemon is left running.
#[tauri::command]
fn sdcp_stop_daemon(bridge: State<'_, Arc<sdcp::SdcpBridge>>) -> Value {
    sdcp::stop_daemon(&bridge)
}

