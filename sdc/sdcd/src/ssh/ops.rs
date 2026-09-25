//! What SDC does *inside* the machine it reached (0.7.13).
//!
//! The connection (`super`) and the trust decision (`super::hostkey`) are worthless on their own: a
//! "connected" VPS with no folder, no file and no branch is a dot on a green circle. This module is the
//! part a person actually uses - and it is the part that used to be missing entirely, because `fs.*`,
//! `git.*` and `shell.run` all resolved their paths on the machine the daemon runs on.
//!
//! ## One shape per operation, matching the local one
//!
//! Every function here answers with the **same JSON the local implementation answers with**
//! (`crate::fs::list`, `crate::git::status`, `pty::run_once`), because the app draws one tree, one
//! Preview editor and one Diff button, and a second shape would mean a second implementation of each
//! in the window. `fs.list` on a VPS and `fs.list` on a laptop are the same method with a different
//! `hostId`, and the tree cannot tell them apart - which is the point.
//!
//! ## Everything dynamic is quoted
//!
//! A remote command is a string a *shell* parses, so every path and every argument goes through
//! [`super::sh_quote`] first (rule 6 of `docs/REMOTE.md`). Paths are named absolutely (`/srv/app`) or
//! from the host's home (`~/app`), and a relative path is refused with a sentence rather than guessed
//! at: `cd models` would resolve against whatever directory the remote shell happened to start in.
//!
//! ## The guard runs here too
//!
//! `crate::fs::blocked_reason` is a *name* rule, so `.env`, `*.pem`, `id_rsa` and `credentials` are
//! refused on a VPS exactly as they are on this machine - and a listing counts them as hidden rather
//! than quietly leaving them out.

use std::time::Duration;

use serde_json::{json, Value};

use super::{sh_quote, Ssh, SshOutput};

use crate::sdcp::envelope::ErrorObject;

/// A listing, a stat, a `test -d`: seconds.
const QUICK: Duration = super::QUICK;

/// A read or a write: a megabyte over a slow link is not a ten-second job.
const TRANSFER: Duration = Duration::from_secs(120);

/// A remote path, as a shell expression.
///
/// Two forms are accepted and nothing else:
///
/// * `/srv/app` - absolute, quoted;
/// * `~/app` - expanded by the remote shell from `$HOME`, so "the app folder in my home directory"
///   is nameable without this daemon having to know what that home is called.
///
/// A relative path is refused. A wrong folder that *works* is worse than a refusal that says why.
pub fn remote_expr(path: &str) -> Result<String, ErrorObject> {
    if path.starts_with('/') {
        return Ok(sh_quote(path));
    }

    if let Some(rest) = path.strip_prefix("~/") {
        return Ok(format!("\"$HOME\"/{}", sh_quote(rest)));
    }

    Err(ErrorObject::bad_request(format!(
        "`{path}` is not an absolute path. A host's folder is named from its root (`/srv/app`) or from its home (`~/app`) - a relative path has no folder to be relative to."
    )))
}

/// A failure sentence that names the host as well as the reason.
pub fn failed(ssh: &Ssh, what: &str, output: &SshOutput) -> ErrorObject {
    ErrorObject::internal(format!("{}: {what} failed - {}", ssh.label(), output.reason()))
}


/* --------------------------------------------------------------------------------------------
 * Folders
 * ------------------------------------------------------------------------------------------ */

/// `fs.list` on a host: one level, the same rows the local listing returns, and how many names the
/// guard hid.
///
/// The listing comes from a `for` loop over the shell's own globs rather than from parsing `ls -l`,
/// and that is deliberate: `printf '%s\t%s\t%s\n'` with a fixed separator is a format this daemon
/// defines, whereas `ls -l`'s columns differ between GNU and BSD (and a name with a space, a date or
/// a `+` in it is a parsing bug waiting to happen). The first line of the answer is the **absolute**
/// directory as the host resolved it, so a `~/app` path comes back as `/home/me/app` and the window's
/// tree can show and re-open absolute paths from then on.
pub fn list(ssh: &Ssh, path: &str) -> Result<(String, Vec<Value>, i64), ErrorObject> {
    guard(path)?;

    let dir = remote_expr(path)?;
    let script = format!(
        "cd {dir} 2>/dev/null || {{ echo \"no such folder\" >&2; exit 4; }}\n\
         pwd\n\
         for entry in * .[!.]* ..?*; do\n\
         \x20 [ -e \"$entry\" ] || continue\n\
         \x20 if [ -d \"$entry\" ]; then kind=d; size=0; else kind=f; size=$(wc -c < \"$entry\" 2>/dev/null || echo 0); fi\n\
         \x20 printf '%s\\t%s\\t%s\\n' \"$kind\" \"$size\" \"$entry\"\n\
         done"
    );

    let output = ssh.run(&script, QUICK)?;

    if !output.ok() {
        return Err(match output.code {
            Some(4) => ErrorObject::not_found(format!("{}: `{path}` is not a folder on that host", ssh.label())),
            _ => failed(ssh, "listing", &output),
        });
    }

    Ok(parse_listing(&output.stdout))
}

/// The listing parser, separated from the connection so its rules can be tested without one.
///
/// The hidden count is not decoration: a name with a tab or a newline in it cannot be represented in
/// a tab-separated line, so it is **counted** rather than dropped silently - the same honesty the
/// guard's own count gets (`3 names hidden`).
fn parse_listing(text: &str) -> (String, Vec<Value>, i64) {
    let mut lines = text.lines();
    let directory = lines.next().unwrap_or_default().trim().to_string();
    let mut rows = Vec::new();
    let mut hidden = 0_i64;

    for line in lines {
        let mut fields = line.splitn(3, '\t');

        let (Some(kind), Some(size), Some(name)) = (fields.next(), fields.next(), fields.next()) else {
            if !line.trim().is_empty() {
                hidden += 1;
            }

            continue;
        };

        if name.contains('\t') || name.contains('\n') || name.is_empty() {
            hidden += 1;
            continue;
        }

        /* The same guard the local listing uses, on the name: it is a name rule, so it holds on
           `/srv/app/.env` exactly as it holds on `H:\app\.env`. */
        if crate::fs::blocked_reason(std::path::Path::new(name)).is_some() {
            hidden += 1;
            continue;
        }

        rows.push(json!({
            "name": name,
            "path": format!("{directory}/{name}"),
            "dir": kind == "d",
            "size": size.trim().parse::<i64>().unwrap_or(0).max(0),
        }));
    }

    rows.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));

    (directory, rows, hidden)
}

/// The host's home directory - the folder a person browses first, and the one a `~/…` path means.
pub fn home(ssh: &Ssh) -> Result<String, ErrorObject> {
    let output = ssh.run("cd \"$HOME\" 2>/dev/null && pwd", QUICK)?;

    if !output.ok() {
        return Err(failed(ssh, "reading the home directory", &output));
    }

    Ok(output.stdout.trim().to_string())
}

/// `test -d`, for `project.add` on a host: the folder is validated by the machine that has it.
pub fn is_dir(ssh: &Ssh, path: &str) -> Result<bool, ErrorObject> {
    let output = ssh.run(&format!("test -d {}", remote_expr(path)?), QUICK)?;

    if output.timed_out {
        return Err(failed(ssh, "checking the folder", &output));
    }

    Ok(output.code == Some(0))
}

/// Is the host's home directory writable?
///
/// It is asked because the daemon keeps two things under it on a host - the pid file a turn can be killed
/// by, and the shadow repository a checkpoint commits to (`ssh::ops::state_dir`) - so a read-only home
/// means a chat there can read files but not remember them.
pub fn writable_home(ssh: &Ssh) -> Result<bool, ErrorObject> {
    let output = ssh.run("test -w \"$HOME\"", QUICK)?;

    if output.timed_out {
        return Err(failed(ssh, "checking the home directory", &output));
    }

    Ok(output.code == Some(0))
}

/* --------------------------------------------------------------------------------------------
 * Files
 * ------------------------------------------------------------------------------------------ */

/// `fs.list`'s guard, for a single path: the same refusal the local read and write get.
fn guard(path: &str) -> Result<(), ErrorObject> {
    match crate::fs::blocked_reason(std::path::Path::new(path)) {
        Some(reason) => Err(ErrorObject::blocked(&format!("{path} was refused because {reason}"))),
        None => Ok(()),
    }
}

/// `fs.read` on a host: the text, its hash, its real size, and whether the text was cut.
///
/// Two round trips for a small file (`wc -c`, then `cat`) and three for a large one, which is the
/// trade this makes: the size is known **before** the content is asked for, so a 200 MB log is never
/// pulled over the link to be thrown away. The hash of a small file is computed here from the exact
/// bytes that arrived; the hash of a large one is the host's own `sha256sum`, because a hash of the
/// first megabyte would be a hash of something that is not the file.
pub fn read(ssh: &Ssh, path: &str, cap: usize) -> Result<Value, ErrorObject> {
    guard(path)?;

    let expr = remote_expr(path)?;
    let sized = ssh.run(&format!("wc -c < {expr} 2>/dev/null || echo 0"), QUICK)?;

    if !sized.ok() {
        return Err(failed(ssh, "reading the file size", &sized));
    }

    let bytes: u64 = sized.stdout.trim().parse().unwrap_or(0);

    if bytes == 0 {
        /* Zero bytes *and* a `wc` that failed are two different things, and only one of them is a
           file: `test -f` is what tells them apart. */
        let exists = ssh.run(&format!("test -f {expr}"), QUICK)?;

        if exists.code != Some(0) {
            return Err(ErrorObject::not_found(format!("{}: `{path}` is not a readable file on that host", ssh.label())));
        }
    }

    if bytes <= cap as u64 {
        let output = ssh.run(&format!("cat {expr}"), TRANSFER)?;

        if !output.ok() {
            return Err(failed(ssh, "reading the file", &output));
        }

        return Ok(json!({
            "path": path,
            "text": output.stdout,
            "sha256": crate::fs::hash(output.stdout.as_bytes()),
            "bytes": bytes,
            "truncated": false,
        }));
    }

    let output = ssh.run(&format!("head -c {cap} {expr}"), TRANSFER)?;

    if !output.ok() {
        return Err(failed(ssh, "reading the file", &output));
    }

    Ok(json!({
        "path": path,
        "text": output.stdout,
        "sha256": sha256_of(ssh, &expr)?,
        "bytes": bytes,
        "truncated": true,
    }))
}

/// The host's own hash of a file: `sha256sum` (GNU), or `shasum -a 256` (BSD and macOS).
fn sha256_of(ssh: &Ssh, expr: &str) -> Result<String, ErrorObject> {
    let output = ssh.run(
        &format!("sha256sum {expr} 2>/dev/null || shasum -a 256 {expr} 2>/dev/null"),
        TRANSFER,
    )?;
    let candidate = output
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();

    if candidate.len() == 64 && candidate.chars().all(|character| character.is_ascii_hexdigit()) {
        return Ok(candidate);
    }

    Err(ErrorObject::internal(format!(
        "{}: neither `sha256sum` nor `shasum` answered for that file, so SDC cannot say what it hashed. The file was not changed.",
        ssh.label()
    )))
}

/// `fs.write` on a host. The text travels on `ssh`'s **stdin** (`cat > <path>`), so nothing in it is
/// a word in a shell command - no quoting, no length limit from the command line, and a file full of
/// `$(...)` is written exactly as typed. Returns the same SHA-256 the local write returns.
pub fn write(ssh: &Ssh, path: &str, text: &str) -> Result<String, ErrorObject> {
    guard(path)?;

    let expr = remote_expr(path)?;
    let output = ssh.run_with_stdin(
        &format!("mkdir -p \"$(dirname {expr})\" && cat > {expr}"),
        text,
        TRANSFER,
    )?;

    if !output.ok() {
        return Err(failed(ssh, "saving the file", &output));
    }

    Ok(crate::fs::hash(text.as_bytes()))
}

/// `fs.rename` on a host: `mv`, refusing to overwrite what is already at the new name.
pub fn rename(ssh: &Ssh, from: &str, to: &str) -> Result<(), ErrorObject> {
    guard(from)?;
    guard(to)?;

    let (source, target) = (remote_expr(from)?, remote_expr(to)?);
    let output = ssh.run(
        &format!("[ -e {source} ] || exit 4; [ -e {target} ] && exit 5; mkdir -p \"$(dirname {target})\" && mv -- {source} {target}"),
        QUICK,
    )?;

    match output.code {
        Some(4) => Err(ErrorObject::not_found(format!("{}: `{from}` does not exist on that host", ssh.label()))),
        Some(5) => Err(ErrorObject::bad_request(format!("{}: `{to}` already exists; pick another name", ssh.label()))),
        _ if !output.ok() => Err(failed(ssh, "renaming", &output)),
        _ => Ok(()),
    }
}

/// `fs.delete` on a host: a file, or a folder with everything in it.
pub fn remove(ssh: &Ssh, path: &str) -> Result<(), ErrorObject> {
    guard(path)?;

    if path.trim_end_matches('/').is_empty() || path == "~" || path == "~/" {
        return Err(ErrorObject::bad_request("That is the root or a home directory; SDC does not delete it."));
    }

    let expr = remote_expr(path)?;
    let output = ssh.run(&format!("[ -e {expr} ] || exit 4; rm -rf -- {expr}"), TRANSFER)?;

    match output.code {
        Some(4) => Err(ErrorObject::not_found(format!("{}: `{path}` does not exist on that host", ssh.label()))),
        _ if !output.ok() => Err(failed(ssh, "deleting", &output)),
        _ => Ok(()),
    }
}

/// `fs.mkdir` on a host.
pub fn mkdir(ssh: &Ssh, path: &str) -> Result<(), ErrorObject> {
    guard(path)?;

    let expr = remote_expr(path)?;
    let output = ssh.run(&format!("mkdir -p -- {expr}"), QUICK)?;

    if !output.ok() {
        return Err(failed(ssh, "creating the folder", &output));
    }

    Ok(())
}

/// `fs.stat` on a host: `{path, size, dir, sha256}` - the same four fields the local stat answers,
/// with an empty hash for a folder (a folder has no contents to hash, and inventing one would make
/// two different folders look identical in a checkpoint's stamp).
pub fn stat(ssh: &Ssh, path: &str) -> Result<Value, ErrorObject> {
    guard(path)?;

    let expr = remote_expr(path)?;
    let output = ssh.run(
        &format!("if [ -d {expr} ]; then echo d 0; elif [ -f {expr} ]; then printf 'f '; wc -c < {expr}; else exit 4; fi"),
        QUICK,
    )?;

    if output.code == Some(4) {
        return Err(ErrorObject::not_found(format!("{}: `{path}` does not exist on that host", ssh.label())));
    }

    if !output.ok() {
        return Err(failed(ssh, "reading the file", &output));
    }

    let text = output.stdout.trim().to_string();
    let mut parts = text.split_whitespace();
    let dir = parts.next().unwrap_or("f") == "d";
    let size = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0).max(0);
    let sha256 = if dir { String::new() } else { sha256_of(ssh, &expr)? };

    Ok(json!({ "path": path, "size": size, "dir": dir, "sha256": sha256 }))
}

/// `fs.search` on a host, with `grep`, capped.
///
/// `grep -rnI` - recursive, with line numbers, and skipping binary files (the `I`), which is what a
/// naive `-r` gets wrong on a folder holding a `.png`. `--fixed-strings` because a query is a string
/// a person typed, not a regular expression they wrote; the local search is a literal `contains` too,
/// so the two agree. The glob is matched by name through `--include`, which keeps its meaning the
/// same as the local implementation's.
pub fn search(ssh: &Ssh, root: &str, query: &str, glob: Option<&str>, limit: usize) -> Result<Vec<Value>, ErrorObject> {
    guard(root)?;

    let expr = remote_expr(root)?;
    let include = match glob {
        Some(glob) => format!(" --include={}", sh_quote(glob)),
        None => String::new(),
    };
    let output = ssh.run(
        &format!(
            "grep -rnI --fixed-strings{include} -- {} {expr} 2>/dev/null | head -n {limit}",
            sh_quote(query)
        ),
        TRANSFER,
    )?;

    /* `grep` exits 1 when there is no match, which is an answer, not a failure. Its other exit codes
       are failures, and `head` closing the pipe reports 141 on some hosts: all three arrive here as a
       non-zero code with usable (or empty) stdout, so the parse decides. */
    if output.stdout.trim().is_empty() && output.code.unwrap_or(1) > 1 && output.code != Some(141) {
        /* No output *and* an error: only report it when `grep` is missing or the folder is not there. */
        let reason = output.reason().to_lowercase();

        if reason.contains("no such file") || reason.contains("not found") {
            return Err(failed(ssh, "searching", &output));
        }
    }

    Ok(parse_hits(&output.stdout, limit))
}

/// The hit parser: `path:line:text`, split from the **right** so a path containing a colon still
/// reads, and trimmed the same way the local search trims.
fn parse_hits(text: &str, limit: usize) -> Vec<Value> {
    let mut hits = Vec::new();

    for line in text.lines() {
        if hits.len() >= limit {
            break;
        }

        let Some(last) = line.rfind(':') else {
            continue;
        };
        let Some(second) = line[..last].rfind(':') else {
            continue;
        };

        let path = &line[..second];
        let number = &line[second + 1..last];

        if path.is_empty() {
            continue;
        }

        if let Ok(line_number) = number.parse::<i64>() {
            hits.push(json!({
                "path": path,
                "line": line_number,
                "text": line[last + 1..].trim(),
            }));
        }
    }

    hits
}

/* --------------------------------------------------------------------------------------------
 * Git
 * ------------------------------------------------------------------------------------------ */

/// `git.status` on a host: the branch of the **project's own** repository, and how many files its
/// working tree has changed - `("", 0)` for a folder that is not a repository, which is the same
/// honest answer the local implementation gives (a badge is not drawn for an empty branch).
///
/// `git -C <root> rev-parse --abbrev-ref HEAD` is asked first: it fails inside a non-repository, and
/// failing is the answer. `git -C <root> status --porcelain` then counts, so a project that is a
/// *subdirectory* of a repository still reports the repository it belongs to.
pub fn git_status(ssh: &Ssh, root: &str) -> Result<(String, i64), ErrorObject> {
    let expr = remote_expr(root)?;
    let branch = ssh.run(&format!("git -C {expr} rev-parse --abbrev-ref HEAD 2>/dev/null"), QUICK)?;

    if !branch.ok() {
        return Ok((String::new(), 0));
    }

    let output = ssh.run(&format!("git -C {expr} status --porcelain 2>/dev/null"), QUICK)?;

    if !output.ok() {
        return Err(failed(ssh, "reading git status", &output));
    }

    let dirty = output.stdout.lines().filter(|line| !line.trim().is_empty()).count() as i64;

    Ok((branch.stdout.trim().to_string(), dirty))
}

/// `git.diff` on a host: the working tree's patch, or the patch since a commit when one is named.
pub fn git_diff(ssh: &Ssh, root: &str, since: Option<&str>) -> Result<String, ErrorObject> {
    let expr = remote_expr(root)?;
    let script = match since {
        Some(sha) => format!("git -C {expr} diff {}", sh_quote(sha)),
        None => format!("git -C {expr} diff HEAD"),
    };
    let output = ssh.run(&script, TRANSFER)?;

    if !output.ok() {
        return Err(failed(ssh, "reading the diff", &output));
    }

    Ok(output.stdout)
}

/* --------------------------------------------------------------------------------------------
 * A turn on the host, and the shadow repository there
 * ------------------------------------------------------------------------------------------ */

/// Where the daemon keeps its own things on a host: `$HOME/.sdc` (0.7.13).
///
/// It is to the far side what the local data directory is to this machine - the pid a turn can be
/// killed by, and the shadow git repository a checkpoint commits to - and it is created lazily, so a
/// host nobody has worked on is not touched at all.
pub fn state_dir() -> &'static str {
    "\"$HOME\"/.sdc"
}

/// The pid file of one turn, as a path relative to the state directory.
///
/// The turn id is daemon-generated (`turn-12`), and this strips anything that is not a letter, a digit,
/// `-` or `_` anyway: a file named from an id that later grows a `/` or a quote would be a remote path a
/// shell parses, and a pid file is not worth that.
pub fn pid_file(turn_id: &str) -> String {
    let safe: String = turn_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-' || *character == '_')
        .collect();

    format!("run/{}.pid", if safe.is_empty() { "turn".to_string() } else { safe })
}

/// The line that runs one CLI turn on the host, so that `engine.cancel` can actually stop it.
///
/// Four parts, and each one earns its place:
///
/// * `cd <root> &&` - the turn has to work in the chat's folder **on that machine**, which is the whole
///   point of a remote chat;
/// * `sh -c 'echo $$ > <pid file>; exec env … <cli> …'` - the shell writes its own pid and then `exec`s
///   the CLI, so the pid in the file *is* the CLI's pid;
/// * `if command -v setsid …; then exec setsid sh -c …; else sh -c …; fi` - with `setsid` the CLI becomes
///   the leader of a **new process group**, so the kill line signals the group (`kill -TERM -<pid>`) and
///   not just one process. That is the difference between stopping a turn and leaving whatever it spawned
///   behind - a test runner, a compiler, a dev server - running on somebody's server. The **host** answers
///   whether `setsid` exists, in the same round trip: one less probe for the daemon to cache, one less
///   state to go stale, and a stripped container still gets a working turn;
/// * `env …` - the quiet flags the adapter sets locally (`NO_COLOR`, `TERM`) travel as environment for
///   the remote process rather than being set on the local `ssh`, where they would do nothing.
///
/// Every dynamic word goes through [`super::sh_quote`]: a program name, an argument and a folder are all
/// things a person's machine decided.
pub fn turn_line(
    program: &str,
    args: &[String],
    root: Option<&str>,
    env: &[(&str, &str)],
    pid_file: &str,
) -> Result<String, ErrorObject> {
    let mut text = "exec env".to_string();

    for (key, value) in env {
        text.push(' ');
        text.push_str(&format!("{key}={}", sh_quote(value)));
    }

    text.push(' ');
    text.push_str(&sh_quote(program));

    for arg in args {
        text.push(' ');
        text.push_str(&sh_quote(arg));
    }

    wrap_line(&text, root, pid_file)
}

/// The same wrapper around a **line** that is already a command - `pty.open`'s `line` parameter, which a
/// terminal sends (0.7.13).
///
/// The line is run by the *host's* own shell (`sh`), which is the point: a person typing into a terminal
/// about a VPS means that machine's shell, not the one the window happens to be running on. The pid file
/// and the `setsid` process group are the same as a turn's, so `pty.close` can stop the whole tree.
pub fn raw_line(line: &str, root: Option<&str>, pid_file: &str) -> Result<String, ErrorObject> {
    wrap_line(line, root, pid_file)
}

/// The pid file, the process group and the folder around a command - shared by a turn and by a line.
fn wrap_line(text: &str, root: Option<&str>, pid_file: &str) -> Result<String, ErrorObject> {
    let pid_path = format!("{}/{}", state_dir(), pid_file);
    /* The run directory is made first: `echo $$ > …` failing on a missing folder would still run the CLI
       (the `;` continues) but would leave *no pid file*, and a process nobody can stop is exactly the bug
       this line exists to prevent. `-p` and a silenced error keep it free when it already exists. */
    let inner = format!("mkdir -p {}/run 2>/dev/null; echo $$ > {pid_path}; {text}", state_dir());
    let quoted = sh_quote(&inner);
    let runner = format!(
        "if command -v setsid >/dev/null 2>&1; then exec setsid sh -c {quoted}; else sh -c {quoted}; fi"
    );

    match root {
        Some(root) => Ok(format!("cd {} && {runner}", remote_expr(root)?)),
        None => Ok(runner),
    }
}

/// The line that kills a turn's process on the host: the process **group** first, the pid second, and a
/// `SIGKILL` if the polite signal did not take.
///
/// Why all three: `kill -TERM <pid>` reaches one process, and a CLI that spawned a test runner leaves the
/// runner behind. `kill -TERM -<pid>` reaches the group `setsid` created (`turn_line`), which is what a
/// Stop button means. And a process that traps `SIGTERM` - or one whose group is not the pid's own
/// because `setsid` was missing on that host - needs the `-KILL` after the grace second.
///
/// Finding nothing is not an error: a turn that already finished has no pid file, and "there was nothing
/// to stop" is a fact rather than a failure.
pub fn kill_line(pid_file: &str) -> String {
    let path = format!("{}/{}", state_dir(), pid_file);

    format!(
        "if [ -r {path} ]; then pid=$(cat {path}); \
         {{ kill -TERM \"-$pid\" 2>/dev/null || kill -TERM \"$pid\" 2>/dev/null; }} || {{ rm -f {path}; echo SDC-NOTHING; exit 0; }}; \
         sleep 1; \
         {{ kill -KILL \"-$pid\" 2>/dev/null || kill -KILL \"$pid\" 2>/dev/null; }} || true; \
         rm -f {path}; echo SDC-KILLED; else echo SDC-NOTHING; fi"
    )
}

/// The line for a **long-running** process on the host (`pty.open` with a `hostId`).
///
/// It is [`turn_line`] without the CLI adapter's environment: the pid file is still written (so
/// `pty.close` can stop the group rather than the pipe) and the command is the one the caller typed, run
/// in the folder the caller named.
pub fn process_line(command: &str, args: &[String], cwd: Option<&str>, pid_file: &str) -> Result<String, ErrorObject> {
    turn_line(command, args, cwd, &[], pid_file)
}

/// The line for a long-running process the caller gave as a **whole line** rather than program + args -
/// a terminal's `Run in background` (0.7.13).
///
/// The **host's** shell runs it (`sh`), which is what a person typing about a VPS means - not the shell
/// of the machine the window happens to be on.
pub fn process_raw_line(line: &str, cwd: Option<&str>, pid_file: &str) -> Result<String, ErrorObject> {
    raw_line(line, cwd, pid_file)
}



/// The host's shadow repository for one project root, as a shell expression.
///
/// The name is the first sixteen hex characters of the SHA-256 of the **root string**, computed here:
/// like the local `git::shadow_path`, it is derived from the project rather than stored, so two daemons
/// (or a daemon and a person with `ssh`) agree on where the history of `/srv/app` lives without asking
/// anybody. Local: `<data>/git/…`; on a host: `$HOME/.sdc/git/…`.
pub fn shadow_dir(root: &str) -> String {
    let digest = crate::fs::hash(root.as_bytes());

    format!("{}/git/{}", state_dir(), &digest[..16])
}

/// One `git` invocation against the host's shadow repository, with the daemon's own identity (a commit
/// needs one, and it must not be the person's - the same rule the local `git` module follows).
fn shadow_git(root: &str, shadow: &str, args: &[&str]) -> Result<String, ErrorObject> {
    let quoted_root = remote_expr(root)?;
    let mut line = format!(
        "git -c user.name=sdcd -c user.email=sdcd@localhost --git-dir={shadow}/.git --work-tree={quoted_root}"
    );

    for arg in args {
        line.push(' ');
        line.push_str(&sh_quote(arg));
    }

    Ok(line)
}

/// `git::checkpoint` on a host: everything staged, one commit, and the commit's sha.
///
/// The initial empty commit of the local `ensure_repository` is made here too, and for the same reason:
/// a checkpoint taken *before* the first edit has to have something to be a point in time against.
pub fn shadow_checkpoint(ssh: &Ssh, root: &str, message: &str) -> Result<String, ErrorObject> {
    let shadow = shadow_dir(root);
    let init = shadow_git(root, &shadow, &["init", "--quiet"])?;
    let head = shadow_git(root, &shadow, &["rev-parse", "--verify", "HEAD"])?;
    let seed = shadow_git(root, &shadow, &["commit", "--allow-empty", "--quiet", "-m", "sdcd: shadow repository"])?;
    let add = shadow_git(root, &shadow, &["add", "-A"])?;
    let commit = shadow_git(root, &shadow, &["commit", "--allow-empty", "--quiet", "-m", message])?;
    let sha = shadow_git(root, &shadow, &["rev-parse", "HEAD"])?;
    let script = format!(
        "mkdir -p {shadow} && {{ {head} >/dev/null 2>&1 || {{ {init} >/dev/null && {seed} >/dev/null; }}; }} && {add} && {commit} && {sha}"
    );
    let output = ssh.run(&script, TRANSFER)?;

    if !output.ok() {
        return Err(failed(ssh, "taking a checkpoint", &output));
    }

    let sha = output.stdout.trim().to_string();

    if sha.len() != 40 || !sha.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err(ErrorObject::internal(format!(
            "{}: the shadow commit did not answer with a sha (`{sha}`), so nothing was recorded",
            ssh.label()
        )));
    }

    Ok(sha)
}

/// `git::diff` against the host's shadow repository - the fallback for a folder that is not a git
/// repository of its own, where the only history there is *is* the checkpoints.
pub fn shadow_diff(ssh: &Ssh, root: &str, since: Option<&str>) -> Result<String, ErrorObject> {
    let shadow = shadow_dir(root);
    let line = match since {
        Some(sha) => shadow_git(root, &shadow, &["diff", sha]),
        None => shadow_git(root, &shadow, &["diff", "HEAD"]),
    }?;
    let output = ssh.run(&line, TRANSFER)?;

    if !output.ok() {
        return Err(failed(ssh, "reading the checkpoint diff", &output));
    }

    Ok(output.stdout)
}

/// What changed in the folder since a shadow commit, **new files included** - what a review reads.
///
/// `git diff <sha>` alone compares tracked files only, so a file an agent created would be invisible to
/// the reviewer. `add -N` records the new files as intended-to-add in the *shadow's* index (the
/// project's own repository, if it has one, is not touched), which is what puts them in the diff.
pub fn shadow_changes(ssh: &Ssh, root: &str, since: &str) -> Result<String, ErrorObject> {
    let shadow = shadow_dir(root);
    let intend = shadow_git(root, &shadow, &["add", "-N", "--", ":/"])?;
    let diff = shadow_git(root, &shadow, &["diff", since])?;
    let output = ssh.run(&format!("{intend} && {diff}"), TRANSFER)?;

    if !output.ok() {
        return Err(failed(ssh, "reading what changed since the checkpoint", &output));
    }

    Ok(output.stdout)
}

/// Restores the host's working tree to a shadow commit - the file half of a rewind, on the far side.
pub fn shadow_restore(ssh: &Ssh, root: &str, sha: &str) -> Result<(), ErrorObject> {
    let shadow = shadow_dir(root);
    let line = shadow_git(root, &shadow, &["checkout", sha, "--", "."])?;
    let output = ssh.run(&line, TRANSFER)?;

    if !output.ok() {
        return Err(failed(ssh, "restoring the files", &output));
    }

    Ok(())
}

/* --------------------------------------------------------------------------------------------
 * One command, and one question
 * ------------------------------------------------------------------------------------------ */

/// `shell.run` on a host: one command, both streams captured, the same answer shape the local runner
/// gives (`pty::run_once`).
///
/// `exitCode` is `null` when `ssh` itself failed to establish the connection (255 is OpenSSH's own
/// code), because reporting 255 as the *command's* exit code would be a small lie about something a
/// caller is going to show a person.
pub fn shell(
    ssh: &Ssh,
    command: &str,
    args: &[String],
    cwd: Option<&str>,
    timeout: Duration,
) -> Result<Value, ErrorObject> {
    /* The same deny list the local runner uses (spec section 5.6), applied here as well: a `shutdown`
       or a `mkfs` refused on this laptop has no business reaching somebody's VPS, and the list is a
       speed bump with a reason attached either way. */
    if let Some(reason) = crate::pty::denied_reason(command, args) {
        return Err(ErrorObject::permission_denied(format!("{command}: {reason}")));
    }

    let mut line = command.to_string();

    for arg in args {
        line.push(' ');
        line.push_str(&sh_quote(arg));
    }

    let script = match cwd {
        Some(cwd) => format!("cd {} && {line}", remote_expr(cwd)?),
        None => line,
    };

    let started = std::time::Instant::now();
    let output = ssh.run(&script, timeout)?;
    let duration = started.elapsed().as_millis() as u64;
    let ok = output.ok();
    let unreachable = output.code == Some(255);

    Ok(json!({
        "command": command,
        "args": args,
        "cwd": cwd,
        "exitCode": if unreachable { Value::Null } else { json!(output.code) },
        "ok": ok,
        "stdout": output.stdout,
        "stderr": output.stderr,
        "durationMs": duration,
        "timedOut": output.timed_out,
        "truncated": output.truncated,
        "error": if ok { Value::Null } else { remote_error(command, &output) },
    }))
}

/// The plain-English failure for a remote command, in the same four keys the local runner uses.
fn remote_error(command: &str, output: &SshOutput) -> Value {
    if output.timed_out {
        let translation = crate::errors::translator::translate(command, "timed out");

        return json!({
            "title": translation.title,
            "explanation": "The command was still running when its timeout expired. The connection was closed; the command may still be running on the host.",
            "rule": translation.rule,
            "fixable": translation.fixable,
        });
    }

    let source = if output.stderr.trim().is_empty() { &output.stdout } else { &output.stderr };
    let translation = crate::errors::translator::translate(command, source);

    json!({
        "title": translation.title,
        "explanation": translation.explanation,
        "rule": translation.rule,
        "fixable": translation.fixable,
    })
}

/// Can this machine reach the host? Returns the status and the sentence that goes with it.
///
/// The command is `true`, i.e. "can I get a shell", which is exactly the question, and it runs with
/// every flag [`Ssh::base_args`] sets: the key SDC owns, against the pin SDC holds, with no prompt
/// and no other key offered.
pub fn probe(ssh: &Ssh) -> (String, String) {
    match ssh.run("true", Duration::from_secs(15)) {
        Ok(output) if output.ok() => ("connected".to_string(), format!("{} is reachable", ssh.label())),
        Ok(output) => (
            "offline".to_string(),
            format!("{}{}", refusal(&ssh.label(), &output.reason()), super::port_hint(&ssh.target)),
        ),
        Err(error) => (
            "offline".to_string(),
            format!(
                "{} could not be contacted: {}{}",
                ssh.label(),
                error.message,
                super::port_hint(&ssh.target)
            ),
        ),
    }
}

/// The sentence for an `ssh` that ran and refused - and the reason this function exists.
///
/// The refusal that used to be invisible is the host key. `StrictHostKeyChecking=accept-new` accepted
/// whatever answered first, so the one case that matters - a machine presenting a key that is not the
/// one SDC pinned - arrived as "did not answer", which is both wrong and unactionable. It now has its
/// own sentence, and the way out is named: re-pin the host (remove it and add it again) and read the
/// fingerprint the dialog shows.
///
/// The other common refusal is a host that wants a password or a one-time code: this call is batch
/// mode on purpose (there is no terminal behind it), so it cannot type either. That is what the
/// one-time key install is for, and the sentence says so.
pub fn refusal(label: &str, reason: &str) -> String {
    /* `reason` is normally one line (`SshOutput::reason`), and taking the first non-empty line here
       keeps that true whoever calls: a sentence with a whole stderr behind it is not a sentence. */
    let reason = reason
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no answer");
    let lowered = reason.to_lowercase();

    if lowered.contains("host key verification failed") {
        return format!(
            "{label} answered, but its host key is not the one SDC pinned for it - or SDC has no pin for it yet. Nothing was sent: no key was offered and no password was typed. Add the host again (Add a host → SSH / VPS) to see the fingerprint it presents now and pin it."
        );
    }

    if lowered.contains("keyboard-interactive") || lowered.contains("permission denied") {
        return format!(
            "{label} answered, but it does not accept SDC's key yet. Add this host again with its password filled in (Add a host → SSH / VPS) and SDC copies its key over once, after which every connection is passwordless - or add a key to `~/.ssh/authorized_keys` yourself."
        );
    }

    /* Everything else is reported in `ssh`'s own words: a timeout, a refused port, a bad key. */
    format!("{label} did not answer: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A listing as the remote loop prints it: the absolute directory, then `kind\tsize\tname`.
    ///
    /// The last line has no name - the shape a name containing a tab or a newline produces - because
    /// that is the case the parser has to **count** rather than swallow.
    const LISTING: &str = "/srv/app\n\
                           d\t0\tsrc\n\
                           f\t12\tREADME.md\n\
                           f\t9\t.env\n\
                           f\t4\n";

    #[test]
    fn a_listing_carries_absolute_paths_and_counts_what_it_hides() {
        let (directory, rows, hidden) = parse_listing(LISTING);
        let names: Vec<&str> = rows.iter().filter_map(|row| row["name"].as_str()).collect();

        assert_eq!(directory, "/srv/app");
        assert_eq!(names, vec!["README.md", "src"]);
        /* `.env` is the guard's, and the malformed `notes` line is counted rather than skipped in
           silence: a tree that is two rows short says so. */
        assert_eq!(hidden, 2, "the guarded name and the malformed line");

        assert_eq!(rows[1]["dir"], true);
        assert_eq!(rows[0]["size"], 12);
        assert_eq!(rows[0]["path"], "/srv/app/README.md", "the tree joins nothing itself");
    }

    #[test]
    fn a_hit_is_split_from_the_right_for_its_path_line_and_text() {
        let hits = parse_hits("/srv/app/src/a.ts:12:  const x = 1;\n/srv/we:ird/b.ts:3:let y = 2;\n", 10);

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["path"], "/srv/app/src/a.ts");
        assert_eq!(hits[0]["line"], 12);
        assert_eq!(hits[0]["text"], "const x = 1;", "trimmed, like the local search");
        assert_eq!(hits[1]["path"], "/srv/we:ird/b.ts", "a colon in the path does not shift the line");

        /* A line with no line number is not a hit, and the cap is honoured. */
        assert!(parse_hits("just a line\n", 10).is_empty());
        assert_eq!(parse_hits("/a:1:x\n/a:2:y\n/a:3:z\n", 2).len(), 2);
    }

    #[test]
    fn a_remote_path_must_be_absolute_or_from_the_home_and_is_quoted() {
        assert_eq!(remote_expr("/srv/app").unwrap(), "'/srv/app'");
        assert_eq!(remote_expr("~/app").unwrap(), "\"$HOME\"/'app'");

        /* Nothing a person types reaches the remote shell unquoted. */
        assert_eq!(remote_expr("/srv/$(rm -rf /)").unwrap(), "'/srv/$(rm -rf /)'");
        assert_eq!(remote_expr("/srv/a b").unwrap(), "'/srv/a b'");

        let refused = remote_expr("models").unwrap_err();

        assert_eq!(refused.code, "bad_request");
        assert!(refused.message.contains("not an absolute path"), "{}", refused.message);
    }

    /// The guard is checked **before** anything is sent, which is the property that matters: a refused
    /// path must not even open a connection.
    #[test]
    fn a_guarded_path_is_refused_before_a_connection_is_made() {
        let ssh = Ssh::parse("root@vps.example").unwrap();

        for path in ["/srv/app/.env", "/srv/app/.env.local", "/srv/app/cert.pem", "/srv/app/id_rsa"] {
            assert_eq!(read(&ssh, path, 1024).unwrap_err().code, "blocked_path", "{path}");
            assert_eq!(write(&ssh, path, "x").unwrap_err().code, "blocked_path", "{path}");
            assert_eq!(list(&ssh, path).unwrap_err().code, "blocked_path", "{path}");
        }

        assert!(guard("/srv/app/src/main.rs").is_ok());
        assert!(guard("/srv/app/README.md").is_ok());
    }

    /// The two refusal sentences that a person can act on, ported from the 0.7.0 tests and joined by
    /// the one this release adds.
    #[test]
    fn a_refusal_names_the_thing_that_can_be_done_about_it() {
        let wants_password = refusal("root@vps.example", "root@vps.example: Permission denied (keyboard-interactive,publickey).");

        assert!(wants_password.contains("does not accept SDC's key yet"), "{wants_password}");
        assert!(wants_password.contains("password"), "{wants_password}");

        let changed_key = refusal("root@vps.example", "Host key verification failed.");

        assert!(changed_key.contains("host key is not the one SDC pinned"), "{changed_key}");
        assert!(changed_key.contains("Nothing was sent"), "{changed_key}");
        assert!(changed_key.contains("Add the host again"), "{changed_key}");

        /* Everything else is `ssh`'s own words. */
        let timeout = refusal("root@vps.example", "connect to host vps.example port 22: Connection timed out");

        assert!(timeout.starts_with("root@vps.example did not answer: "), "{timeout}");
    }

    #[test]
    fn a_host_is_named_with_the_port_it_was_added_on() {
        assert_eq!(Ssh::parse("root@vps.example").unwrap().label(), "root@vps.example");
        assert_eq!(Ssh::parse("ssh -p 8443 root@vps.example").unwrap().label(), "root@vps.example:8443");
    }

    /// The turn line puts the CLI in **its own process group** when the host can, without a probe.
    ///
    /// The `setsid` branch is what makes Stop mean stop: `kill -TERM -<pid>` reaches everything the turn
    /// started, not just the CLI. The fallback is in the same round trip, so a host without `setsid` still
    /// gets a working turn - and the pid it writes is then the session's, which the kill line handles.
    #[test]
    fn a_turn_starts_its_own_process_group_where_the_host_can() {
        let args = vec!["--print".to_string(), "--include-partial-messages".to_string()];
        let line = turn_line("claude", &args, Some("/srv/app"), &[("NO_COLOR", "1")], "run/turn-9.pid").unwrap();

        assert!(line.contains("cd '/srv/app' &&"), "{line}");
        assert!(line.contains("if command -v setsid >/dev/null 2>&1; then exec setsid sh -c "), "{line}");
        assert!(line.contains("; else sh -c "), "the host without setsid still runs the turn: {line}");
        assert!(line.contains("mkdir -p "), "the pid file needs its folder to exist: {line}");
        assert!(line.contains("echo $$ >"), "the pid comes from the shell that execs the CLI: {line}");
        assert!(line.contains("run/turn-9.pid"), "{line}");
        assert!(line.contains("NO_COLOR='\\''1'\\''"), "the value is quoted for the remote shell: {line}");
        assert!(line.contains("'--print'"), "{line}");

        /* No folder means no `cd` - a chat that was never given one. */
        assert!(!turn_line("claude", &[], None, &[], "run/x.pid").unwrap().contains("cd "));

        /* A long-running process uses the same line, without the adapter's environment. */
        let process = process_line("npm", &["run".to_string(), "dev".to_string()], Some("~/app"), "run/pty-1.pid").unwrap();

        assert!(process.contains("cd \"$HOME\"/'app' &&"), "{process}");
        assert!(process.contains("'\\''npm'\\'' '\\''run'\\'' '\\''dev'\\''"), "the words are quoted inside the quoted shell: {process}");
    }

    /// A cancel signals the group, then the pid, then kills - and finds nothing as a *fact*.
    #[test]
    fn a_cancel_signals_the_group_before_the_pid_and_never_lies_about_nothing() {
        let line = kill_line("run/turn-9.pid");

        assert!(line.contains("kill -TERM \"-$pid\""), "the group first: {line}");
        assert!(line.contains("|| kill -TERM \"$pid\""), "then the one process: {line}");
        assert!(line.contains("sleep 1"), "a moment for a polite exit: {line}");
        assert!(line.contains("kill -KILL \"-$pid\""), "and then the whole group again, unpolitely: {line}");
        assert!(line.contains("echo SDC-KILLED"), "{line}");
        assert!(line.contains("echo SDC-NOTHING"), "{line}");
        assert!(line.contains("rm -f "), "the pid file does not outlive the turn: {line}");
        assert!(line.contains("if [ -r "), "a turn that already finished is not an error: {line}");
    }
}




