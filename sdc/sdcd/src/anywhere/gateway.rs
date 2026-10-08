//! What a browser may see of the files, and how a chat is started from it (plan 5.3, 5.4).
//!
//! The browser never names a path. It is handed **opaque ids** (`path_id`) by the daemon and sends them back;
//! the daemon keeps the real path behind each id, per connection. So there is no `..` to send, no drive letter
//! to get wrong, and the same page works on Windows, macOS and Linux. An id exists only for a place that is
//! inside an allowed root, and it is made only after the path has been resolved the way the operating system
//! resolves it (symlinks followed): a link that leads out of the root is shown, but gets no id.
//!
//! ## What is a root
//!
//! The folders SDC already has as projects, on each host (`projects` table). Nothing else is reachable: not the
//! home directory, not the drive. (Plan `remote.fs.roots`, default "registered projects only".)
//!
//! ## What is refused
//!
//! * **Hard-blocked names** (`.env*`, keys, `.npmrc`, `credentials`...): never listed, never read. This is
//!   `fs::blocked_reason`, the same rule the AI is held to (DESIGN.md OQ-10).
//! * **Policy-protected paths** (`wp-config.php`, `*.sql`, `backup/**`, `.git/**`, the project's own
//!   `protected_paths`): listed with a lock, and readable only with a **fresh passkey assertion for that exact
//!   file** (the caller, `core`, checks it and passes `critical = true`). Never downloadable (Phase 3).
//!
//! Everything here is read-only. Changing a file is Phase 3 and goes through the Trust Kernel.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};

use crate::sdcp::envelope::ErrorObject;
use crate::trust::policy::Policy;
use crate::DaemonState;

use super::backend;
use super::crypto::{b64u, random};

/// Entries per page of a folder listing.
pub const PAGE: usize = 200;
/// The first window of a file (plan 5.3: 512 KB), and the largest window after it.
pub const READ_WINDOW: usize = 512 * 1024;
/// A picture up to this size is sent as a preview; bigger ones are not (a download in Phase 3).
pub const IMAGE_PREVIEW_MAX: u64 = 2 * 1024 * 1024;
/// Files up to this size also report their SHA-256 (Phase 3 needs it to notice a changed file).
pub const HASH_MAX: u64 = 4 * 1024 * 1024;
/// Ids a connection may hold. A listing of a huge tree cannot grow the daemon without bound.
pub const MAX_IDS: usize = 20_000;
/// How long a project's policy is reused.
const POLICY_TTL: Duration = Duration::from_secs(60);

/// One place the browser may refer to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    /// Absolute, symlinks resolved.
    pub path: String,
    /// The allowed root this place is inside (equal to `path` for a root itself).
    pub root: String,
    pub dir: bool,
    /// The policy pattern that protects it, when one does.
    pub protected: Option<String>,
}

/// The ids one connection has been given.
#[derive(Default)]
pub struct PathTable {
    by_id: HashMap<String, Target>,
    by_place: HashMap<(String, String), String>,
    policies: HashMap<(String, String), (Instant, Policy)>,
}

impl PathTable {
    /// The id for a place, the same one every time within this connection.
    pub fn id_for(&mut self, target: Target) -> Option<String> {
        let key = (target.host.clone(), target.path.clone());

        if let Some(id) = self.by_place.get(&key) {
            return Some(id.clone());
        }

        if self.by_id.len() >= MAX_IDS {
            return None;
        }

        let id = format!("p{}", b64u(&random::<9>()));

        self.by_place.insert(key, id.clone());
        self.by_id.insert(id.clone(), target);

        Some(id)
    }

    pub fn get(&self, id: &str) -> Option<Target> {
        self.by_id.get(id).cloned()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// A handle the core passes around; equality is "the same table".
#[derive(Clone, Default)]
pub struct PathsHandle(pub Arc<Mutex<PathTable>>);

impl std::fmt::Debug for PathsHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PathsHandle")
    }
}

impl PartialEq for PathsHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The methods this module answers, and the level each needs. `core` consults it before anything runs.
pub fn is_gateway_method(method: &str) -> bool {
    matches!(method, "hosts.list" | "fs.list" | "fs.read" | "fs.search" | "fs.git" | "chat.send" | "chat.options")
}

fn bad(message: impl Into<String>) -> ErrorObject {
    ErrorObject::bad_request(message)
}

fn gone() -> ErrorObject {
    ErrorObject::not_found("that place is not available any more; open its folder again")
}

/// What a call needs from the daemon.
pub struct Ctx<'a> {
    pub state: &'a Arc<DaemonState>,
    pub table: &'a PathsHandle,
    /// The core verified a fresh passkey assertion for exactly this read.
    pub critical: bool,
}

/// Runs one gateway method. Blocking (it may wait on SSH): call it on the blocking pool.
pub fn call(ctx: &Ctx, method: &str, params: &Value) -> Result<Value, ErrorObject> {
    match method {
        "hosts.list" => hosts_list(ctx),
        "fs.list" => fs_list(ctx, params),
        "fs.read" => fs_read(ctx, params),
        "fs.search" => fs_search(ctx, params),
        "fs.git" => fs_git(ctx, params),
        "chat.send" => chat_send(ctx, params),
        "chat.options" => chat_options(ctx),
        other => Err(ErrorObject::unsupported(other)),
    }
}

/// The protected pattern of a path, for the core to decide whether a read needs a passkey before it runs.
pub fn protection_of(table: &PathsHandle, path_id: &str) -> Option<(String, String, String)> {
    let target = table.0.lock().ok()?.get(path_id)?;

    target.protected.map(|pattern| (target.host, target.path, pattern))
}

// --- hosts ------------------------------------------------------------------------------------------------------

fn ssh_for(state: &DaemonState, host: &str) -> Result<Option<crate::ssh::Ssh>, ErrorObject> {
    if host == "local" {
        return Ok(None);
    }

    match state.store.host_address(host).map_err(ErrorObject::internal)? {
        Some((Some(target), port)) => Ok(Some(crate::ssh::Ssh::new(crate::auth::remote::SshTarget { user_host: target, port }))),
        _ => Err(ErrorObject::not_found(format!("`{host}` is not a host SDC can reach"))),
    }
}

fn root_target(state: &DaemonState, host: &str, root: &str) -> Target {
    let _ = state;

    Target { host: host.to_string(), path: root.to_string(), root: root.to_string(), dir: true, protected: None }
}

fn hosts_list(ctx: &Ctx) -> Result<Value, ErrorObject> {
    let hosts = ctx.state.store.hosts().map_err(ErrorObject::internal)?;
    let projects = ctx.state.store.projects().map_err(ErrorObject::internal)?;
    let mut table = ctx.table.0.lock().map_err(|_| ErrorObject::internal("paths poisoned"))?;
    let mut out = Vec::new();

    for host in hosts {
        let id = host["hostId"].as_str().unwrap_or_default().to_string();
        let mut roots = Vec::new();

        for project in projects.iter().filter(|project| project["hostId"] == id) {
            let root = project["root"].as_str().unwrap_or_default();

            if root.is_empty() {
                continue;
            }

            /* A root on this machine is shown as the disk resolves it, so the ids made under it are comparable. */
            let path = if id == "local" { normalise_local(Path::new(root)).display().to_string() } else { root.trim_end_matches('/').to_string() };

            if let Some(path_id) = table.id_for(root_target(ctx.state, &id, &path)) {
                roots.push(json!({ "name": project["name"], "path": display_path(&path), "path_id": path_id }));
            }
        }

        out.push(json!({
            "host": id,
            "name": host["name"],
            "type": host["hostType"],
            "status": host["status"],
            "roots": roots,
        }));
    }

    Ok(json!({ "hosts": out }))
}

/// A path as it should be shown: the real one, with Windows' verbatim prefix taken off.
fn display_path(path: &str) -> String {
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}

fn normalise_local(path: &Path) -> PathBuf {
    crate::fs::real_path(path)
}

// --- policy -------------------------------------------------------------------------------------------------------

fn policy_of(ctx: &Ctx, host: &str, root: &str) -> Policy {
    let key = (host.to_string(), root.to_string());

    if let Ok(table) = ctx.table.0.lock() {
        if let Some((at, policy)) = table.policies.get(&key) {
            if at.elapsed() < POLICY_TTL {
                return policy.clone();
            }
        }
    }

    let policy = match ssh_for(ctx.state, host) {
        Ok(Some(ssh)) => Policy::load(Some(root), Some(&ssh)),
        _ => Policy::load_local(Path::new(root)),
    };

    if let Ok(mut table) = ctx.table.0.lock() {
        table.policies.insert(key, (Instant::now(), policy.clone()));
    }

    policy
}

fn protection(policy: &Policy, root: &str, path: &str) -> Option<String> {
    let rooted = crate::trust::policy::relative_to(&display_path(path), Some(&display_path(root)));

    policy.protected(&rooted, None)
}

// --- listing --------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Row {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub modified: Option<i64>,
    /// A symlink that leads outside the root: shown, not followed.
    pub escapes: bool,
    pub link: bool,
}

fn required_target(ctx: &Ctx, params: &Value, key: &str) -> Result<Target, ErrorObject> {
    let id = params.get(key).and_then(Value::as_str).ok_or_else(|| bad(format!("`{key}` is required")))?;

    ctx.table.0.lock().map_err(|_| ErrorObject::internal("paths poisoned"))?.get(id).ok_or_else(gone)
}

fn fs_list(ctx: &Ctx, params: &Value) -> Result<Value, ErrorObject> {
    let target = required_target(ctx, params, "path_id")?;

    if !target.dir {
        return Err(bad("that is a file, not a folder"));
    }

    let offset = cursor_offset(params.get("cursor").and_then(Value::as_str))?;
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(PAGE as u64).clamp(1, PAGE as u64) as usize;
    let policy = policy_of(ctx, &target.host, &target.root);
    let (rows, hidden) = match ssh_for(ctx.state, &target.host)? {
        Some(ssh) => remote_rows(&ssh, &target)?,
        None => local_rows(&target)?,
    };
    let total = rows.len();
    let page: Vec<Row> = rows.into_iter().skip(offset).take(limit).collect();
    let mut table = ctx.table.0.lock().map_err(|_| ErrorObject::internal("paths poisoned"))?;
    let mut entries = Vec::with_capacity(page.len());

    for row in &page {
        let protected = protection(&policy, &target.root, &row.path);
        let path_id = if row.escapes {
            None
        } else {
            table.id_for(Target { host: target.host.clone(), path: row.path.clone(), root: target.root.clone(), dir: row.dir, protected: protected.clone() })
        };

        entries.push(json!({
            "name": row.name,
            "path_id": path_id,
            "dir": row.dir,
            "size": row.size,
            "modified": row.modified,
            "protected": protected.is_some(),
            "link": row.link,
            "outside": row.escapes,
        }));
    }

    let breadcrumbs = breadcrumbs(&mut table, &target);
    let next = (offset + page.len() < total).then(|| URL_SAFE_NO_PAD.encode((offset + page.len()).to_string()));

    Ok(json!({
        "path_id": params["path_id"],
        "path": display_path(&target.path),
        "breadcrumbs": breadcrumbs,
        "entries": entries,
        "total": total,
        "hidden": hidden,
        "next_cursor": next,
    }))
}

fn cursor_offset(cursor: Option<&str>) -> Result<usize, ErrorObject> {
    let Some(text) = cursor else { return Ok(0) };
    let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| bad("bad cursor"))?;

    String::from_utf8(bytes).ok().and_then(|number| number.parse().ok()).ok_or_else(|| bad("bad cursor"))
}

/// The folders from the root down to `target`, each with its own id, so the page can offer "up" and "root".
fn breadcrumbs(table: &mut PathTable, target: &Target) -> Vec<Value> {
    let local = target.host == "local";
    let root = target.root.trim_end_matches(['/', '\\']).to_string();
    let rest = target.path.strip_prefix(&root).unwrap_or("").trim_matches(['/', '\\']).to_string();
    let separator = if local && root.contains('\\') { "\\" } else { "/" };
    let mut crumbs = Vec::new();
    let mut walked = root.clone();
    let name_of = |path: &str| path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path).to_string();

    if let Some(id) = table.id_for(Target { host: target.host.clone(), path: root.clone(), root: root.clone(), dir: true, protected: None }) {
        crumbs.push(json!({ "name": name_of(&root), "path_id": id }));
    }

    for part in rest.split(['/', '\\']).filter(|part| !part.is_empty()) {
        walked = format!("{walked}{separator}{part}");

        if let Some(id) = table.id_for(Target { host: target.host.clone(), path: walked.clone(), root: root.clone(), dir: true, protected: None }) {
            crumbs.push(json!({ "name": part, "path_id": id }));
        }
    }

    crumbs
}

fn modified_ms(meta: &std::fs::Metadata) -> Option<i64> {
    meta.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis() as i64)
}

fn is_inside(root: &Path, path: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn local_rows(target: &Target) -> Result<(Vec<Row>, usize), ErrorObject> {
    let dir = Path::new(&target.path);

    crate::fs::guard(dir)?;

    let root = Path::new(&target.root);
    let read = std::fs::read_dir(dir).map_err(|error| ErrorObject::not_found(format!("{}: {error}", display_path(&target.path))))?;
    let mut rows = Vec::new();
    let mut hidden = 0;

    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if crate::fs::blocked_reason(&path).is_some() {
            hidden += 1;

            continue;
        }

        let link = entry.file_type().map(|kind| kind.is_symlink()).unwrap_or(false);
        let real = if link { crate::fs::real_path(&path) } else { path.clone() };
        let escapes = link && !is_inside(root, &real);
        /* A link to a secret is hidden like the secret itself; a link out of the root is shown but not followed. */
        let leads_to_secret = link && crate::fs::blocked_reason(&real).is_some();

        if leads_to_secret {
            hidden += 1;

            continue;
        }

        let meta = std::fs::metadata(&real).or_else(|_| entry.metadata()).ok();

        rows.push(Row {
            name,
            path: real.display().to_string(),
            dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: meta.as_ref().and_then(modified_ms),
            escapes,
            link,
        });
    }

    sort_rows(&mut rows);

    Ok((rows, hidden))
}

/// Folders first, then names, case-insensitively: what a person expects of a file list.
fn sort_rows(rows: &mut [Row]) {
    rows.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.name.cmp(&b.name)));
}

fn remote_rows(ssh: &crate::ssh::Ssh, target: &Target) -> Result<(Vec<Row>, usize), ErrorObject> {
    crate::ssh::ops::guard(&target.path)?;

    let dir = crate::ssh::ops::remote_expr(&target.path)?;
    /* Like `ssh::ops::list`, with one more column: where a symlink really leads. */
    let script = format!(
        "cd {dir} 2>/dev/null || {{ echo \"no such folder\" >&2; exit 4; }}\n\
         pwd -P\n\
         for entry in * .[!.]* ..?*; do\n\
         \x20 [ -e \"$entry\" ] || [ -L \"$entry\" ] || continue\n\
         \x20 if [ -L \"$entry\" ]; then link=1; real=$(readlink -f -- \"$entry\" 2>/dev/null); else link=0; real=; fi\n\
         \x20 if [ -d \"$entry\" ]; then kind=d; size=0; else kind=f; size=$(wc -c < \"$entry\" 2>/dev/null || echo 0); fi\n\
         \x20 printf '%s\\t%s\\t%s\\t%s\\t%s\\n' \"$kind\" \"$size\" \"$link\" \"$real\" \"$entry\"\n\
         done"
    );
    let output = ssh.run(&script, Duration::from_secs(20))?;

    if !output.ok() {
        return Err(match output.code {
            Some(4) => ErrorObject::not_found(format!("{}: that folder is not there", ssh.label())),
            _ => crate::ssh::ops::failed(ssh, "listing", &output),
        });
    }

    Ok(parse_remote_listing(&output.stdout, &target.root))
}

/// The listing parser, apart from the connection so its rules are testable.
pub fn parse_remote_listing(text: &str, root: &str) -> (Vec<Row>, usize) {
    let mut lines = text.lines();
    let directory = lines.next().unwrap_or_default().trim().to_string();
    let mut rows = Vec::new();
    let mut hidden = 0;
    let root = root.trim_end_matches('/');

    for line in lines {
        let mut fields = line.splitn(5, '\t');
        let (Some(kind), Some(size), Some(link), Some(real), Some(name)) = (fields.next(), fields.next(), fields.next(), fields.next(), fields.next()) else {
            if !line.trim().is_empty() {
                hidden += 1;
            }

            continue;
        };

        if name.is_empty() || name.contains('\t') || name.contains('\n') || crate::fs::blocked_reason(Path::new(name)).is_some() {
            hidden += 1;

            continue;
        }

        let linked = link == "1";

        if linked && crate::fs::blocked_reason(Path::new(real)).is_some() {
            hidden += 1;

            continue;
        }

        let path = if linked && !real.is_empty() { real.to_string() } else { format!("{directory}/{name}") };
        let escapes = linked && !(path == root || path.starts_with(&format!("{root}/")));

        rows.push(Row {
            name: name.to_string(),
            path,
            dir: kind == "d",
            size: size.trim().parse::<u64>().unwrap_or(0),
            modified: None,
            escapes,
            link: linked,
        });
    }

    sort_rows(&mut rows);

    (rows, hidden)
}

// --- reading -----------------------------------------------------------------------------------------------------------

fn mime_of(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();

    [(".png", "image/png"), (".jpg", "image/jpeg"), (".jpeg", "image/jpeg"), (".gif", "image/gif"), (".webp", "image/webp"), (".svg", "image/svg+xml")]
        .iter()
        .find(|(suffix, _)| lower.ends_with(suffix))
        .map(|(_, mime)| *mime)
}

fn looks_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8192)];

    head.contains(&0) || (!head.is_empty() && String::from_utf8_lossy(head).chars().filter(|c| *c == '\u{fffd}').count() * 20 > head.len())
}

/// Cuts a window at a character boundary: `(text, bytes consumed)`.
pub fn text_window(bytes: &[u8]) -> (String, usize) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), bytes.len()),
        Err(error) => {
            let valid = error.valid_up_to();

            /* An incomplete character at the very end is carried to the next window; real garbage is shown as replacement characters. */
            if error.error_len().is_none() {
                (String::from_utf8_lossy(&bytes[..valid]).to_string(), valid)
            } else {
                (String::from_utf8_lossy(bytes).to_string(), bytes.len())
            }
        }
    }
}

fn fs_read(ctx: &Ctx, params: &Value) -> Result<Value, ErrorObject> {
    let target = required_target(ctx, params, "path_id")?;

    if target.dir {
        return Err(bad("that is a folder, not a file"));
    }

    if target.protected.is_some() && !ctx.critical {
        return Err(ErrorObject::new("needs_critical", "this file is protected; opening it needs your passkey"));
    }

    let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let length = params.get("length").and_then(Value::as_u64).unwrap_or(READ_WINDOW as u64).clamp(1, READ_WINDOW as u64) as usize;
    let name = target.path.rsplit(['/', '\\']).next().unwrap_or(&target.path).to_string();

    match ssh_for(ctx.state, &target.host)? {
        Some(ssh) => remote_read(&ssh, &target, &name, offset, length),
        None => local_read(&target, &name, offset, length),
    }
}

fn local_read(target: &Target, name: &str, offset: u64, length: usize) -> Result<Value, ErrorObject> {
    let path = Path::new(&target.path);

    crate::fs::guard(path)?;

    /* The path was resolved when the id was made; it is resolved again now, so a link swapped in since cannot lead out. */
    let real = crate::fs::real_path(path);

    if !is_inside(Path::new(&target.root), &real) {
        return Err(ErrorObject::permission_denied("that file is outside this project"));
    }

    crate::fs::guard(&real)?;

    let mut file = std::fs::File::open(&real).map_err(|error| ErrorObject::not_found(format!("{name}: {error}")))?;
    let meta = file.metadata().map_err(ErrorObject::internal)?;
    let size = meta.len();

    file.seek(SeekFrom::Start(offset)).map_err(ErrorObject::internal)?;

    let mut buffer = vec![0_u8; length];
    let mut filled = 0;

    while filled < length {
        let read = file.read(&mut buffer[filled..]).map_err(ErrorObject::internal)?;

        if read == 0 {
            break;
        }

        filled += read;
    }

    buffer.truncate(filled);

    let hash = (size <= HASH_MAX).then(|| crate::fs::hash_file(&real).ok()).flatten();
    let mime = mime_of(name);

    if offset == 0 && looks_binary(&buffer) {
        let image = mime.filter(|_| size <= IMAGE_PREVIEW_MAX && !name.to_lowercase().ends_with(".svg")).and_then(|mime| {
            let mut whole = Vec::new();

            std::fs::File::open(&real).ok()?.take(IMAGE_PREVIEW_MAX).read_to_end(&mut whole).ok()?;

            Some(json!({ "mime": mime, "data": base64::engine::general_purpose::STANDARD.encode(&whole) }))
        });

        return Ok(json!({ "name": name, "size": size, "binary": true, "mime": mime, "image": image, "sha256": hash, "modified": modified_ms(&meta) }));
    }

    let (text, used) = text_window(&buffer);
    let next = offset + used as u64;

    Ok(json!({
        "name": name,
        "size": size,
        "offset": offset,
        "text": text,
        "binary": false,
        "next_offset": (next < size).then_some(next),
        "sha256": hash,
        "modified": modified_ms(&meta),
    }))
}

fn remote_read(ssh: &crate::ssh::Ssh, target: &Target, name: &str, offset: u64, length: usize) -> Result<Value, ErrorObject> {
    crate::ssh::ops::guard(&target.path)?;

    let expr = crate::ssh::ops::remote_expr(&target.path)?;
    let guard = crate::ssh::ops::link_guard(&expr);
    /* The size first (so a huge file is never pulled), then exactly the window asked for. */
    let size_line = ssh.run(&format!("{guard}wc -c < {expr}"), Duration::from_secs(20))?;

    if size_line.code == Some(crate::ssh::ops::LINK_REFUSED) {
        return Err(ErrorObject::blocked(&format!("{name} leads to a file that holds secrets or keys")));
    }

    if !size_line.ok() {
        return Err(crate::ssh::ops::failed(ssh, "reading", &size_line));
    }

    let size: u64 = size_line.stdout.trim().parse().unwrap_or(0);
    let window = ssh.run(&format!("{guard}tail -c +{} {expr} | head -c {length}", offset + 1), Duration::from_secs(120))?;

    if !window.ok() {
        return Err(crate::ssh::ops::failed(ssh, "reading", &window));
    }

    let bytes = window.stdout.as_bytes();

    if offset == 0 && looks_binary(bytes) {
        return Ok(json!({ "name": name, "size": size, "binary": true, "mime": mime_of(name), "image": Value::Null, "sha256": Value::Null }));
    }

    let (text, used) = text_window(bytes);
    let next = offset + used as u64;

    Ok(json!({ "name": name, "size": size, "offset": offset, "text": text, "binary": false, "next_offset": (next < size).then_some(next), "sha256": Value::Null }))
}

// --- searching -----------------------------------------------------------------------------------------------------------

fn fs_search(ctx: &Ctx, params: &Value) -> Result<Value, ErrorObject> {
    let target = required_target(ctx, params, "path_id")?;
    let query = params.get("query").and_then(Value::as_str).unwrap_or_default().trim().to_string();

    if query.is_empty() || query.len() > 200 {
        return Err(bad("`query` must be 1 to 200 characters"));
    }

    if !target.dir {
        return Err(bad("search starts from a folder"));
    }

    let mode = params.get("mode").and_then(Value::as_str).unwrap_or("both");
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1, 500) as usize;
    let glob = params.get("glob").and_then(Value::as_str).map(str::to_string);
    let remote = ssh_for(ctx.state, &target.host)?;
    let (names, hits) = match &remote {
        Some(ssh) => (
            if mode != "content" { crate::ssh::ops::find_names(ssh, &target.path, &query, limit)? } else { Vec::new() },
            if mode != "name" { crate::ssh::ops::search(ssh, &target.path, &query, glob.as_deref(), limit.min(500))? } else { Vec::new() },
        ),
        None => {
            let root = Path::new(&target.path);

            (
                if mode != "content" { crate::fs::find_names(root, &query, limit)? } else { Vec::new() },
                if mode != "name" { crate::fs::search(root, &query, glob.as_deref(), limit)? } else { Vec::new() },
            )
        }
    };
    let policy = policy_of(ctx, &target.host, &target.root);
    let mut table = ctx.table.0.lock().map_err(|_| ErrorObject::internal("paths poisoned"))?;
    let mut place = |path: &str, dir: bool| -> Option<(String, String, bool)> {
        let real = if remote.is_none() { crate::fs::real_path(Path::new(path)).display().to_string() } else { path.to_string() };
        let inside = if remote.is_none() { is_inside(Path::new(&target.root), Path::new(&real)) } else { real == target.root || real.starts_with(&format!("{}/", target.root.trim_end_matches('/'))) };

        if !inside || crate::fs::blocked_reason(Path::new(&real)).is_some() {
            return None;
        }

        let protected = protection(&policy, &target.root, &real);
        let locked = protected.is_some();
        let id = table.id_for(Target { host: target.host.clone(), path: real.clone(), root: target.root.clone(), dir, protected })?;
        let rel = crate::trust::policy::relative_to(&display_path(&real), Some(&display_path(&target.root)));

        Some((id, rel, locked))
    };
    let mut found_names = Vec::new();

    for item in &names {
        let path = item.get("path").and_then(Value::as_str).unwrap_or_default();
        let dir = item.get("dir").and_then(Value::as_bool).unwrap_or(false);

        if let Some((id, rel, _)) = place(path, dir) {
            let name = rel.rsplit(['/', '\\']).next().unwrap_or(&rel).to_string();

            found_names.push(json!({ "name": name, "path_id": id, "rel": rel, "dir": dir }));
        }
    }

    let mut found_hits = Vec::new();

    for hit in &hits {
        let path = hit.get("path").and_then(Value::as_str).unwrap_or_default();

        /* Lines from protected files are not quoted back: a search must not be a way to read what a read refuses. */
        if let Some((id, rel, protected)) = place(path, false) {
            found_hits.push(json!({
                "path_id": id,
                "rel": rel,
                "line": hit["line"],
                "text": if protected { Value::Null } else { hit["text"].clone() },
                "protected": protected,
            }));
        }
    }

    Ok(json!({ "names": found_names, "hits": found_hits, "mode": mode }))
}

// --- git ---------------------------------------------------------------------------------------------------------------------

fn fs_git(ctx: &Ctx, params: &Value) -> Result<Value, ErrorObject> {
    let target = required_target(ctx, params, "path_id")?;

    if !target.dir {
        return Err(bad("git status starts from a folder"));
    }

    let root = target.root.clone();
    let (branch, dirty, files) = match ssh_for(ctx.state, &target.host)? {
        Some(ssh) => {
            let (branch, dirty) = crate::ssh::ops::git_status(&ssh, &root)?;
            let listing = ssh.run(&format!("cd {} && git status --porcelain=v1 2>/dev/null | head -n 500", crate::ssh::ops::remote_expr(&root)?), Duration::from_secs(20))?;

            (branch, dirty as u64, if listing.ok() { parse_porcelain(&listing.stdout) } else { Vec::new() })
        }
        None => {
            /* A verbatim Windows path (\\?\C:\...) is fine for the file system and not for every program; git gets the plain one. */
            let plain = display_path(&root);
            let output = crate::host::program::command("git").map(|mut command| command.args(["status", "--porcelain=v1"]).current_dir(&plain).output());
            let files = match output {
                Some(Ok(out)) if out.status.success() => parse_porcelain(&String::from_utf8_lossy(&out.stdout)),
                _ => Vec::new(),
            };
            /* A repository with no commit yet has no HEAD, and `git::status` refuses it; the counts come from the listing then. */
            let (branch, dirty) = crate::git::status(Path::new(&plain)).map(|(branch, dirty)| (branch, dirty as u64)).unwrap_or_else(|_| (String::new(), files.len() as u64));

            (branch, dirty, files)
        }
    };
    let mut table = ctx.table.0.lock().map_err(|_| ErrorObject::internal("paths poisoned"))?;
    let local = target.host == "local";
    let rows: Vec<Value> = files
        .into_iter()
        .take(500)
        .map(|(status, rel)| {
            let joined = if local { Path::new(&root).join(&rel).display().to_string() } else { format!("{}/{}", root.trim_end_matches('/'), rel) };
            let real = if local { crate::fs::real_path(Path::new(&joined)).display().to_string() } else { joined };
            let blocked = crate::fs::blocked_reason(Path::new(&real)).is_some();
            let path_id = (!blocked)
                .then(|| table.id_for(Target { host: target.host.clone(), path: real, root: root.clone(), dir: false, protected: None }))
                .flatten();

            json!({ "status": status, "rel": rel, "path_id": path_id, "hidden": blocked })
        })
        .collect();

    Ok(json!({ "branch": branch, "dirty": dirty, "files": rows }))
}

/// `git status --porcelain=v1`: `(status, path)`.
pub fn parse_porcelain(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| line.len() > 3)
        .map(|line| {
            let status = line[..2].trim().to_string();
            let path = line[3..].rsplit(" -> ").next().unwrap_or(&line[3..]).trim_matches('"').to_string();

            (if status.is_empty() { "?".into() } else { status }, path)
        })
        .collect()
}

// --- chat ---------------------------------------------------------------------------------------------------------------------

/// What a chat can be started with: the subscription CLIs that are signed in, the API providers that have a key,
/// and local models. Each choice carries the `engine`, `provider` and `model` to send back, so the page never has to
/// know how SDC maps one to the other. Nothing secret is in it: a key is never part of a provider's listing.
fn chat_options(ctx: &Ctx) -> Result<Value, ErrorObject> {
    let providers = crate::providers::list(&ctx.state.store);
    let connected = |id: &str| providers.iter().any(|provider| provider["id"] == id && provider["status"] == "connected");
    let mut choices = Vec::new();

    for (provider, engine, label) in [("claude", "claude_code", "Claude (your subscription)"), ("openai", "codex", "ChatGPT / Codex (your subscription)"), ("gemini", "gemini", "Gemini (your subscription)")] {
        if connected(provider) {
            choices.push(json!({ "label": label, "engine": engine, "provider": Value::Null, "model": crate::duel::model_for(engine), "group": "subscription" }));
        }
    }

    let models = crate::providers::models::list(&ctx.state.store, None, false)?;

    for model in models["models"].as_array().cloned().unwrap_or_default() {
        let provider = model["providerId"].as_str().unwrap_or_default();

        if !connected(provider) {
            continue;
        }

        let (engine, group) = match provider {
            "ollama" => ("ollama", "local"),
            "claude" | "openai" | "gemini" => continue,
            _ => ("native_api", "api"),
        };

        choices.push(json!({
            "label": format!("{} · {}", model["providerLabel"].as_str().unwrap_or(provider), model["name"].as_str().unwrap_or_default()),
            "engine": engine,
            "provider": provider,
            "model": model["id"],
            "group": group,
        }));
    }

    Ok(json!({ "choices": choices, "selected": models["selected"] }))
}

/// The longest message a browser may send as a prompt.
pub const MAX_PROMPT_CHARS: usize = 20_000;
pub const MAX_ATTACHMENTS: usize = 25;

fn chat_send(ctx: &Ctx, params: &Value) -> Result<Value, ErrorObject> {
    let text = params.get("text").and_then(Value::as_str).unwrap_or_default().trim().to_string();

    if text.is_empty() {
        return Err(bad("write something to send"));
    }

    if text.chars().count() > MAX_PROMPT_CHARS {
        return Err(bad(format!("a message is at most {MAX_PROMPT_CHARS} characters")));
    }

    let ids: Vec<&str> = params.get("attachments").and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).collect()).unwrap_or_default();

    if ids.len() > MAX_ATTACHMENTS {
        return Err(bad(format!("at most {MAX_ATTACHMENTS} files can be attached")));
    }

    /* Attachments are paths, not contents: the AI reads them itself, on the machine that has them. */
    let mut attached = Vec::new();
    let mut host: Option<String> = None;
    let mut root: Option<String> = None;

    {
        let table = ctx.table.0.lock().map_err(|_| ErrorObject::internal("paths poisoned"))?;

        for id in &ids {
            let target = table.get(id).ok_or_else(gone)?;

            if target.protected.is_some() {
                return Err(ErrorObject::permission_denied("a protected file cannot be attached; the AI would read it without your passkey"));
            }

            if host.as_deref().is_some_and(|h| h != target.host) || root.as_deref().is_some_and(|r| r != target.root) {
                return Err(bad("attach files from one project at a time"));
            }

            host = Some(target.host.clone());
            root = Some(target.root.clone());
            attached.push(target);
        }
    }

    let session_id = params.get("session_id").and_then(Value::as_str).map(str::to_string);
    let (session_id, session_host) = match session_id {
        Some(id) => (id, host.clone()),
        None => {
            let place = match (&host, &root) {
                (Some(host), Some(root)) => Some((host.clone(), root.clone())),
                _ => params.get("root_id").and_then(Value::as_str).and_then(|id| ctx.table.0.lock().ok()?.get(id)).map(|t| (t.host, t.root)),
            };
            let (host, root) = place.ok_or_else(|| bad("choose a project first (a folder, or a file in it)"))?;
            let project = ctx
                .state
                .store
                .projects()
                .map_err(ErrorObject::internal)?
                .into_iter()
                .find(|project| project["hostId"] == host && normalised_root(&host, project["root"].as_str().unwrap_or_default()) == root)
                .ok_or_else(|| bad("that folder is not a project"))?;
            let title: String = text.lines().next().unwrap_or("New chat").chars().take(60).collect();
            let opened = backend::call(ctx.state, "session.open", json!({ "hostId": host, "title": title, "projectId": project["projectId"] }))?;

            (opened["sessionId"].as_str().unwrap_or_default().to_string(), Some(host))
        }
    };

    let mut prompt = text.clone();

    if !attached.is_empty() {
        prompt.push_str("\n\nFiles to look at:\n");

        for target in &attached {
            prompt.push_str(&format!("@{}\n", display_path(&target.path)));
        }
    }

    /* A phone is not at the desk: every action asks, unless the project is stricter still. Never Auto. */
    let autonomy = match params.get("autonomy").and_then(Value::as_str) {
        Some("pro") => "pro",
        _ => "ask",
    };
    let mut start = json!({
        "sessionId": session_id,
        "prompt": prompt,
        "agent": params.get("agent").and_then(Value::as_bool).unwrap_or(true),
        "autonomy": autonomy,
    });

    for key in ["engine", "model", "provider"] {
        if let Some(value) = params.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()) {
            start[key] = json!(value);
        }
    }

    if let Some(host) = &session_host {
        start["hostId"] = json!(host);
    }

    let started = backend::call(ctx.state, "engine.start", start)?;

    Ok(json!({ "session_id": session_id, "turn_id": started["turnId"], "attached": attached.len() }))
}

fn normalised_root(host: &str, root: &str) -> String {
    if host == "local" {
        normalise_local(Path::new(root)).display().to_string()
    } else {
        root.trim_end_matches('/').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn target(root: &Path, path: &Path, dir: bool) -> Target {
        Target { host: "local".into(), path: path.display().to_string(), root: root.display().to_string(), dir, protected: None }
    }

    #[test]
    fn an_id_is_stable_and_unguessable_per_place() {
        let mut table = PathTable::default();
        let place = Target { host: "local".into(), path: "/p/a".into(), root: "/p".into(), dir: true, protected: None };
        let one = table.id_for(place.clone()).unwrap();
        let two = table.id_for(place.clone()).unwrap();
        let other = table.id_for(Target { path: "/p/b".into(), ..place }).unwrap();

        assert_eq!(one, two);
        assert_ne!(one, other);
        assert!(one.starts_with('p') && one.len() > 10);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn a_connection_cannot_hold_unlimited_ids() {
        let mut table = PathTable::default();

        for n in 0..MAX_IDS {
            assert!(table.id_for(Target { host: "local".into(), path: format!("/p/{n}"), root: "/p".into(), dir: false, protected: None }).is_some());
        }

        assert!(table.id_for(Target { host: "local".into(), path: "/p/one-more".into(), root: "/p".into(), dir: false, protected: None }).is_none());
    }

    #[test]
    fn a_listing_is_sorted_folders_first_and_hides_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fs::real_path(dir.path());

        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("zeta.txt"), "z").unwrap();
        fs::write(root.join("Alpha.txt"), "a").unwrap();
        fs::write(root.join(".env"), "SECRET=1").unwrap();
        fs::write(root.join("server.pem"), "key").unwrap();

        let (rows, hidden) = local_rows(&target(&root, &root, true)).unwrap();
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();

        assert_eq!(names, vec!["src", "Alpha.txt", "zeta.txt"], "folders first, then case-insensitive names");
        assert_eq!(hidden, 2, ".env and the key are counted, not named");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_root_is_shown_but_gets_no_way_in() {
        let outside = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fs::real_path(dir.path());

        fs::write(outside.path().join("passwd"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).unwrap();
        std::os::unix::fs::symlink(root.join("real.txt"), root.join("inside")).unwrap();
        fs::write(root.join("real.txt"), "r").unwrap();

        let (rows, _) = local_rows(&target(&root, &root, true)).unwrap();
        let escape = rows.iter().find(|row| row.name == "escape").unwrap();
        let inside = rows.iter().find(|row| row.name == "inside").unwrap();

        assert!(escape.escapes && escape.link);
        assert!(!inside.escapes && inside.link);
    }

    #[test]
    fn a_link_to_a_secret_is_hidden_like_the_secret() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fs::real_path(dir.path());

        fs::write(root.join(".env"), "X=1").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join(".env"), root.join("notes")).unwrap();

        let (rows, hidden) = local_rows(&target(&root, &root, true)).unwrap();

        assert!(rows.iter().all(|row| row.name != ".env" && row.name != "notes"));
        assert!(hidden >= 1);
    }

    #[test]
    fn a_text_file_is_read_in_windows_and_never_split_inside_a_character() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fs::real_path(dir.path());
        let file = root.join("bn.txt");
        let body = "ডাটাবেস ".repeat(2000);

        fs::write(&file, &body).unwrap();

        let target = target(&root, &file, false);
        let first = local_read(&target, "bn.txt", 0, 1000).unwrap();
        let used = first["next_offset"].as_u64().unwrap();
        let second = local_read(&target, "bn.txt", used, 1000).unwrap();
        let glued = format!("{}{}", first["text"].as_str().unwrap(), second["text"].as_str().unwrap());

        assert!(body.starts_with(&glued), "two windows read back to back are the start of the file, with no broken characters");
        assert!(!glued.contains('\u{fffd}'));
        assert_eq!(first["size"].as_u64().unwrap(), body.len() as u64);
        assert!(first["sha256"].is_string());
    }

    #[test]
    fn a_binary_file_is_reported_not_dumped() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fs::real_path(dir.path());
        let file = root.join("blob.bin");

        fs::write(&file, [0_u8, 1, 2, 3, 0, 255, 254]).unwrap();

        let read = local_read(&target(&root, &file, false), "blob.bin", 0, 1000).unwrap();

        assert_eq!(read["binary"], true);
        assert!(read.get("text").is_none());
    }

    #[test]
    fn a_small_picture_comes_back_as_a_preview() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fs::real_path(dir.path());
        let file = root.join("logo.png");

        fs::write(&file, [0x89_u8, b'P', b'N', b'G', 0, 0, 0, 13, 0xff, 0xfe]).unwrap();

        let read = local_read(&target(&root, &file, false), "logo.png", 0, 1000).unwrap();

        assert_eq!(read["image"]["mime"], "image/png");
        assert!(read["image"]["data"].as_str().unwrap().len() > 4);
    }

    #[test]
    fn a_file_outside_the_root_is_refused_even_with_an_id() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root = crate::fs::real_path(dir.path());
        let file = crate::fs::real_path(other.path()).join("elsewhere.txt");

        fs::write(&file, "nope").unwrap();

        let error = local_read(&target(&root, &file, false), "elsewhere.txt", 0, 100).unwrap_err();

        assert_eq!(error.code, "permission_denied");
    }

    #[test]
    fn a_secret_file_is_refused_by_name_even_inside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fs::real_path(dir.path());
        let file = root.join(".env");

        fs::write(&file, "TOKEN=1").unwrap();

        assert_eq!(local_read(&target(&root, &file, false), ".env", 0, 100).unwrap_err().code, "blocked_path");
    }

    #[test]
    fn the_remote_listing_parser_resolves_links_and_counts_what_it_cannot_show() {
        let text = "/srv/shop\nd\t0\t0\t\tapp\nf\t120\t0\t\tindex.php\nf\t5\t0\t\t.env\nf\t9\t1\t/etc/passwd\toutside\nf\t3\t1\t/srv/shop/app/page.tsx\tinside\nf\t1\t1\t/home/u/.ssh/id_rsa\tsneaky\nbroken line\n";
        let (rows, hidden) = parse_remote_listing(text, "/srv/shop");
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();

        assert_eq!(names, vec!["app", "index.php", "inside", "outside"]);
        assert!(rows.iter().find(|row| row.name == "outside").unwrap().escapes);
        assert!(!rows.iter().find(|row| row.name == "inside").unwrap().escapes);
        assert_eq!(rows.iter().find(|row| row.name == "inside").unwrap().path, "/srv/shop/app/page.tsx");
        assert_eq!(hidden, 3, ".env, the link to a private key, and the line that was not a row");
    }

    #[test]
    fn a_root_that_only_shares_a_prefix_is_not_inside() {
        let (rows, _) = parse_remote_listing("/srv/shop\nf\t1\t1\t/srv/shop-old/a\tneighbour\n", "/srv/shop");

        assert!(rows[0].escapes, "/srv/shop-old is not under /srv/shop");
    }

    #[test]
    fn porcelain_lines_are_parsed() {
        let files = parse_porcelain(" M src/a.ts\n?? new.txt\nA  added.rs\nR  old.rs -> new.rs\n");

        assert_eq!(files[0], ("M".into(), "src/a.ts".into()));
        assert_eq!(files[1], ("??".into(), "new.txt".into()));
        assert_eq!(files[2], ("A".into(), "added.rs".into()));
        assert_eq!(files[3], ("R".into(), "new.rs".into()));
    }

    #[test]
    fn cursors_round_trip_and_reject_garbage() {
        assert_eq!(cursor_offset(None).unwrap(), 0);
        assert_eq!(cursor_offset(Some(&URL_SAFE_NO_PAD.encode("400"))).unwrap(), 400);
        assert!(cursor_offset(Some("!!!")).is_err());
        assert!(cursor_offset(Some(&URL_SAFE_NO_PAD.encode("-1x"))).is_err());
    }

    #[test]
    fn breadcrumbs_run_from_the_root_down() {
        let mut table = PathTable::default();
        let place = Target { host: "h".into(), path: "/srv/shop/app/checkout".into(), root: "/srv/shop".into(), dir: true, protected: None };
        let crumbs = breadcrumbs(&mut table, &place);
        let names: Vec<&str> = crumbs.iter().map(|c| c["name"].as_str().unwrap()).collect();

        assert_eq!(names, vec!["shop", "app", "checkout"]);
    }

    #[test]
    fn windows_are_cut_at_character_boundaries() {
        let text = "aé".as_bytes();

        assert_eq!(text_window(&text[..2]), ("a".to_string(), 1), "half of é is carried to the next window");
        assert_eq!(text_window(text), ("aé".to_string(), 3));
    }

    #[test]
    fn only_the_gateway_methods_are_claimed() {
        for method in ["hosts.list", "fs.list", "fs.read", "fs.search", "fs.git", "chat.send"] {
            assert!(is_gateway_method(method));
        }

        for method in ["fs.write", "fs.delete", "shell.run", "host.status"] {
            assert!(!is_gateway_method(method), "{method} is not a gateway method");
        }
    }
}
