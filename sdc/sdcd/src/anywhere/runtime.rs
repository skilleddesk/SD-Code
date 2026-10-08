//! Turning SDC Anywhere on and off, and what the desktop app can ask of it.
//!
//! `remote.enabled` is **off** by default (plan principle 8). While it is off nothing in this module
//! has started: no task, no socket, no key is loaded. The setting lives in the daemon's own store and
//! only the desktop app's SDCP connection can change it; a browser session cannot, because every
//! `anywhere.*` method is in the never-remote list (`session::REMOTE_FORBIDDEN_PREFIX`).

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use crate::sdcp::envelope::ErrorObject;
use crate::DaemonState;

use super::backend::DaemonBackend;
use super::core::{Core, Out, Settings};
use super::identity::{self, Identity};
use super::registry::Registry;
use super::relay::{self, LinkStatus, Shared};
use super::router::OnTimeout;
use super::session::Limits;
use super::webauthn::RelyingParty;

/// Where the relay lives unless the person points SDC elsewhere (`anywhere.relay`).
pub const DEFAULT_RELAY: &str = "wss://sdc.skilleddesk.com";

/// The page the QR code opens.
pub const DEFAULT_WEB: &str = "https://sdc.skilleddesk.com";

const KEY_ENABLED: &str = "anywhere.enabled";
const KEY_RELAY: &str = "anywhere.relay";
const KEY_ACCEPT_FILE_KEY: &str = "anywhere.accept_file_key";
const KEY_NOTIFY_WHEN: &str = "anywhere.notify_when";
const KEY_IDLE_MINUTES: &str = "anywhere.idle_minutes";
const KEY_TIMEOUT_SEC: &str = "anywhere.approval_timeout_sec";
const KEY_ON_TIMEOUT: &str = "anywhere.on_timeout";
const KEY_VIEW_LOCK_MIN: &str = "anywhere.view_idle_lock_minutes";
const KEY_OPERATE_MIN: &str = "anywhere.operate_window_minutes";
const KEY_GUEST_MIN: &str = "anywhere.guest_session_max_minutes";
const KEY_SCOPED_MIN: &str = "anywhere.max_scoped_grant_minutes";
const KEY_EMAIL: &str = "anywhere.email";
const KEY_ESCALATE_SEC: &str = "anywhere.escalate_email_sec";

/// A plain check of an address the owner typed: one `@`, no spaces or control characters, a dot in the domain. The relay checks
/// it again before it keeps it; this keeps an obvious typo from ever being sent.
fn plausible_address(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else { return false };

    text.len() <= 254
        && !local.is_empty()
        && local.len() <= 64
        && domain.contains('.')
        && !domain.contains('@')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && text.chars().all(|c| !c.is_whitespace() && !c.is_control() && !"<>(),;:\"\\[]".contains(c))
}

struct Running {
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
    to_driver: mpsc::UnboundedSender<Vec<Out>>,
}

#[cfg(test)]
mod address_tests {
    use super::plausible_address;

    #[test]
    fn ordinary_addresses_pass_and_dangerous_ones_do_not() {
        for good in ["a@b.co", "first.last+tag@sub.example.co.uk", "o'brien@example.com"] {
            assert!(plausible_address(good), "{good}");
        }

        for bad in ["", "plain", "@b.co", "a@", "a@b", "a@@b.co", "a b@c.co", "a@b.co\r\nBcc: x@y.co", "a@b.co,c@d.co", "<a@b.co>", "a@.co", "a@b.", &format!("{}@b.co", "x".repeat(65)), &format!("a@{}.co", "x".repeat(250))] {
            assert!(!plausible_address(bad), "{bad:?}");
        }
    }
}

/// The handle `DaemonState` keeps. Cheap when disabled: a mutex around `None`.
#[derive(Default)]
pub struct Anywhere {
    running: Mutex<Option<Running>>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
}

fn invalid(message: impl Into<String>) -> ErrorObject {
    ErrorObject::bad_request(message)
}

fn setting_i64(state: &DaemonState, key: &str, default: i64) -> i64 {
    state.store.setting(key).ok().flatten().and_then(|value| value.parse().ok()).unwrap_or(default)
}

fn setting_str(state: &DaemonState, key: &str, default: &str) -> String {
    state.store.setting(key).ok().flatten().filter(|value| !value.is_empty()).unwrap_or_else(|| default.to_string())
}

/// The settings the core runs on. A stored value can shorten a limit but never lengthen it (`Limits::clamp`).
pub fn settings_from(state: &DaemonState) -> Settings {
    let minutes = |key: &str, default: i64| setting_i64(state, key, default).max(0) * 60_000;
    let limits = Limits {
        view_idle_ms: minutes(KEY_VIEW_LOCK_MIN, 15),
        operate_window_ms: minutes(KEY_OPERATE_MIN, 5),
        guest_max_ms: minutes(KEY_GUEST_MIN, 120),
        ..Limits::default()
    };

    Settings {
        limits: limits.clamp(),
        /* An allow-for-a-while may be shortened but never run past an hour. */
        max_scoped_grant_ms: setting_i64(state, KEY_SCOPED_MIN, 60).clamp(1, 60) * 60_000,
        approval_timeout_ms: setting_i64(state, KEY_TIMEOUT_SEC, 1800).clamp(30, 24 * 3600) * 1000,
        on_timeout: if setting_str(state, KEY_ON_TIMEOUT, "pause") == "deny" { OnTimeout::Deny } else { OnTimeout::Pause },
        rp: relying_party(state),
        escalate_email_sec: setting_i64(state, KEY_ESCALATE_SEC, super::core::DEFAULT_ESCALATE_EMAIL_SEC).clamp(1, 3600),
    }
}

/// The relying party follows the web origin, so a self-hosted relay (or a test one) gets its own passkeys.
fn relying_party(state: &DaemonState) -> RelyingParty {
    let relay = setting_str(state, KEY_RELAY, DEFAULT_RELAY);
    let origin = relay.replacen("wss://", "https://", 1).replacen("ws://", "http://", 1);
    let host = origin.trim_start_matches("https://").trim_start_matches("http://").split(['/', ':']).next().unwrap_or_default().to_string();

    RelyingParty { id: host, origin: origin.trim_end_matches('/').to_string() }
}

impl Anywhere {
    /// The tokio handle the driver task is spawned on. Called once by the daemon's `main`; a daemon built
    /// without one (a unit test, the CLI) can still answer `anywhere.status`, but cannot start.
    pub fn attach(&self, handle: tokio::runtime::Handle) {
        if let Ok(mut slot) = self.runtime.lock() {
            *slot = Some(handle);
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().map(|running| running.is_some()).unwrap_or(false)
    }

    /// Starts it again after a restart, if the person had left it on.
    pub fn resume(&self, state: &Arc<DaemonState>) {
        if setting_str(state, KEY_ENABLED, "0") == "1" {
            if let Err(error) = self.start(state) {
                eprintln!("sdcd: SDC Anywhere did not start: {}", error.message);
            }
        }
    }

    /// Turns it on: checks where the keys will be kept, loads or makes them, and starts the driver.
    pub fn start(&self, state: &Arc<DaemonState>) -> Result<(), ErrorObject> {
        if self.is_running() {
            return Ok(());
        }

        let accepted = setting_str(state, KEY_ACCEPT_FILE_KEY, "0") == "1";

        /* DESIGN.md OQ-7: the identity key is exactly what the keychain exists for. */
        if !identity::storage_allowed(accepted) {
            return Err(ErrorObject::permission_denied(
                "This computer has no OS keychain that SDC can reach, so SDC Anywhere's identity key would be kept in a file. \
                 Turn on 'Keep the key in a protected file' in Settings → SDC Anywhere to accept that, then try again.",
            ));
        }

        let handle = self.runtime.lock().ok().and_then(|slot| slot.clone()).ok_or_else(|| ErrorObject::internal("the daemon has no async runtime to start SDC Anywhere on"))?;
        let identity = Identity::load_or_create().map_err(ErrorObject::internal)?;
        let registry = Arc::new(Registry::new(state.store.clone()));
        let backend = Arc::new(DaemonBackend::new(state.clone()));
        let clock: Arc<dyn Fn() -> i64 + Send + Sync> = Arc::new(|| chrono::Utc::now().timestamp_millis());
        let core = Core::new(identity, registry, backend, settings_from(state), clock);
        let (to_driver, from_desktop) = mpsc::unbounded_channel();
        let feedback = to_driver.clone();
        let shared = Arc::new(Shared {
            core: Mutex::new(core),
            status: Mutex::new(LinkStatus::default()),
            notify_when_idle: AtomicBool::new(setting_str(state, KEY_NOTIFY_WHEN, "idle") == "idle"),
            idle_minutes: Mutex::new(setting_i64(state, KEY_IDLE_MINUTES, 5).clamp(1, 240) as u64),
            email: Mutex::new(setting_str(state, KEY_EMAIL, "")),
            wake: tokio::sync::Notify::new(),
            feedback,
        });
        let (stop, stop_rx) = watch::channel(false);
        let url = setting_str(state, KEY_RELAY, DEFAULT_RELAY);
        let driver_state = state.clone();
        let driver_shared = shared.clone();

        handle.spawn(async move { relay::run(driver_state, driver_shared, url, stop_rx, from_desktop).await });

        *self.running.lock().map_err(|_| ErrorObject::internal("anywhere state poisoned"))? = Some(Running { shared, stop, to_driver });
        state.store.set_setting(KEY_ENABLED, "1").map_err(ErrorObject::internal)?;

        Ok(())
    }

    /// Turns it off. Every browser session ends; paired devices stay paired.
    pub fn stop(&self, state: &Arc<DaemonState>) -> Result<(), ErrorObject> {
        if let Some(running) = self.running.lock().map_err(|_| ErrorObject::internal("anywhere state poisoned"))?.take() {
            let _ = running.stop.send(true);
        }

        state.store.set_setting(KEY_ENABLED, "0").map_err(ErrorObject::internal)?;

        Ok(())
    }

    /// Runs `f` on the core and hands whatever it returns to the driver to carry out.
    fn with_core<R>(&self, f: impl FnOnce(&mut Core) -> (R, Vec<Out>)) -> Result<R, ErrorObject> {
        let guard = self.running.lock().map_err(|_| ErrorObject::internal("anywhere state poisoned"))?;
        let running = guard.as_ref().ok_or_else(|| invalid("SDC Anywhere is turned off"))?;
        let mut core = running.shared.core.lock().map_err(|_| ErrorObject::internal("anywhere core poisoned"))?;
        let (result, outs) = f(&mut core);

        if !outs.is_empty() {
            let _ = running.to_driver.send(outs);
        }

        Ok(result)
    }

    pub fn begin_pairing(&self, state: &Arc<DaemonState>, guest: bool) -> Result<Value, ErrorObject> {
        let offer = self.with_core(|core| (core.begin_pairing(guest), Vec::new()))?.map_err(ErrorObject::internal)?;
        let web = setting_str(state, KEY_RELAY, DEFAULT_RELAY).replacen("wss://", "https://", 1).replacen("ws://", "http://", 1);

        Ok(json!({
            "url": format!("{}/pair#{}", web.trim_end_matches('/'), offer.fragment),
            "fingerprint": offer.fingerprint,
            "expiresAt": offer.expires_at,
            "guest": guest,
        }))
    }

    pub fn pair_requests(&self) -> Result<Value, ErrorObject> {
        self.with_core(|core| {
            let requests: Vec<Value> = core
                .pair_requests()
                .into_iter()
                .map(|request| json!({ "deviceId": request.device_id, "name": request.name, "userAgent": request.user_agent, "guest": request.guest, "code": request.sas, "askedAt": request.asked_at }))
                .collect();

            (json!({ "requests": requests }), Vec::new())
        })
    }

    pub fn confirm_pairing(&self, device_id: &str, accept: bool) -> Result<Value, ErrorObject> {
        self.with_core(|core| match core.confirm_pairing(device_id, accept) {
            Ok(outs) => (Ok(json!({ "paired": accept })), outs),
            Err(error) => (Err(ErrorObject::not_found(error.to_string())), Vec::new()),
        })?
    }

    pub fn revoke(&self, device_id: &str) -> Result<Value, ErrorObject> {
        self.with_core(|core| match core.revoke_device(device_id) {
            Ok(outs) => (Ok(json!({ "revoked": true })), outs),
            Err(error) => (Err(ErrorObject::internal(error)), Vec::new()),
        })?
    }

    /// The paired devices, read straight from the store so the list works even while it is off.
    pub fn devices(&self, state: &Arc<DaemonState>) -> Value {
        let registry = Registry::new(state.store.clone());
        let devices: Vec<Value> = registry
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|device| {
                json!({
                    "id": device.id, "name": device.name, "userAgent": device.user_agent, "guest": device.guest,
                    "createdAt": device.created_at, "lastSeen": device.last_seen, "revokedAt": device.revoked_at, "expiresAt": device.expires_at,
                })
            })
            .collect();

        json!({ "devices": devices })
    }

    pub fn status(&self, state: &Arc<DaemonState>) -> Value {
        let storage = identity::storage();
        let (link, connections, waiting, pairing, id) = match self.running.lock().ok().and_then(|guard| guard.as_ref().map(|running| running.shared.clone())) {
            Some(shared) => {
                let link = shared.status.lock().map(|status| status.clone()).unwrap_or_default();
                let core = shared.core.lock();

                match core {
                    Ok(core) => (Some(link), core.connections(), core.open_requests(), core.pair_requests().len(), Some(core.daemon_id().to_string())),
                    Err(_) => (Some(link), 0, 0, 0, None),
                }
            }
            None => (None, 0, 0, 0, None),
        };

        json!({
            "enabled": setting_str(state, KEY_ENABLED, "0") == "1",
            "running": link.is_some(),
            "connected": link.as_ref().is_some_and(|link| link.connected),
            "lastError": link.as_ref().and_then(|link| link.last_error.clone()),
            "relay": setting_str(state, KEY_RELAY, DEFAULT_RELAY),
            "daemonId": id,
            "browserConnections": connections,
            "waitingApprovals": waiting,
            "pairingRequests": pairing,
            "keyStorage": { "backend": storage.backend, "protection": storage.protection, "acceptedFileKey": setting_str(state, KEY_ACCEPT_FILE_KEY, "0") == "1" },
            "settings": {
                "notifyWhen": setting_str(state, KEY_NOTIFY_WHEN, "idle"),
                "idleMinutes": setting_i64(state, KEY_IDLE_MINUTES, 5),
                "approvalTimeoutSec": setting_i64(state, KEY_TIMEOUT_SEC, 1800),
                "onTimeout": setting_str(state, KEY_ON_TIMEOUT, "pause"),
                "viewIdleLockMinutes": setting_i64(state, KEY_VIEW_LOCK_MIN, 15),
                "operateWindowMinutes": setting_i64(state, KEY_OPERATE_MIN, 5),
                "guestSessionMaxMinutes": setting_i64(state, KEY_GUEST_MIN, 120),
                "maxScopedGrantMinutes": setting_i64(state, KEY_SCOPED_MIN, 60),
                "email": setting_str(state, KEY_EMAIL, ""),
                "escalateEmailSec": setting_i64(state, KEY_ESCALATE_SEC, super::core::DEFAULT_ESCALATE_EMAIL_SEC),
            },
        })
    }

    /// Changes a setting. Only the keys listed here; anything else is refused. Takes effect on the next start.
    pub fn configure(&self, state: &Arc<DaemonState>, params: &serde_json::Map<String, Value>) -> Result<Value, ErrorObject> {
        let allowed: [(&str, &str); 13] = [
            ("email", KEY_EMAIL),
            ("escalateEmailSec", KEY_ESCALATE_SEC),
            ("maxScopedGrantMinutes", KEY_SCOPED_MIN),
            ("relay", KEY_RELAY),
            ("acceptFileKey", KEY_ACCEPT_FILE_KEY),
            ("notifyWhen", KEY_NOTIFY_WHEN),
            ("idleMinutes", KEY_IDLE_MINUTES),
            ("approvalTimeoutSec", KEY_TIMEOUT_SEC),
            ("onTimeout", KEY_ON_TIMEOUT),
            ("viewIdleLockMinutes", KEY_VIEW_LOCK_MIN),
            ("operateWindowMinutes", KEY_OPERATE_MIN),
            ("guestSessionMaxMinutes", KEY_GUEST_MIN),
            ("enabled", KEY_ENABLED),
        ];

        for (name, value) in params {
            let Some((_, key)) = allowed.iter().find(|(field, _)| field == name) else { continue };

            if *key == KEY_ENABLED {
                continue;
            }

            let text = match value {
                Value::String(text) => text.clone(),
                Value::Bool(flag) => if *flag { "1".into() } else { "0".into() },
                Value::Number(number) => number.to_string(),
                _ => return Err(invalid(format!("`{name}` must be a string, a number or a boolean"))),
            };

            if *key == KEY_RELAY && !(text.starts_with("wss://") || text.starts_with("ws://127.0.0.1") || text.starts_with("ws://localhost")) {
                return Err(invalid("the relay address must start with wss:// (plain ws:// is only allowed for this computer)"));
            }

            if *key == KEY_NOTIFY_WHEN && !["idle", "always", "never"].contains(&text.as_str()) {
                return Err(invalid("notifyWhen is idle, always or never"));
            }

            if *key == KEY_ON_TIMEOUT && !["pause", "deny"].contains(&text.as_str()) {
                return Err(invalid("onTimeout is pause or deny"));
            }

            if *key == KEY_EMAIL && !text.is_empty() && !plausible_address(&text) {
                return Err(invalid("that does not look like an email address"));
            }

            if *key == KEY_ESCALATE_SEC && !text.parse::<i64>().is_ok_and(|seconds| (1..=3600).contains(&seconds)) {
                return Err(invalid("escalateEmailSec is a number of seconds from 1 to 3600"));
            }

            state.store.set_setting(key, &text).map_err(ErrorObject::internal)?;

            if *key == KEY_EMAIL {
                self.tell_relay_email(&text);
            }
        }

        Ok(self.status(state))
    }

    /// The address is used at once, without a restart: the running link learns it and tells the relay.
    fn tell_relay_email(&self, address: &str) {
        if let Ok(guard) = self.running.lock() {
            if let Some(running) = guard.as_ref() {
                if let Ok(mut email) = running.shared.email.lock() {
                    *email = address.to_string();
                }

                let _ = running.to_driver.send(vec![Out::Ctl(json!({ "ctl": "email.set", "address": address }))]);
            }
        }
    }

    /// Erases the identity: every paired device must pair again. Only the desktop can do this.
    pub fn reset_identity(&self, state: &Arc<DaemonState>) -> Result<(), ErrorObject> {
        self.stop(state)?;
        Identity::reset().map_err(ErrorObject::internal)
    }
}
