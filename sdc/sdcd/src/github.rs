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
//!
//! **Each project its own repository** (1.0.8): one account, many projects - on this computer and on every VPS - and
//! each pushes to its own repository. The binding is the project folder's own `origin` (`.git/config`), so it lives
//! with the folder on whichever machine holds it, two chats in one folder share it, and a folder already cloned from
//! GitHub needs nothing. `github.repo` reads it, `github.link` sets it (an existing repository, or a new one made
//! under the account), and `github.repos` lists the account's repositories to choose from.

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

/// What a push never sends, whatever the folder's own `.gitignore` says (1.0.10): installed packages, build output,
/// caches, SDC's own working data, system junk and key files. A push is "what I worked on", and none of this is -
/// a folder without a `.gitignore` sent 18,370 files of `.pnpm-store` (the report, 2026-10-30). Written as pathspecs,
/// so the same list works on this computer and on a server, and nothing is written into the folder.
const NEVER_SENT: &[&str] = &[
    "node_modules",
    ".pnpm-store",
    ".yarn/cache",
    ".npm",
    "bower_components",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    ".gradle",
    ".sdc/objects",
    ".sdc/vault",
    ".sdc/tx",
    ".sdc/audit",
    ".sdc/padma",
    ".sdc/knowledge",
];

/// Files never sent by name (1.0.10): system junk, and files that hold keys.
const NEVER_SENT_FILES: &[&str] = &[
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    "._*",
    "*.log",
    ".env",
    ".env.local",
    ".env.*.local",
    ".env.production",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "id_rsa*",
    "id_ed25519*",
    "id_ecdsa*",
];

/// What a key looks like inside a file (1.0.10) - the providers SDC talks to, and private keys. A file that holds one is
/// left out of the push and named, so a key typed into `notes.txt` never reaches GitHub. Extended regular expressions,
/// for `git grep -E`, which reads the staged copy wherever the folder is.
const KEY_PATTERNS: &[&str] = &[
    "AIza[0-9A-Za-z_-]{35}",
    "sk-ant-[A-Za-z0-9_-]{20,}",
    "sk-(proj-)?[A-Za-z0-9_-]{32,}",
    "gh[pousr]_[A-Za-z0-9]{36}",
    "github_pat_[A-Za-z0-9_]{40,}",
    "glpat-[A-Za-z0-9_-]{20}",
    "xox[abprs]-[A-Za-z0-9-]{10,}",
    "AKIA[0-9A-Z]{16}",
    "-----BEGIN ([A-Z]+ )?PRIVATE KEY-----",
];

/// A key written as a setting - `GEMINI_API_KEY=…`, `"password": "…"`, `token: …` - whatever the provider (the report's
/// `geminiapi.txt` held one in a form no provider pattern knew). Read only in [`KEY_FILES`], where such a line is a
/// value and not code: `token = getToken()` in a source file is not a key.
const KEY_ASSIGNMENT: &str = "(api[_-]?key|secret|token|passw(or)?d|private[_-]?key|access[_-]?key)[a-z0-9_]*[\"']?[[:space:]]*[:=][[:space:]]*[\"']?[a-z0-9._/+=-]{16,}";

/// The files where a setting line is read as a possible key.
const KEY_FILES: &[&str] = &["*.txt", "*.env", "*.ini", "*.cfg", "*.conf", "*.properties", "*.yaml", "*.yml", "*.toml", "*.json", "*.md"];

/// The pathspecs that stage a folder: all of it, less [`NEVER_SENT`] and [`NEVER_SENT_FILES`].
fn stage_pathspecs() -> Vec<String> {
    let mut specs = vec![".".to_string()];

    specs.extend(NEVER_SENT.iter().map(|dir| format!(":(exclude,glob)**/{dir}/**")));
    specs.extend(NEVER_SENT_FILES.iter().map(|file| format!(":(exclude,glob)**/{file}")));
    specs
}

/// Why a file was left out of a push.
fn skipped(path: &str, why: &str) -> Value {
    json!({ "path": path, "why": why })
}

/// After a folder took a repository's history: the files only GitHub has stay there (1.0.10). They are not in this
/// folder, and git would read that as "deleted" - on this push and every one after. `skip-worktree` tells git their
/// absence here is not a change; nothing is written into the folder. A file the person had and deletes is deleted.
fn keep_theirs(run: &mut dyn FnMut(&[&str]) -> (bool, String)) {
    let (_, missing) = run(&["-c", "core.quotePath=false", "ls-files", "--deleted"]);
    let paths: Vec<&str> = missing.lines().map(str::trim).filter(|path| !path.is_empty()).collect();

    for batch in paths.chunks(200) {
        let mut args = vec!["update-index", "--skip-worktree", "--"];

        args.extend(batch);
        run(&args);
    }
}

/// Stages what the person worked on (1.0.10) and answers what was left out, and why.
fn stage(run: &mut dyn FnMut(&[&str]) -> (bool, String)) -> Result<Vec<Value>, ErrorObject> {
    let specs = stage_pathspecs();
    let mut args: Vec<&str> = vec!["add", "-A", "--"];

    args.extend(specs.iter().map(String::as_str));

    let (ok, text) = run(&args);

    if !ok {
        return Err(ErrorObject::bad_request(friendly_git_error("git add", &text)));
    }

    let mut left_out = Vec::new();

    /* A folder inside that is its own repository is pushed from its own project, not as a broken link in this one. */
    let (_, index) = run(&["-c", "core.quotePath=false", "ls-files", "--stage"]);
    let (_, modules) = run(&["config", "--file", ".gitmodules", "--get-regexp", "path"]);

    for line in index.lines().filter(|line| line.starts_with("160000 ")) {
        let Some(path) = line.split('\t').nth(1) else { continue };

        if !modules.lines().any(|module| module.split_whitespace().nth(1) == Some(path)) {
            unstage(run, path);
            left_out.push(skipped(path, "its own repository - open it as its own project to push it"));
        }
    }

    /* A new or changed file that holds a key stays on this computer. */
    let (_, staged) = run(&["-c", "core.quotePath=false", "diff", "--cached", "--name-only", "--diff-filter=AM"]);

    if !staged.trim().is_empty() {
        let mut grep: Vec<&str> = vec!["-c", "core.quotePath=false", "grep", "--cached", "-l", "-I", "-E"];

        for pattern in KEY_PATTERNS {
            grep.extend(["-e", pattern]);
        }

        let (_, mut hits) = run(&grep);
        /* And a key written as a setting, in the files where such a line is a value. */
        let mut settings: Vec<&str> = vec!["-c", "core.quotePath=false", "grep", "--cached", "-l", "-I", "-i", "-E", "-e", KEY_ASSIGNMENT, "--"];

        settings.extend(KEY_FILES.iter().copied());
        hits.push('\n');
        hits.push_str(&run(&settings).1);

        let changed: Vec<&str> = staged.lines().map(str::trim).collect();
        let mut found: Vec<&str> = hits.lines().map(str::trim).filter(|path| changed.contains(path)).collect();

        found.sort_unstable();
        found.dedup();

        for path in found {
            unstage(run, path);
            left_out.push(skipped(path, "holds a key or password - kept on this computer"));
        }
    }

    Ok(left_out)
}

/// Takes one path's change out of the commit, leaving the file as it is - a file already on GitHub stays there as it was.
fn unstage(run: &mut dyn FnMut(&[&str]) -> (bool, String), path: &str) {
    if !run(&["reset", "-q", "--", path]).0 {
        /* No commit yet: there is nothing to reset to, and the path was only ever added. */
        run(&["rm", "--cached", "-q", "-r", "--", path]);
    }
}

/// The default branch of `origin`, when it has any commits - `None` for an empty repository.
fn remote_branch(run: &mut dyn FnMut(&[&str]) -> (bool, String)) -> Option<String> {
    let (ok, text) = run(&["ls-remote", "--symref", "origin", "HEAD"]);

    if !ok {
        return None;
    }

    text.lines()
        .find_map(|line| line.strip_prefix("ref: refs/heads/")?.split_whitespace().next().map(str::to_string))
}

/// Git's own sentence, in words a person can act on where SDC knows better.
fn friendly_git_error(step: &str, text: &str) -> String {
    let text = text.trim();

    if text.contains("index.lock") {
        return "Another git program is still working in this folder. Wait a moment and push again - if it keeps saying this, close other git tools (an editor's git panel, GitHub Desktop) and try once more.".to_string();
    }

    format!("{step} failed: {text}")
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

    /* Where it goes comes first (1.0.10): what is committed depends on whether that repository already has files. */
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

    /* 1.0.10: a folder with no commit of its own, sent to a repository that already has files - the report's case: a
       folder opened in SDC, a repository picked, Push. What a person means is "put my work on top of what is there",
       exactly as if they had cloned it, copied their files in and committed. So the folder takes the repository's
       history first (`reset`, which changes no file here), and its work becomes one commit on top: a plain
       fast-forward push - nothing on GitHub is overwritten, and a file only GitHub has stays. */
    let fresh = !run(&["rev-parse", "--verify", "HEAD"]).0;

    if fresh && !created {
        if let Some(theirs) = remote_branch(run) {
            let (fetched, text) = run(&["fetch", "-q", "origin", &theirs]);

            if !fetched {
                return Err(ErrorObject::bad_request(friendly_git_error("Reading the repository from GitHub", &text)));
            }

            run(&["symbolic-ref", "HEAD", &format!("refs/heads/{theirs}")]);

            let (reset, text) = run(&["reset", "-q", "FETCH_HEAD"]);

            if !reset {
                return Err(ErrorObject::bad_request(friendly_git_error("Taking the repository's history", &text)));
            }

            keep_theirs(run);
        }
    }

    let left_out = stage(run)?;
    let mut committed = false;

    /* `diff --cached --quiet` fails exactly when something is staged. */
    if !run(&["diff", "--cached", "--quiet"]).0 {
        let message = options.message.clone().filter(|message| !message.trim().is_empty()).unwrap_or_else(|| "Update from SDC".to_string());

        commit_args.extend(["commit".into(), "-q".into(), "-m".into(), message]);

        let args: Vec<&str> = commit_args.iter().map(String::as_str).collect();
        let (ok, text) = run(&args);

        if !ok {
            return Err(ErrorObject::bad_request(friendly_git_error("git commit", &text)));
        }

        committed = true;
    }

    if !run(&["rev-parse", "--verify", "HEAD"]).0 {
        return Err(ErrorObject::bad_request(if left_out.is_empty() {
            "The folder has no files to push yet".to_string()
        } else {
            format!("Nothing to push - the only new files were left out: {}", left_out.iter().filter_map(|row| row["path"].as_str()).collect::<Vec<_>>().join(", "))
        }));
    }

    let (_, branch) = run(&["rev-parse", "--abbrev-ref", "HEAD"]);
    let branch = branch.trim().to_string();
    let (mut ok, mut text) = run(&["push", "-u", "origin", "HEAD"]);

    /* 1.0.8: a repository that already has commits this folder lacks (a README made on GitHub, a push from another
       machine) is caught up first - its commits under this folder's, by a rebase - and pushed again. A rebase that
       meets a conflict is undone, so the folder is exactly as it was, and the person is told. */
    if !ok && rejected_as_behind(&text) {
        let mut pull: Vec<String> = commit_args.iter().take_while(|arg| *arg != "commit").cloned().collect();

        pull.extend(["pull".into(), "--rebase".into(), "--allow-unrelated-histories".into(), "origin".into(), branch.clone()]);

        let args: Vec<&str> = pull.iter().map(String::as_str).collect();
        let (pulled, pull_text) = run(&args);

        if !pulled {
            run(&["rebase", "--abort"]);

            return Err(ErrorObject::bad_request(format!(
                "GitHub has changes this folder does not, and they touch the same lines - nothing was changed here. Ask the agent to merge them (git pull), then push again. ({})",
                pull_text.trim().lines().last().unwrap_or_default()
            )));
        }

        (ok, text) = run(&["push", "-u", "origin", "HEAD"]);
    }

    if !ok {
        return Err(ErrorObject::bad_request(friendly_git_error("git push", &text)));
    }

    Ok(json!({ "url": web_url(&remote), "branch": branch, "created": created, "committed": committed, "login": login, "skipped": left_out }))
}

/// Put before every `git` SDC runs in a project folder: the person chose the folder, so git's ownership check (1.0.9) must
/// not refuse it - a drive formatted exFAT/FAT records no owner ("detected dubious ownership"), and a server folder is
/// often owned by another user. Only these commands are told; the person's own git config is left alone.
const TRUST_FOLDER: &[&str] = &["-c", "safe.directory=*"];

/// How long a git command waits, before each try, for another git in the same folder to let go of its lock: at once,
/// then for up to about eleven seconds in all.
const LOCK_WAITS: &[Duration] = &[Duration::ZERO, Duration::from_millis(500), Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(3), Duration::from_secs(5)];

/// Whether git refused because another git holds the folder's lock (`Unable to create '…/.git/index.lock': File exists`).
fn held_by_another(text: &str) -> bool {
    text.contains(".lock") && (text.contains("File exists") || text.contains("Another git process"))
}

/// Runs one git command, again after a pause while another git holds the folder's lock (1.0.10).
fn with_lock_waits(mut once: impl FnMut() -> (bool, String)) -> (bool, String) {
    let mut answer = (false, String::new());

    for wait in LOCK_WAITS {
        std::thread::sleep(*wait);
        answer = once();

        if answer.0 || !held_by_another(&answer.1) {
            break;
        }
    }

    answer
}

/// `git <args>` in a project folder, answering `(ok, stdout+stderr)`.
type Git<'s> = Box<dyn FnMut(&[&str]) -> (bool, String) + 's>;

/// Where a project folder is: on this computer, or on a host reached over SSH.
pub enum Place<'a> {
    Local(&'a Path),
    Remote(&'a crate::ssh::Ssh, &'a str),
}

impl Place<'_> {
    /// The folder's own name - what a new repository is called when none was chosen.
    fn folder_name(&self) -> String {
        match self {
            Place::Local(root) => root.file_name().and_then(|name| name.to_str()).unwrap_or("project").to_string(),
            Place::Remote(_, root) => root.trim_end_matches('/').rsplit('/').next().filter(|name| !name.is_empty()).unwrap_or("project").to_string(),
        }
    }

    /// `git <args>` in the folder, as `(ok, stdout+stderr)`. With a token, `git` may reach GitHub as the account - the
    /// token only ever in the environment (here) or on the SSH session's stdin (a host), never a command line.
    fn git<'s>(&'s self, token: Option<&str>) -> Git<'s> {
        let env = token.map(auth_env).unwrap_or_else(|| vec![("GIT_TERMINAL_PROMPT".into(), "0".into())]);

        match *self {
            Place::Local(root) => Box::new(move |args: &[&str]| -> (bool, String) {
                if git_command().is_none() {
                    return (false, "Git is not installed - Settings → Environment installs it".into());
                }

                /* Another git in the same folder (an editor's, or SDC's own a moment ago) holds the lock for a moment:
                   wait for it rather than fail (1.0.10). */
                with_lock_waits(|| match git_command() {
                    Some(mut git) => match git.current_dir(root).args(TRUST_FOLDER).args(args).envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str()))).stdin(Stdio::null()).output() {
                        Ok(output) => (output.status.success(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))),
                        Err(error) => (false, error.to_string()),
                    },
                    None => (false, "Git is not installed - Settings → Environment installs it".into()),
                })
            }),
            Place::Remote(ssh, root) => {
                let exports: String = env.iter().map(|(key, value)| format!("export {key}={}\n", crate::ssh::sh_quote(value))).collect();
                let cd = format!("cd {} || exit 97\n", crate::ssh::sh_quote(root));

                Box::new(move |args: &[&str]| -> (bool, String) {
                    let command: Vec<String> = TRUST_FOLDER.iter().chain(args).map(|arg| crate::ssh::sh_quote(arg)).collect();
                    let script = format!("{exports}{cd}command -v git >/dev/null 2>&1 || {{ echo 'Git is not installed on this server'; exit 98; }}\ngit {} 2>&1\n", command.join(" "));

                    with_lock_waits(|| match ssh.run_with_stdin("sh -s", &script, Duration::from_secs(180)) {
                        Ok(output) => (output.ok(), output.stdout),
                        Err(error) => (false, error.message),
                    })
                })
            }
        }
    }
}

/// `github.push` for a project folder, here or on a host (every `git` runs where the folder is).
pub fn push(place: &Place, options: &PushOptions) -> Result<Value, ErrorObject> {
    let token = token_required()?;
    let mut run = place.git(Some(&token));

    push_with(&place.folder_name(), options, &mut *run)
}

/// A push GitHub refused because the branch there has commits this one does not.
fn rejected_as_behind(text: &str) -> bool {
    text.contains("fetch first") || text.contains("non-fast-forward") || text.contains("[rejected]")
}

/* ------------------------------------------------------------------------------------------------
 * Each project its own repository (1.0.8).
 * ---------------------------------------------------------------------------------------------- */

/// What a person typed as a repository.
#[derive(Debug, PartialEq, Eq)]
pub enum RepoSpec {
    /// `owner/name`, `https://github.com/owner/name(.git)` or `git@github.com:owner/name(.git)`.
    Existing { owner: String, name: String },
    /// A bare name: a repository under the account, made when it does not exist yet.
    New { name: String },
}

fn valid_part(part: &str) -> bool {
    !part.is_empty() && part.len() <= 100 && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Reads a repository from what was typed or pasted; `None` when it is not one.
pub fn parse_repo(input: &str) -> Option<RepoSpec> {
    let text = input.trim().trim_end_matches('/');

    if text.is_empty() {
        return None;
    }

    let path = text
        .strip_prefix("git@github.com:")
        .or_else(|| text.strip_prefix("ssh://git@github.com/"))
        .or_else(|| text.strip_prefix("https://github.com/"))
        .or_else(|| text.strip_prefix("http://github.com/"))
        .or_else(|| text.strip_prefix("https://www.github.com/"))
        .or_else(|| text.strip_prefix("github.com/"))
        .unwrap_or(text);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<&str> = path.split('/').collect();

    match parts.as_slice() {
        [name] if !text.contains("://") && !text.contains(':') => {
            let name = repo_name(name);

            valid_part(&name).then_some(RepoSpec::New { name })
        }
        [owner, name, ..] if valid_part(owner) && valid_part(name) && (parts.len() == 2 || text.contains("github.com")) => {
            Some(RepoSpec::Existing { owner: (*owner).to_string(), name: (*name).to_string() })
        }
        _ => None,
    }
}

/// `owner/name` of a GitHub remote, when it is one.
pub fn full_name(remote: &str) -> Option<String> {
    match parse_repo(remote) {
        Some(RepoSpec::Existing { owner, name }) if remote.contains("github.com") => Some(format!("{owner}/{name}")),
        _ => None,
    }
}

/// `github.repo`: the project folder's repository - whether it is one, its `origin`, branch and unsaved changes.
pub fn repo_info(place: &Place) -> Value {
    let mut run = place.git(None);

    info_with(&mut *run)
}

fn info_with(run: &mut dyn FnMut(&[&str]) -> (bool, String)) -> Value {
    let (is_repo, answer) = run(&["rev-parse", "--is-inside-work-tree"]);

    if !is_repo || answer.trim() != "true" {
        let missing = answer.contains("not installed");

        return json!({ "isRepo": false, "gitMissing": missing, "remote": null, "url": null, "fullName": null, "onGitHub": false, "branch": null, "changes": 0 });
    }

    let (has_origin, origin) = run(&["remote", "get-url", "origin"]);
    let remote = if has_origin && !origin.trim().is_empty() { Some(origin.trim().to_string()) } else { None };
    /* A folder with no commit yet has a branch but no `HEAD` to ask: `rev-parse` answers with an error, which used to be
       shown as the branch's name ("HEAD fatal: ambiguous argument 'HEAD'"). `symbolic-ref` knows it either way. */
    let (_, branch) = run(&["symbolic-ref", "--short", "-q", "HEAD"]);
    /* Read-only, so it never takes git's lock from a push running at the same moment (1.0.10). The count leaves out
       what a push never sends, so it is the number a push will actually commit. */
    let specs = stage_pathspecs();
    let mut status: Vec<&str> = vec!["--no-optional-locks", "status", "--porcelain", "--"];

    status.extend(specs.iter().map(String::as_str));

    let (_, changes) = run(&status);
    let full = remote.as_deref().and_then(full_name);

    json!({
        "isRepo": true,
        "gitMissing": false,
        "remote": remote,
        "url": remote.as_deref().map(web_url),
        "fullName": full,
        "onGitHub": full.is_some(),
        "branch": Some(branch.trim()).filter(|branch| !branch.is_empty() && *branch != "HEAD" && !branch.contains(' ')),
        "changes": changes.lines().filter(|line| !line.trim().is_empty()).count(),
    })
}

/// `github.link`: points the project folder's `origin` at a repository - an existing one the account can reach, or a new
/// one made under the account. A folder that is not a repository becomes one first.
pub fn link(place: &Place, input: &str, private: bool) -> Result<Value, ErrorObject> {
    let spec = parse_repo(input).ok_or_else(|| ErrorObject::bad_request(format!("`{input}` is not a GitHub repository - use owner/name, its URL, or a new name")))?;
    let token = token_required()?;
    let user = whoami(&token).map_err(ErrorObject::bad_request)?;
    let login = user["login"].as_str().unwrap_or_default().to_string();
    let mut resolve = |spec: &RepoSpec| -> Result<(String, bool), ErrorObject> {
        match spec {
            RepoSpec::New { name } => ensure_repository(&token, &login, name, private),
            RepoSpec::Existing { owner, name } => match api(&token, "GET", &format!("/repos/{owner}/{name}"), None) {
                Ok(found) => Ok((found["clone_url"].as_str().map(str::to_string).unwrap_or_else(|| format!("https://github.com/{owner}/{name}.git")), false)),
                /* The account's own repository that is not there yet is made; someone else's is a typo or no access. */
                Err((404, _)) if owner.eq_ignore_ascii_case(&login) => ensure_repository(&token, &login, name, private),
                Err((404, _)) => Err(ErrorObject::bad_request(format!("{owner}/{name} was not found, or {login} has no access to it"))),
                Err((_, message)) => Err(ErrorObject::bad_request(format!("GitHub did not answer for {owner}/{name}: {message}"))),
            },
        }
    };
    let mut run = place.git(None);

    link_steps(&spec, &mut resolve, &mut *run)
}

/// The git half of a link, given a way to find or make the repository - no network of its own.
fn link_steps(spec: &RepoSpec, resolve: &mut CreateRepositoryFor, run: &mut dyn FnMut(&[&str]) -> (bool, String)) -> Result<Value, ErrorObject> {
    let (inside, answer) = run(&["rev-parse", "--is-inside-work-tree"]);

    if !inside || answer.trim() != "true" {
        let (ok, text) = run(&["init", "-b", "main"]);

        if !ok {
            return Err(ErrorObject::bad_request(format!("git init failed: {}", text.trim())));
        }
    }

    let (url, created) = resolve(spec)?;
    let (has_origin, origin) = run(&["remote", "get-url", "origin"]);
    let (ok, text) = if has_origin && !origin.trim().is_empty() { run(&["remote", "set-url", "origin", &url]) } else { run(&["remote", "add", "origin", &url]) };

    if !ok {
        return Err(ErrorObject::bad_request(format!("Could not set the repository: {}", text.trim())));
    }

    let mut info = info_with(run);

    info["created"] = json!(created);
    info["previous"] = json!(Some(origin.trim()).filter(|origin| has_origin && !origin.is_empty()));

    Ok(info)
}

/// Finds or makes the repository a spec names: its clone URL, and whether it was made now.
type CreateRepositoryFor<'a> = dyn FnMut(&RepoSpec) -> Result<(String, bool), ErrorObject> + 'a;

/// `github.repos`: the account's repositories, most recently pushed first - what the picker offers.
pub fn repos() -> Result<Value, ErrorObject> {
    let token = token_required()?;
    let list = api(&token, "GET", "/user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member", None)
        .map_err(|(_, message)| ErrorObject::bad_request(format!("GitHub did not list the repositories: {message}")))?;
    let rows: Vec<Value> = list
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    json!({
                        "fullName": row["full_name"],
                        "private": row["private"].as_bool().unwrap_or(false),
                        "url": row["html_url"],
                        "description": row["description"],
                        "pushedAt": row["pushed_at"],
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(json!({ "repos": rows }))
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
                /* Something is staged: `diff --cached --quiet` fails exactly then. */
                ["diff", "--cached", "--quiet"] => (false, String::new()),
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
        assert!(calls.iter().any(|call| call == "-c user.name=Me -c user.email=7+me@users.noreply.github.com commit -q -m Update from SDC"));
        assert!(calls.iter().any(|call| call == "remote add origin https://github.com/me/My-Shop.git"));
        assert_eq!(calls.last().unwrap(), "push -u origin HEAD");
        assert!(calls.iter().any(|call| call.starts_with("add -A -- . :(exclude,glob)**/node_modules/**")), "what a push never sends is left out: {calls:?}");
        assert!(!calls.iter().any(|call| call.starts_with("fetch")), "a repository made now has nothing to take");
    }

    #[test]
    fn what_a_person_types_becomes_a_repository() {
        let existing = |owner: &str, name: &str| Some(RepoSpec::Existing { owner: owner.into(), name: name.into() });

        assert_eq!(parse_repo("me/site"), existing("me", "site"));
        assert_eq!(parse_repo("https://github.com/me/site.git"), existing("me", "site"));
        assert_eq!(parse_repo("https://github.com/me/site/"), existing("me", "site"));
        assert_eq!(parse_repo("https://github.com/me/site/tree/main"), existing("me", "site"));
        assert_eq!(parse_repo("git@github.com:me/site.git"), existing("me", "site"));
        assert_eq!(parse_repo("github.com/me/site"), existing("me", "site"));
        assert_eq!(parse_repo("My Shop"), Some(RepoSpec::New { name: "My-Shop".into() }));
        assert_eq!(parse_repo("https://gitlab.com/me/site"), None);
        assert_eq!(parse_repo("a/b/c"), None);
        assert_eq!(parse_repo("  "), None);
        assert_eq!(full_name("https://github.com/me/site.git").as_deref(), Some("me/site"));
        assert_eq!(full_name("https://gitlab.com/me/site.git"), None);
    }

    #[test]
    fn a_project_is_pointed_at_its_own_repository() {
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let mut run = |args: &[&str]| -> (bool, String) {
            calls.borrow_mut().push(args.join(" "));

            match args {
                ["rev-parse", "--is-inside-work-tree"] => (true, "true\n".into()),
                ["remote", "get-url", "origin"] if calls.borrow().iter().filter(|call| call.starts_with("remote get-url")).count() == 1 => {
                    (true, "https://github.com/me/old.git\n".into())
                }
                ["remote", "get-url", "origin"] => (true, "https://github.com/me/shop.git\n".into()),
                ["symbolic-ref", "--short", "-q", "HEAD"] => (true, "main\n".into()),
                ["--no-optional-locks", "status", "--porcelain", ..] => (true, " M index.html\n".into()),
                _ => (true, String::new()),
            }
        };
        let mut resolve = |spec: &RepoSpec| -> Result<(String, bool), ErrorObject> {
            assert_eq!(spec, &RepoSpec::Existing { owner: "me".into(), name: "shop".into() });
            Ok(("https://github.com/me/shop.git".into(), false))
        };
        let info = link_steps(&parse_repo("me/shop").unwrap(), &mut resolve, &mut run).unwrap();

        assert!(calls.borrow().iter().any(|call| call == "remote set-url origin https://github.com/me/shop.git"));
        assert_eq!(info["fullName"], "me/shop");
        assert_eq!(info["previous"], "https://github.com/me/old.git");
        assert_eq!(info["changes"], 1);
    }

    #[test]
    fn a_push_github_is_ahead_of_is_caught_up_and_pushed_again() {
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let mut run = |args: &[&str]| -> (bool, String) {
            calls.borrow_mut().push(args.join(" "));

            match args {
                ["rev-parse", "--is-inside-work-tree"] | ["rev-parse", "--verify", "HEAD"] => (true, "true".into()),
                ["config", "user.email"] => (true, "me@example.com".into()),
                ["status", "--porcelain"] => (true, String::new()),
                ["remote", "get-url", "origin"] => (true, "https://github.com/me/shop.git".into()),
                ["rev-parse", "--abbrev-ref", "HEAD"] => (true, "main".into()),
                ["push", ..] if calls.borrow().iter().filter(|call| call.starts_with("push")).count() == 1 => {
                    (false, " ! [rejected]        HEAD -> main (fetch first)".into())
                }
                _ => (true, String::new()),
            }
        };
        let mut create = |_: &str| -> Result<(String, bool), ErrorObject> { panic!("the folder has its repository") };
        let options = PushOptions { repo: None, private: true, message: None };
        let answer = push_steps("shop", &options, &json!({ "login": "me", "id": 1 }), &mut create, &mut run).unwrap();

        assert_eq!(answer["url"], "https://github.com/me/shop");
        assert!(calls.borrow().iter().any(|call| call == "pull --rebase --allow-unrelated-histories origin main"));
        assert_eq!(calls.borrow().iter().filter(|call| call.starts_with("push")).count(), 2);
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

    /// Real git, a fake GitHub: `https://github.com/` is a folder on this machine, so the whole push runs offline.
    fn real_git(dir: &Path, hub: &Path) -> impl FnMut(&[&str]) -> (bool, String) {
        let (dir, hub) = (dir.to_path_buf(), format!("{}/", hub.display().to_string().replace('\\', "/")));

        move |args: &[&str]| {
            /* `remote get-url` applies the redirect too; SDC must still see the GitHub address it was given. */
            let redirect = if args.starts_with(&["remote", "get-url"]) { String::new() } else { format!("url.{hub}.insteadOf=https://github.com/") };
            let output = std::process::Command::new("git")
                .current_dir(&dir)
                .args(["-c", "safe.directory=*", "-c", "init.defaultBranch=main"])
                .args(if redirect.is_empty() { Vec::new() } else { vec!["-c", redirect.as_str()] })
                .args(args)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .unwrap();

            (output.status.success(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
        }
    }

    fn git_here(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(["-c", "safe.directory=*", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
            .args(args)
            .output()
            .unwrap();

        assert!(output.status.success(), "git {args:?} in {}: {}", dir.display(), String::from_utf8_lossy(&output.stderr));
    }

    fn write(path: std::path::PathBuf, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// 1.0.10, the report end to end: "a folder opened in SDC, a repository picked, Push" - a folder with no commit of
    /// its own, a repository that already has files, no `.gitignore`, a package store, a key in a text file and a
    /// repository inside. The push puts the work on top of what is there, sends none of the rest, overwrites and
    /// deletes nothing on GitHub - and the next push does the same.
    #[test]
    fn a_folder_opened_anywhere_is_pushed_into_a_repository_that_already_has_files() {
        if !std::process::Command::new("git").arg("--version").output().is_ok_and(|output| output.status.success()) {
            return;
        }

        let root = std::env::temp_dir().join(format!("sdc-push-{}", uuid::Uuid::new_v4()));
        let hub = root.join("hub");
        let bare = hub.join("me").join("shop.git");

        std::fs::create_dir_all(&bare).unwrap();
        git_here(&bare, &["init", "-q", "--bare"]);

        /* GitHub already has two files. */
        let seed = root.join("seed");

        std::fs::create_dir_all(&seed).unwrap();
        git_here(&seed, &["init", "-q"]);
        write(seed.join("README.md"), "# shop\n");
        write(seed.join("docs/guide.md"), "only on GitHub\n");
        git_here(&seed, &["add", "-A"]);
        git_here(&seed, &["commit", "-q", "-m", "first"]);
        git_here(&seed, &["push", "-q", bare.to_str().unwrap(), "HEAD:main"]);

        /* The folder the person opened: their work, and everything a push must not send. */
        let work = root.join("work");

        write(work.join("index.html"), "<h1>my work</h1>\n");
        write(work.join("README.md"), "# shop - my version\n");
        write(work.join(".pnpm-store/v10/files/00/abc"), "package cache\n");
        write(work.join("node_modules/lib/index.js"), "module.exports = 1;\n");
        write(work.join("notes.txt"), "gemini key AIzaSyA1234567890abcdefghijklmnopqrstuv\n");
        /* The report's own file: a key as a setting, in a shape no provider pattern knows. */
        write(work.join("geminiapi.txt"), "GEMINI_API_KEY=AQ.Ab8RN6Lx0aZq-3vT9_kPq2Wm7YcD1eFgHiJkLmNoPqRsTuVw\n");
        /* Code that only mentions a token is the person's work, and is sent. */
        write(work.join("app.js"), "const token = getTokenFromTheServer();\n");
        write(work.join("._index.html"), "macOS junk\n");
        write(work.join(".env"), "SECRET=1\n");
        write(work.join("inner/app.txt"), "a project of its own\n");
        git_here(&work.join("inner"), &["init", "-q"]);
        git_here(&work.join("inner"), &["add", "-A"]);
        git_here(&work.join("inner"), &["commit", "-q", "-m", "inner"]);
        git_here(&work, &["init", "-q"]);
        git_here(&work, &["remote", "add", "origin", "https://github.com/me/shop.git"]);

        let mut run = real_git(&work, &hub);
        let user = json!({ "login": "me", "id": 7, "name": "Me" });
        let options = PushOptions { repo: None, private: true, message: Some("my work".into()) };
        let mut create = |_: &str| -> Result<(String, bool), ErrorObject> { panic!("the repository exists") };
        let answer = push_steps("work", &options, &user, &mut create, &mut run).unwrap_or_else(|error| panic!("{}", error.message));
        let skipped: Vec<String> = answer["skipped"].as_array().unwrap().iter().map(|row| row["path"].as_str().unwrap().to_string()).collect();

        assert!(skipped.contains(&"notes.txt".to_string()) && skipped.contains(&"geminiapi.txt".to_string()) && skipped.contains(&"inner".to_string()), "{skipped:?}");
        assert!(!skipped.contains(&"app.js".to_string()), "code that mentions a token is sent: {skipped:?}");

        /* What GitHub has now. */
        let check = root.join("check");

        git_here(&root, &["clone", "-q", bare.to_str().unwrap(), check.to_str().unwrap()]);

        /* The check clone may turn line ends into CRLF on Windows (`core.autocrlf`); the content is what is compared. */
        let read = |name: &str| std::fs::read_to_string(check.join(name)).unwrap().replace("\r\n", "\n");

        assert_eq!(read("index.html"), "<h1>my work</h1>\n");
        assert_eq!(read("README.md"), "# shop - my version\n", "the folder's work is the newest");
        assert!(check.join("docs/guide.md").exists(), "a file only GitHub had is kept");

        assert!(check.join("app.js").exists());

        for never in [".pnpm-store", "node_modules", "notes.txt", "geminiapi.txt", "._index.html", ".env", "inner"] {
            assert!(!check.join(never).exists(), "{never} was sent");
        }

        let (_, log) = real_git(&check, &hub)(&["log", "--format=%s"]);

        assert_eq!(log.lines().collect::<Vec<_>>(), ["my work", "first"], "on top of GitHub's history, nothing rewritten");
        assert!(!work.join("docs").exists(), "nothing was written into the person's folder");

        /* The next push: one more change, and the file only GitHub has is still not deleted. */
        write(work.join("about.html"), "about\n");

        let options = PushOptions { repo: None, private: true, message: Some("more".into()) };

        push_steps("work", &options, &user, &mut create, &mut run).unwrap_or_else(|error| panic!("{}", error.message));
        git_here(&check, &["pull", "-q"]);

        assert!(check.join("about.html").exists());
        assert!(check.join("docs/guide.md").exists(), "still kept on the second push");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_with_no_commit_shows_its_branch_not_an_error() {
        let mut run = |args: &[&str]| -> (bool, String) {
            match args {
                ["rev-parse", "--is-inside-work-tree"] => (true, "true\n".into()),
                ["symbolic-ref", "--short", "-q", "HEAD"] => (true, "main\n".into()),
                ["rev-parse", ..] => (false, "fatal: ambiguous argument 'HEAD': unknown revision".into()),
                _ => (true, String::new()),
            }
        };

        assert_eq!(info_with(&mut run)["branch"], "main");
    }

    #[test]
    fn a_lock_held_by_another_git_is_waited_for() {
        let mut tries = 0;
        let answer = with_lock_waits(|| {
            tries += 1;

            if tries < 3 {
                (false, "fatal: Unable to create 'F:/x/.git/index.lock': File exists.".into())
            } else {
                (true, "done".into())
            }
        });

        assert_eq!((answer.0, tries), (true, 3));
        assert!(friendly_git_error("git add", "fatal: Unable to create '.git/index.lock': File exists.").contains("Another git program"));
    }
}
