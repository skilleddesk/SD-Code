//! GitHub (1.0.0): connect an account once, then push any project - on this computer or on a VPS - in one click.
//!
//! **Connecting** never asks anyone to make a token by hand:
//!
//! * **Approve in the browser** (GitHub's device flow): SDC asks GitHub for a short code, opens
//!   `github.com/login/device`, the person signs in and presses *Authorize*, and SDC receives the token itself. It
//!   needs SDC's OAuth App *client ID* - public, not a secret - from `SDC_GITHUB_CLIENT_ID` at build time, the same
//!   variable at run time, or the `github.clientId` setting.
//! * **Git Credential Manager** (Windows): Git for Windows' own sign-in window - also a browser approval - when no
//!   client ID is configured. Its answer is the token Git itself would use.
//!
//! The token lives in the OS keychain (`github.token`) and only ever reaches `git` through the environment
//! (`GIT_CONFIG_*`), never a command line - on a VPS through the SSH session's stdin. **Disconnecting** deletes it.
//!
//! **Pushing** a folder: a folder that is not a repository becomes one (`main`), the work is committed (with the GitHub
//! account's no-reply address when the folder has no identity), a private repository named after the folder is created
//! when there is no `origin`, and the branch is pushed.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};

use crate::auth::keychain;
use crate::sdcp::envelope::ErrorObject;

/// Where the token is kept.
pub const TOKEN_KEY: &str = "github.token";

/// What SDC asks for: push to repositories (public and private) and create them, and read who signed in.
const SCOPES: &str = "repo read:user user:email";

/// The OAuth App client ID SDC was built with, if any.
const BUILT_CLIENT_ID: Option<&str> = option_env!("SDC_GITHUB_CLIENT_ID");

fn http() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .user_agent(&format!("sdcd/{}", crate::VERSION))
        .build()
}

/// The client ID: the setting, else the environment, else the one SDC was built with.
pub fn client_id(store: &crate::store::Store) -> Option<String> {
    store
        .setting("github.clientId")
        .ok()
        .flatten()
        .or_else(|| std::env::var("SDC_GITHUB_CLIENT_ID").ok())
        .or_else(|| BUILT_CLIENT_ID.map(str::to_string))
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
}

/// Whether Git Credential Manager is here to drive a browser sign-in.
pub fn has_credential_manager() -> bool {
    git_command().is_some_and(|mut git| {
        git.args(["credential-manager", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
    })
}

fn git_command() -> Option<std::process::Command> {
    crate::host::program::command("git")
}

/* ------------------------------------------------------------------------------------------------
 * The account.
 * ---------------------------------------------------------------------------------------------- */

/// A response's body as JSON.
fn json_of(response: ureq::Response) -> Result<Value, String> {
    let text = response.into_string().map_err(|error| error.to_string())?;

    serde_json::from_str(&text).map_err(|error| error.to_string())
}

/// `GET https://api.github.com<path>` (or another verb) with the token.
fn api(token: &str, method: &str, path: &str, body: Option<Value>) -> Result<Value, (u16, String)> {
    let request = http()
        .request(method, &format!("https://api.github.com{path}"))
        .set("authorization", &format!("Bearer {token}"))
        .set("accept", "application/vnd.github+json")
        .set("x-github-api-version", "2022-11-28");
    let response = match body {
        Some(body) => request.set("content-type", "application/json").send_string(&body.to_string()),
        None => request.call(),
    };

    match response {
        Ok(response) => json_of(response).map_err(|error| (0, error)),
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<Value>(&text).ok().and_then(|value| value["message"].as_str().map(str::to_string)).unwrap_or(text);

            Err((code, message))
        }
        Err(error) => Err((0, error.to_string())),
    }
}

/// The signed-in account (`login`, `name`, `avatar_url`, `id`), or why there is none.
fn whoami(token: &str) -> Result<Value, String> {
    api(token, "GET", "/user", None).map_err(|(code, message)| if code == 401 { "GitHub no longer accepts this sign-in - connect again".to_string() } else { message })
}

/// Keeps a token after checking it with GitHub. Answers the account.
pub fn save_token(token: &str) -> Result<Value, ErrorObject> {
    let token = token.trim();
    let user = whoami(token).map_err(ErrorObject::bad_request)?;

    keychain::set(TOKEN_KEY, token)?;
    *account_cache().lock().unwrap_or_else(|poison| poison.into_inner()) = Some(user.clone());

    Ok(user)
}

fn account_cache() -> &'static Mutex<Option<Value>> {
    static CACHE: OnceLock<Mutex<Option<Value>>> = OnceLock::new();

    CACHE.get_or_init(|| Mutex::new(None))
}

/// `github.status`: connected or not, as whom, and which ways of connecting this machine offers.
pub fn status(store: &crate::store::Store) -> Value {
    let token = keychain::get(TOKEN_KEY);
    let account = token.as_deref().and_then(|token| {
        let cached = account_cache().lock().unwrap_or_else(|poison| poison.into_inner()).clone();

        cached.or_else(|| {
            let user = whoami(token).ok()?;

            *account_cache().lock().unwrap_or_else(|poison| poison.into_inner()) = Some(user.clone());
            Some(user)
        })
    });
    let pending = pending().lock().unwrap_or_else(|poison| poison.into_inner()).clone();

    json!({
        "connected": account.is_some(),
        "tokenStored": token.is_some(),
        "login": account.as_ref().and_then(|user| user["login"].as_str()),
        "name": account.as_ref().and_then(|user| user["name"].as_str()),
        "avatarUrl": account.as_ref().and_then(|user| user["avatar_url"].as_str()),
        "deviceFlow": client_id(store).is_some(),
        "credentialManager": has_credential_manager(),
        "pending": pending,
    })
}

/// `github.disconnect`: SDC forgets the token. GitHub keeps the authorization until it is revoked there.
pub fn disconnect() -> Result<Value, ErrorObject> {
    keychain::delete(TOKEN_KEY)?;
    *account_cache().lock().unwrap_or_else(|poison| poison.into_inner()) = None;
    *pending().lock().unwrap_or_else(|poison| poison.into_inner()) = Value::Null;

    Ok(json!({ "connected": false, "revokeUrl": "https://github.com/settings/applications" }))
}

/* ------------------------------------------------------------------------------------------------
 * Connecting: the browser approval, polled on a thread; the window reads `pending` from `github.status`.
 * ---------------------------------------------------------------------------------------------- */

fn pending() -> &'static Mutex<Value> {
    static PENDING: OnceLock<Mutex<Value>> = OnceLock::new();

    PENDING.get_or_init(|| Mutex::new(Value::Null))
}

fn set_pending(value: Value) {
    *pending().lock().unwrap_or_else(|poison| poison.into_inner()) = value;
}

/// `github.connect`: starts the browser approval. Answers what to show (`userCode`, `verificationUri`) for the device
/// flow, or `{ method: "credential-manager" }` while Git's own sign-in window is open.
pub fn connect(store: &crate::store::Store) -> Result<Value, ErrorObject> {
    if let Some(client) = client_id(store) {
        return device_start(&client);
    }

    if has_credential_manager() {
        set_pending(json!({ "state": "waiting", "method": "credential-manager" }));
        std::thread::spawn(|| match credential_manager_sign_in() {
            Ok(token) => match save_token(&token) {
                Ok(user) => set_pending(json!({ "state": "done", "login": user["login"] })),
                Err(error) => set_pending(json!({ "state": "failed", "error": error.message })),
            },
            Err(error) => set_pending(json!({ "state": "failed", "error": error })),
        });

        return Ok(json!({ "method": "credential-manager" }));
    }

    Err(ErrorObject::bad_request(
        "Browser approval needs SDC's GitHub app ID on this build (SDC_GITHUB_CLIENT_ID), and Git Credential Manager is not here either",
    ))
}

fn device_start(client: &str) -> Result<Value, ErrorObject> {
    let response = http()
        .post("https://github.com/login/device/code")
        .set("accept", "application/json")
        .send_form(&[("client_id", client), ("scope", SCOPES)])
        .map_err(|error| ErrorObject::bad_request(format!("GitHub did not start the sign-in: {error}")))
        .and_then(|response| json_of(response).map_err(ErrorObject::bad_request))?;

    if let Some(error) = response["error_description"].as_str().or(response["error"].as_str()) {
        return Err(ErrorObject::bad_request(format!("GitHub refused the sign-in: {error}")));
    }

    let device = response["device_code"].as_str().unwrap_or_default().to_string();
    let user_code = response["user_code"].as_str().unwrap_or_default().to_string();
    let uri = response["verification_uri"].as_str().unwrap_or("https://github.com/login/device").to_string();
    let interval = response["interval"].as_u64().unwrap_or(5).max(1);
    let expires = response["expires_in"].as_u64().unwrap_or(900);
    let client = client.to_string();

    set_pending(json!({ "state": "waiting", "method": "device", "userCode": user_code, "verificationUri": uri }));
    std::thread::spawn(move || device_poll(&client, &device, interval, expires));

    Ok(json!({ "method": "device", "userCode": user_code, "verificationUri": uri, "expiresIn": expires }))
}

fn device_poll(client: &str, device: &str, mut interval: u64, expires: u64) {
    let started = std::time::Instant::now();

    while started.elapsed().as_secs() < expires {
        std::thread::sleep(Duration::from_secs(interval));

        /* A newer `connect` or a `disconnect` replaced this one: stop asking. */
        if pending().lock().unwrap_or_else(|poison| poison.into_inner())["state"].as_str() != Some("waiting") {
            return;
        }

        let answer = http()
            .post("https://github.com/login/oauth/access_token")
            .set("accept", "application/json")
            .send_form(&[("client_id", client), ("device_code", device), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")])
            .ok()
            .and_then(|response| json_of(response).ok());
        let Some(answer) = answer else { continue };

        if let Some(token) = answer["access_token"].as_str() {
            match save_token(token) {
                Ok(user) => set_pending(json!({ "state": "done", "login": user["login"] })),
                Err(error) => set_pending(json!({ "state": "failed", "error": error.message })),
            }

            return;
        }

        match answer["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            Some("access_denied") => {
                set_pending(json!({ "state": "failed", "error": "The sign-in was cancelled on GitHub" }));
                return;
            }
            Some(other) => {
                set_pending(json!({ "state": "failed", "error": answer["error_description"].as_str().unwrap_or(other) }));
                return;
            }
            None => {}
        }
    }

    set_pending(json!({ "state": "failed", "error": "The code expired before it was approved - press Connect again" }));
}

/// Git Credential Manager's own sign-in: `git credential fill` for github.com, which opens its browser window.
fn credential_manager_sign_in() -> Result<String, String> {
    use std::io::Write;

    let mut git = git_command().ok_or("Git is not installed")?;
    let mut child = git
        .args(["-c", "credential.helper=", "-c", "credential.helper=manager", "credential", "fill"])
        .env("GCM_INTERACTIVE", "always")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;

    child.stdin.take().ok_or("no stdin")?.write_all(b"protocol=https\nhost=github.com\n\n").map_err(|error| error.to_string())?;

    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);

    text.lines()
        .find_map(|line| line.strip_prefix("password="))
        .map(str::to_string)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if error.is_empty() { "The GitHub sign-in window was closed".to_string() } else { error }
        })
}

/* ------------------------------------------------------------------------------------------------
 * Pushing.
 * ---------------------------------------------------------------------------------------------- */

/// What a push needs to know beyond the folder.
pub struct PushOptions {
    /// The repository name for a folder with no `origin`; the folder's name when absent.
    pub repo: Option<String>,
    pub private: bool,
    pub message: Option<String>,
}

/// A repository name GitHub accepts, from a folder name: `My Shop!` -> `My-Shop`.
pub fn repo_name(folder: &str) -> String {
    let mut name: String = folder.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' }).collect();

    while name.contains("--") {
        name = name.replace("--", "-");
    }

    let name = name.trim_matches(|c| c == '-' || c == '.').to_string();

    if name.is_empty() { "sdc-project".to_string() } else { name }
}

/// The header that lets `git` push with the token, as the `GIT_CONFIG_*` environment (never a command line).
fn auth_env(token: &str) -> Vec<(String, String)> {
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));

    vec![
        ("GIT_CONFIG_COUNT".into(), "2".into()),
        ("GIT_CONFIG_KEY_0".into(), "http.https://github.com/.extraheader".into()),
        ("GIT_CONFIG_VALUE_0".into(), format!("AUTHORIZATION: basic {basic}")),
        /* No other helper is asked - a stale stored password must not win over this account. */
        ("GIT_CONFIG_KEY_1".into(), "credential.helper".into()),
        ("GIT_CONFIG_VALUE_1".into(), String::new()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
    ]
}

/// The token, or the sentence that says to connect first.
fn token_required() -> Result<String, ErrorObject> {
    keychain::get(TOKEN_KEY).ok_or_else(|| ErrorObject::bad_request("GitHub is not connected - Settings → GitHub → Connect"))
}

/// Creates `name` under the account (private unless asked otherwise), or finds it when it already exists.
fn ensure_repository(token: &str, login: &str, name: &str, private: bool) -> Result<(String, bool), ErrorObject> {
    match api(token, "POST", "/user/repos", Some(json!({ "name": name, "private": private, "auto_init": false }))) {
        Ok(created) => Ok((created["clone_url"].as_str().unwrap_or_default().to_string(), true)),
        Err((422, _)) => api(token, "GET", &format!("/repos/{login}/{name}"), None)
            .map(|found| (found["clone_url"].as_str().unwrap_or_default().to_string(), false))
            .map_err(|(_, message)| ErrorObject::bad_request(format!("{login}/{name} exists but cannot be read: {message}"))),
        Err((_, message)) => Err(ErrorObject::bad_request(format!("GitHub did not create {name}: {message}"))),
    }
}

/// The repository's web address from its remote URL: `https://github.com/a/b.git` -> `https://github.com/a/b`.
pub fn web_url(remote: &str) -> String {
    let trimmed = remote.trim().trim_end_matches(".git");

    match trimmed.strip_prefix("git@github.com:") {
        Some(path) => format!("https://github.com/{path}"),
        None => trimmed.to_string(),
    }
}

/// The same steps on either machine: `run(args)` runs `git <args>` in the folder and answers `(ok, stdout+stderr)`.
fn push_with(folder_name: &str, options: &PushOptions, run: &mut dyn FnMut(&[&str]) -> (bool, String)) -> Result<Value, ErrorObject> {
    let token = token_required()?;
    let user = whoami(&token).map_err(ErrorObject::bad_request)?;
    let login = user["login"].as_str().unwrap_or_default().to_string();
    let mut create = |name: &str| ensure_repository(&token, &login, name, options.private);

    push_steps(folder_name, options, &user, &mut create, run)
}

/// Creates (or finds) a repository by name: its clone URL, and whether it was created now.
type CreateRepository<'a> = dyn FnMut(&str) -> Result<(String, bool), ErrorObject> + 'a;

/// The git steps of a push, given the account and a way to create the repository - no network of its own.
fn push_steps(
    folder_name: &str,
    options: &PushOptions,
    user: &Value,
    create: &mut CreateRepository,
    run: &mut dyn FnMut(&[&str]) -> (bool, String),
) -> Result<Value, ErrorObject> {
    let login = user["login"].as_str().unwrap_or_default().to_string();
    let email = format!("{}+{login}@users.noreply.github.com", user["id"].as_u64().unwrap_or(0));
    let display = user["name"].as_str().filter(|name| !name.is_empty()).unwrap_or(&login).to_string();

    if !run(&["rev-parse", "--is-inside-work-tree"]).0 {
        let (ok, text) = run(&["init", "-b", "main"]);

        if !ok {
            return Err(ErrorObject::bad_request(format!("git init failed: {text}")));
        }
    }

    let identity = run(&["config", "user.email"]);
    let mut commit_args: Vec<String> = Vec::new();

    if !identity.0 || identity.1.trim().is_empty() {
        commit_args.extend(["-c".into(), format!("user.name={display}"), "-c".into(), format!("user.email={email}")]);
    }

    let (_, changes) = run(&["status", "--porcelain"]);
    let mut committed = false;

    if !changes.trim().is_empty() {
        let (ok, text) = run(&["add", "-A"]);

        if !ok {
            return Err(ErrorObject::bad_request(format!("git add failed: {text}")));
        }

        let message = options.message.clone().filter(|message| !message.trim().is_empty()).unwrap_or_else(|| "Update from SDC".to_string());

        commit_args.extend(["commit".into(), "-m".into(), message]);

        let args: Vec<&str> = commit_args.iter().map(String::as_str).collect();
        let (ok, text) = run(&args);

        if !ok {
            return Err(ErrorObject::bad_request(format!("git commit failed: {text}")));
        }

        committed = true;
    }

    if !run(&["rev-parse", "--verify", "HEAD"]).0 {
        return Err(ErrorObject::bad_request("The folder has no files to push yet"));
    }

    let (has_origin, origin) = run(&["remote", "get-url", "origin"]);
    let (remote, created) = if has_origin && !origin.trim().is_empty() {
        (origin.trim().to_string(), false)
    } else {
        let name = repo_name(options.repo.as_deref().unwrap_or(folder_name));
        let (url, created) = create(&name)?;
        let (ok, text) = run(&["remote", "add", "origin", &url]);

        if !ok {
            return Err(ErrorObject::bad_request(format!("git remote add failed: {text}")));
        }

        (url, created)
    };

    if !remote.contains("github.com") {
        return Err(ErrorObject::bad_request(format!("This folder's origin is not on GitHub ({remote})")));
    }

    let (_, branch) = run(&["rev-parse", "--abbrev-ref", "HEAD"]);
    let branch = branch.trim().to_string();
    let (ok, text) = run(&["push", "-u", "origin", "HEAD"]);

    if !ok {
        return Err(ErrorObject::bad_request(format!("git push failed: {}", text.trim())));
    }

    Ok(json!({ "url": web_url(&remote), "branch": branch, "created": created, "committed": committed, "login": login }))
}

/// `github.push` for a folder on this computer.
pub fn push_local(root: &Path, options: &PushOptions) -> Result<Value, ErrorObject> {
    let token = token_required()?;
    let env = auth_env(&token);
    let folder = root.file_name().and_then(|name| name.to_str()).unwrap_or("project").to_string();
    let mut run = |args: &[&str]| -> (bool, String) {
        let Some(mut git) = git_command() else {
            return (false, "Git is not installed - Settings → Environment installs it".into());
        };

        match git.current_dir(root).args(args).envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str()))).stdin(Stdio::null()).output() {
            Ok(output) => (output.status.success(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))),
            Err(error) => (false, error.to_string()),
        }
    };

    push_with(&folder, options, &mut run)
}

/// `github.push` for a folder on a host: every `git` runs there, the token travels on the SSH session's stdin.
pub fn push_remote(ssh: &crate::ssh::Ssh, root: &str, options: &PushOptions) -> Result<Value, ErrorObject> {
    let token = token_required()?;
    let exports: String = auth_env(&token).iter().map(|(key, value)| format!("export {key}={}\n", crate::ssh::sh_quote(value))).collect();
    let folder = root.trim_end_matches('/').rsplit('/').next().unwrap_or("project").to_string();
    let cd = format!("cd {} || exit 97\n", crate::ssh::sh_quote(root));
    let mut run = |args: &[&str]| -> (bool, String) {
        let command: Vec<String> = args.iter().map(|arg| crate::ssh::sh_quote(arg)).collect();
        let script = format!("{exports}{cd}git {} 2>&1\n", command.join(" "));

        match ssh.run_with_stdin("sh -s", &script, Duration::from_secs(180)) {
            Ok(output) => (output.ok(), output.stdout),
            Err(error) => (false, error.message),
        }
    };

    push_with(&folder, options, &mut run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_name_becomes_a_repository_name() {
        assert_eq!(repo_name("My Shop!"), "My-Shop");
        assert_eq!(repo_name("sdc"), "sdc");
        assert_eq!(repo_name("  "), "sdc-project");
        assert_eq!(repo_name("site.v2"), "site.v2");
    }

    #[test]
    fn a_remote_becomes_its_web_page() {
        assert_eq!(web_url("https://github.com/a/b.git"), "https://github.com/a/b");
        assert_eq!(web_url("git@github.com:a/b.git"), "https://github.com/a/b");
    }

    #[test]
    fn the_token_is_never_on_a_command_line() {
        let env = auth_env("secret-token");
        let header = &env.iter().find(|(key, _)| key == "GIT_CONFIG_VALUE_0").unwrap().1;

        assert!(header.starts_with("AUTHORIZATION: basic "));
        assert!(!header.contains("secret-token"));
    }

    #[test]
    fn a_new_folder_is_initialised_committed_given_a_repository_and_pushed() {
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let mut run = |args: &[&str]| -> (bool, String) {
            calls.borrow_mut().push(args.join(" "));

            match args {
                ["rev-parse", "--is-inside-work-tree"] => (false, String::new()),
                ["config", "user.email"] => (false, String::new()),
                ["status", "--porcelain"] => (true, "?? index.html
".into()),
                ["remote", "get-url", "origin"] => (false, String::new()),
                ["rev-parse", "--abbrev-ref", "HEAD"] => (true, "main
".into()),
                _ => (true, String::new()),
            }
        };
        let mut created = Vec::new();
        let mut create = |name: &str| {
            created.push(name.to_string());
            Ok(("https://github.com/me/My-Shop.git".to_string(), true))
        };
        let user = json!({ "login": "me", "id": 7, "name": "Me" });
        let options = PushOptions { repo: None, private: true, message: None };
        let answer = push_steps("My Shop", &options, &user, &mut create, &mut run).unwrap();

        assert_eq!(created, ["My-Shop"]);
        assert_eq!(answer["url"], "https://github.com/me/My-Shop");
        assert_eq!(answer["created"], true);
        assert_eq!(answer["committed"], true);

        let calls = calls.borrow();

        assert_eq!(calls[1], "init -b main");
        assert!(calls.iter().any(|call| call == "-c user.name=Me -c user.email=7+me@users.noreply.github.com commit -m Update from SDC"));
        assert!(calls.iter().any(|call| call == "remote add origin https://github.com/me/My-Shop.git"));
        assert_eq!(calls.last().unwrap(), "push -u origin HEAD");
    }

    #[test]
    fn a_folder_with_another_origin_is_not_pushed() {
        let mut run = |args: &[&str]| -> (bool, String) {
            match args {
                ["remote", "get-url", "origin"] => (true, "https://gitlab.com/me/site.git".into()),
                ["status", "--porcelain"] => (true, String::new()),
                _ => (true, String::new()),
            }
        };
        let mut create = |_: &str| -> Result<(String, bool), ErrorObject> { panic!("no repository is created for a folder with an origin") };
        let options = PushOptions { repo: None, private: true, message: None };
        let error = push_steps("site", &options, &json!({ "login": "me", "id": 1 }), &mut create, &mut run).unwrap_err();

        assert!(error.message.contains("not on GitHub"));
    }
}
