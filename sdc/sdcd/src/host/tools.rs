//! The tools SDC installs for itself (0.21) - so a fresh computer needs no terminal.
//!
//! WHY THIS EXISTS. Until 0.20 a missing program ended in a sentence: the doctor's `Install` button said
//! "install it in your own terminal", the Connect card printed the command to type, and Ollama asked for
//! `ollama serve` and `ollama pull`. The owner's rule for the app is that whatever SDC offers works on any
//! computer it is installed on, without anybody typing a command anywhere. So the daemon installs what
//! it needs itself, on every platform the same way:
//!
//! | tool | from | where |
//! | ---- | ---- | ----- |
//! | Node.js (LTS) | nodejs.org's official archive | `<tools>/node` |
//! | Claude Code, Codex, Gemini CLI | npm, with the Node above (or the machine's own) | `<tools>/npm-global` |
//! | Ollama | the official release archive | `<tools>/ollama` |
//! | ripgrep | the official release archive | `<tools>/bin` |
//! | Git | Windows: Git for Windows' PortableGit (git, bash and ssh); macOS: Apple's own Command Line Tools installer; Linux: the system package manager behind the desktop's password prompt | `<tools>/git` on Windows |
//!
//! `<tools>` is `%LOCALAPPDATA%\sdc-tools`, `~/Library/Application Support/sdc-tools` or
//! `~/.local/share/sdc-tools` - the user's own folder, so nothing needs an administrator, and outside the
//! app's install folder, so an update or an uninstall does not take the tools with it. The folders are on
//! the daemon's `PATH` from the start (`env_path::widen`), so a tool installed while SDC runs is found by
//! the next lookup, and every program the daemon starts finds it too (an npm CLI's `node`, for one).
//!
//! The only permission ever asked is the operating system's own: Apple's installer window for Git on a
//! Mac without it, and the desktop's password prompt for Git on Linux. Nothing is installed without the
//! person pressing Install, except the one tool SDC itself cannot work without - Git on Windows, which the
//! daemon fetches in the background on first start when the machine has none (checkpoints and the Time
//! Machine are Git).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

/// One tool SDC can install: its id (the doctor row's), the label, the program that proves it is there.
pub struct Tool {
    pub id: &'static str,
    pub label: &'static str,
    pub program: &'static str,
    /// Roughly what the download weighs, said before the button is pressed.
    pub size: &'static str,
}

pub const TOOLS: &[Tool] = &[
    Tool { id: "node", label: "Node.js", program: "node", size: "~30 MB" },
    Tool { id: "claude", label: "Claude Code CLI", program: "claude", size: "~60 MB" },
    Tool { id: "codex", label: "Codex CLI", program: "codex", size: "~50 MB" },
    Tool { id: "gemini", label: "Gemini CLI", program: "gemini", size: "~40 MB" },
    Tool { id: "ollama", label: "Ollama", program: "ollama", size: if cfg!(target_os = "macos") { "~30 MB" } else { "~1.5 GB" } },
    Tool { id: "ripgrep", label: "ripgrep", program: "rg", size: "~2 MB" },
    Tool { id: "git", label: "Git", program: "git", size: if cfg!(windows) { "~60 MB" } else { "~80 MB" } },
    /* 0.21.1: the agent's browser (screenshots, clicking through a page it built) on a machine with no
       Chrome, Edge or Chromium - many Linux desktops have only Firefox. */
    Tool { id: "browser", label: "Browser for the agent", program: "chrome-headless-shell", size: "~100 MB" },
];

/// The npm package behind each CLI.
fn npm_package(id: &str) -> Option<&'static str> {
    match id {
        "claude" => Some("@anthropic-ai/claude-code"),
        "codex" => Some("@openai/codex"),
        "gemini" => Some("@google/gemini-cli"),
        _ => None,
    }
}

pub fn tool(id: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.id == id)
}

/// `<tools>`: `SDC_TOOLS_DIR` when set (tests, portable installs), else the user's local data folder.
pub fn root() -> PathBuf {
    if let Some(custom) = std::env::var_os("SDC_TOOLS_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(custom);
    }

    let base = if cfg!(windows) { dirs::data_local_dir() } else { dirs::data_dir() };

    base.unwrap_or_else(std::env::temp_dir).join("sdc-tools")
}

/// The folders under `root` that hold programs, in the order they are searched. All of them go on `PATH`
/// whether they exist yet or not - a folder made by an install minutes later must be found without a restart.
pub fn path_dirs(root: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![
            root.join("node"),
            root.join("npm-global"),
            root.join("bin"),
            root.join("git").join("cmd"),
            root.join("git").join("bin"),
            root.join("ollama"),
        ]
    } else {
        vec![
            root.join("node").join("bin"),
            root.join("npm-global").join("bin"),
            root.join("bin"),
            root.join("ollama").join("bin"),
            root.join("ollama"),
            conda_git_bin(root),
        ]
    }
}

/// Whether the program SDC would run for `id` is one SDC installed.
pub fn managed(program: &Path) -> bool {
    program.starts_with(root())
}

/* ------------------------------------------------------------------------------------------------
 * Jobs: an install runs on its own thread; the window asks how it is going.
 * ---------------------------------------------------------------------------------------------- */

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub id: String,
    pub tool: String,
    /// `running`, `done` or `failed`.
    pub state: &'static str,
    /// What is happening now, in a few words.
    pub step: String,
    pub done: u64,
    pub total: Option<u64>,
    pub log: Vec<String>,
    pub error: Option<String>,
}

impl Job {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "tool": self.tool,
            "state": self.state,
            "step": self.step,
            "done": self.done,
            "total": self.total,
            "log": self.log,
            "error": self.error,
        })
    }
}

fn jobs() -> &'static Mutex<HashMap<String, Job>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Job>>> = OnceLock::new();

    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A handle the install code reports through.
#[derive(Clone)]
pub struct Progress {
    id: String,
}

impl Progress {
    fn update(&self, change: impl FnOnce(&mut Job)) {
        if let Some(job) = jobs().lock().unwrap_or_else(|poison| poison.into_inner()).get_mut(&self.id) {
            change(job);
        }
    }

    pub fn step(&self, step: &str) {
        let step = step.to_string();

        self.update(|job| {
            job.log.push(step.clone());
            job.step = step;
            job.done = 0;
            job.total = None;
        });
    }

    pub fn bytes(&self, done: u64, total: Option<u64>) {
        self.update(|job| {
            job.done = done;
            job.total = total;
        });
    }

    pub fn line(&self, line: &str) {
        let line = line.trim_end().to_string();

        if line.is_empty() {
            return;
        }

        self.update(|job| {
            job.log.push(line);

            /* The window shows the tail; the rest is not worth the memory. */
            if job.log.len() > 200 {
                job.log.drain(..job.log.len() - 200);
            }
        });
    }
}

/// Starts `work` as the job for `key`, unless one is already running for it - then that one is answered.
pub fn spawn_job(key: &str, label: &str, work: impl FnOnce(&Progress) -> Result<String, String> + Send + 'static) -> Job {
    let mut all = jobs().lock().unwrap_or_else(|poison| poison.into_inner());

    if let Some(running) = all.values().find(|job| job.tool == key && job.state == "running") {
        return running.clone();
    }

    let id = format!("job-{}", uuid::Uuid::new_v4().simple());
    let job = Job { id: id.clone(), tool: key.to_string(), state: "running", step: format!("Preparing {label}"), done: 0, total: None, log: Vec::new(), error: None };

    all.insert(id.clone(), job.clone());
    drop(all);

    let progress = Progress { id };

    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&progress)))
            .unwrap_or_else(|_| Err("the installer stopped unexpectedly".to_string()));

        progress.update(|job| match outcome {
            Ok(summary) => {
                job.state = "done";
                job.log.push(summary.clone());
                job.step = summary;
            }
            Err(reason) => {
                job.state = "failed";
                job.step = "Failed".to_string();
                job.error = Some(reason);
            }
        });
    });

    job
}

pub fn status(id: &str) -> Option<Job> {
    jobs().lock().unwrap_or_else(|poison| poison.into_inner()).get(id).cloned()
}

/// Installs one tool, as a job.
pub fn install(id: &str) -> Result<Job, String> {
    let tool = tool(id).ok_or_else(|| format!("SDC cannot install `{id}`"))?;

    Ok(spawn_job(tool.id, tool.label, move |progress| install_now(tool, progress)))
}

/// The rows the window lists: every tool, whether it is there, and whether SDC put it there.
pub fn list() -> Value {
    let rows: Vec<Value> = TOOLS
        .iter()
        .map(|tool| {
            /* The browser is not asked for `--version` (a headless shell may start instead of answering): it is
               there when the agent would find one. */
            let (found, version) = if tool.id == "browser" {
                let found = crate::agent::browser::find_browser();
                let version = found.as_ref().map(|path| path.display().to_string());

                (found, version)
            } else {
                let found = crate::host::program::resolve(tool.program);
                let version = found.as_ref().and_then(|_| crate::host::doctor::version_of(tool.program));

                (found, version)
            };

            json!({
                "id": tool.id,
                "label": tool.label,
                "installed": version.is_some(),
                "version": version,
                "managed": found.as_deref().is_some_and(managed),
                "size": tool.size,
                "running": jobs().lock().unwrap_or_else(|poison| poison.into_inner()).values().find(|job| job.tool == tool.id && job.state == "running").map(Job::to_json),
            })
        })
        .collect();

    json!({ "tools": rows, "root": root().display().to_string() })
}

fn install_now(tool: &Tool, progress: &Progress) -> Result<String, String> {
    let root = root();

    std::fs::create_dir_all(&root).map_err(|error| format!("cannot create {}: {error}", root.display()))?;

    match tool.id {
        "node" => install_node(&root, progress),
        "claude" | "codex" | "gemini" => install_cli(&root, tool, progress),
        "ollama" => install_ollama(&root, progress),
        "ripgrep" => install_ripgrep(&root, progress),
        "git" => install_git(&root, progress),
        "browser" => install_browser(&root, progress),
        other => Err(format!("SDC cannot install `{other}`")),
    }
}

/* ------------------------------------------------------------------------------------------------
 * The installers.
 * ---------------------------------------------------------------------------------------------- */

/// `x64` / `arm64` as nodejs.org spells them.
fn node_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" }
}

/// The file name of a Node release for this machine.
pub fn node_archive(version: &str) -> String {
    let arch = node_arch();

    if cfg!(windows) {
        format!("node-{version}-win-{arch}.zip")
    } else if cfg!(target_os = "macos") {
        format!("node-{version}-darwin-{arch}.tar.gz")
    } else {
        format!("node-{version}-linux-{arch}.tar.gz")
    }
}

fn install_node(root: &Path, progress: &Progress) -> Result<String, String> {
    /* A CLI's install brings Node when it is missing, so "Install Node.js" can be pressed while that runs:
       one at a time, and the second finds the first one's Node and stops there. */
    static NODE: Mutex<()> = Mutex::new(());

    let _one = NODE.lock().unwrap_or_else(|poison| poison.into_inner());

    if let Some(version) = crate::host::program::resolve("node").filter(|found| managed(found)).and_then(|_| crate::host::doctor::version_of("node")) {
        return Ok(format!("Node.js {version} installed"));
    }

    progress.step("Finding the newest Node.js LTS");

    let index: Value = serde_json::from_str(&get_text("https://nodejs.org/dist/index.json")?).map_err(|error| format!("nodejs.org's release list could not be read: {error}"))?;
    let version = index
        .as_array()
        .and_then(|releases| releases.iter().find(|release| release["lts"].as_str().is_some()))
        .and_then(|release| release["version"].as_str())
        .ok_or("nodejs.org lists no LTS release")?
        .to_string();
    let file = node_archive(&version);
    let archive = download(&format!("https://nodejs.org/dist/{version}/{file}"), &root.join("downloads").join(&file), progress, &format!("Downloading Node.js {version}"))?;

    progress.step("Unpacking Node.js");

    let staging = fresh_dir(&root.join(".staging-node"))?;

    extract(&archive, &staging)?;
    replace_dir(&single_child(&staging)?, &root.join("node"))?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);

    Ok(format!("Node.js {version} installed"))
}

fn install_cli(root: &Path, tool: &Tool, progress: &Progress) -> Result<String, String> {
    let package = npm_package(tool.id).ok_or("no npm package for this tool")?;

    /* The CLIs are Node programs: a machine without Node gets SDC's own first. */
    if crate::host::program::resolve("npm").is_none() {
        install_node(root, progress)?;
    }

    let npm = crate::host::program::resolve("npm").ok_or("npm is still missing after installing Node.js")?;
    let prefix = root.join("npm-global");

    std::fs::create_dir_all(&prefix).map_err(|error| error.to_string())?;
    progress.step(&format!("Installing {} with npm", tool.label));

    let mut command = crate::host::program::command_for(&npm);

    command
        .args(["install", "--global", "--no-fund", "--no-audit", "--loglevel=error", "--prefix"])
        .arg(&prefix)
        .arg(format!("{package}@latest"));

    run_logged(command, progress)?;

    let version = crate::host::doctor::version_of(tool.program).ok_or_else(|| format!("npm finished, but `{}` does not start", tool.program))?;

    Ok(format!("{} installed · {version}", tool.label))
}

/// The newest release's asset whose name passes `pick`, from GitHub: `(version, url, name)`.
fn github_asset(repo: &str, pick: impl Fn(&str) -> bool) -> Result<(String, String, String), String> {
    let release: Value = serde_json::from_str(&get_text(&format!("https://api.github.com/repos/{repo}/releases/latest"))?)
        .map_err(|error| format!("GitHub's answer about {repo} could not be read: {error}"))?;
    let tag = release["tag_name"].as_str().unwrap_or("latest").to_string();

    release["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|asset| asset["name"].as_str().is_some_and(&pick)))
        .and_then(|asset| Some((tag.clone(), asset["browser_download_url"].as_str()?.to_string(), asset["name"].as_str()?.to_string())))
        .ok_or_else(|| format!("{repo} {tag} has no download for this computer ({} {})", std::env::consts::OS, std::env::consts::ARCH))
}

/// The Ollama archive for this machine - the plain build, not the ROCm / MLX / Jetson variants.
pub fn ollama_asset(name: &str) -> bool {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" };

    if cfg!(windows) {
        name == format!("ollama-windows-{arch}.zip")
    } else if cfg!(target_os = "macos") {
        name == "ollama-darwin.tgz"
    } else {
        name == format!("ollama-linux-{arch}.tar.zst") || name == format!("ollama-linux-{arch}.tgz")
    }
}

fn install_ollama(root: &Path, progress: &Progress) -> Result<String, String> {
    progress.step("Finding the newest Ollama");

    let (tag, url, name) = github_asset("ollama/ollama", ollama_asset)?;
    let archive = download(&url, &root.join("downloads").join(&name), progress, &format!("Downloading Ollama {tag}"))?;

    progress.step("Unpacking Ollama");

    let target = root.join("ollama");
    let staging = fresh_dir(&root.join(".staging-ollama"))?;

    extract(&archive, &staging)?;
    replace_dir(&staging, &target)?;
    let _ = std::fs::remove_file(&archive);

    Ok(format!("Ollama {tag} installed"))
}

/// The ripgrep archive for this machine.
pub fn ripgrep_asset(name: &str) -> bool {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "aarch64-pc-windows-msvc.zip",
        ("windows", _) => "x86_64-pc-windows-msvc.zip",
        ("macos", "aarch64") => "aarch64-apple-darwin.tar.gz",
        ("macos", _) => "x86_64-apple-darwin.tar.gz",
        (_, "aarch64") => "aarch64-unknown-linux-gnu.tar.gz",
        _ => "x86_64-unknown-linux-musl.tar.gz",
    };

    name.starts_with("ripgrep-") && name.ends_with(triple)
}

fn install_ripgrep(root: &Path, progress: &Progress) -> Result<String, String> {
    progress.step("Finding the newest ripgrep");

    let (tag, url, name) = github_asset("BurntSushi/ripgrep", ripgrep_asset)?;
    let archive = download(&url, &root.join("downloads").join(&name), progress, &format!("Downloading ripgrep {tag}"))?;
    let staging = fresh_dir(&root.join(".staging-rg"))?;

    progress.step("Unpacking ripgrep");
    extract(&archive, &staging)?;

    let program = if cfg!(windows) { "rg.exe" } else { "rg" };
    let found = find_file(&staging, program).ok_or("the ripgrep archive has no rg program")?;
    let bin = root.join("bin");

    std::fs::create_dir_all(&bin).map_err(|error| error.to_string())?;
    std::fs::copy(&found, bin.join(program)).map_err(|error| error.to_string())?;
    make_executable(&bin.join(program));
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);

    Ok(format!("ripgrep {tag} installed"))
}

/// PortableGit for this Windows machine: git, bash (which Claude Code needs on Windows) and ssh.
pub fn portable_git_asset(name: &str) -> bool {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "64-bit" };

    name.starts_with("PortableGit-") && name.ends_with(&format!("-{arch}.7z.exe"))
}

fn install_git(root: &Path, progress: &Progress) -> Result<String, String> {
    if cfg!(windows) {
        progress.step("Finding the newest Git for Windows");

        let (tag, url, name) = github_asset("git-for-windows/git", portable_git_asset)?;
        let archive = download(&url, &root.join("downloads").join(&name), progress, &format!("Downloading Git {tag}"))?;
        let target = root.join("git");

        progress.step("Unpacking Git");

        /* PortableGit is a self-extracting 7-Zip archive: `-y -o<dir>` unpacks it silently, without
           administrator rights and without a window. */
        let staging = fresh_dir(&root.join(".staging-git"))?;
        let mut command = Command::new(&archive);

        command.arg("-y").arg(format!("-o{}", staging.display()));
        hide_window(&mut command);
        run_logged(command, progress)?;
        replace_dir(&staging, &target)?;
        let _ = std::fs::remove_file(&archive);
        point_claude_at_bash();

        return Ok(format!("Git {tag} installed"));
    }

    /* macOS and Linux (0.22): Git from conda-forge, through micromamba - one official program, no administrator,
       no window. Apple's Command Line Tools window and the Linux password prompt are gone: the same silent
       install as PortableGit on Windows. */
    install_git_conda(root, progress)
}

/// micromamba's own name for this machine.
pub fn conda_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "osx-arm64",
        ("macos", _) => "osx-64",
        (_, "aarch64") => "linux-aarch64",
        _ => "linux-64",
    }
}

/// The Git SDC installs on macOS and Linux: `<tools>/git-env/bin/git`.
pub fn conda_git_bin(root: &Path) -> PathBuf {
    root.join("git-env").join("bin")
}

fn install_git_conda(root: &Path, progress: &Progress) -> Result<String, String> {
    let platform = conda_platform();
    let micromamba = root.join("micromamba").join("micromamba");

    if !micromamba.is_file() {
        let url = format!("https://github.com/mamba-org/micromamba-releases/releases/latest/download/micromamba-{platform}");

        download(&url, &micromamba, progress, "Downloading micromamba (the installer for Git)")?;
        make_executable(&micromamba);
    }

    progress.step("Installing Git from conda-forge");

    let prefix = root.join("git-env");
    let mut command = Command::new(&micromamba);

    command
        .args(["create", "--yes", "--quiet", "--prefix"])
        .arg(&prefix)
        .args(["--channel", "conda-forge", "--override-channels", "git"])
        .env("MAMBA_ROOT_PREFIX", root.join("mamba"));
    run_logged(command, progress)?;

    let git = conda_git_bin(root).join("git");
    let output = Command::new(&git).arg("--version").output().map_err(|error| format!("Git was installed but does not start: {error}"))?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if !output.status.success() || version.is_empty() {
        return Err("Git was installed but does not answer --version".to_string());
    }

    Ok(format!("Git installed · {version}"))
}

/// Whether this Mac has Apple's Command Line Tools - without running `git`, whose stub would open Apple's installer.
pub fn mac_has_command_line_tools() -> bool {
    Command::new("/usr/bin/xcode-select").arg("-p").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
}

/// Whether this machine has a Git that works, asked without side effects: on a Mac without the Command Line
/// Tools `/usr/bin/git` is a stub that opens Apple's installer, so it is not counted (and not run).
pub fn has_working_git() -> bool {
    let root = root();

    if cfg!(windows) {
        return crate::host::program::resolve("git").is_some();
    }

    if conda_git_bin(&root).join("git").is_file() {
        return true;
    }

    match crate::host::program::resolve("git") {
        Some(path) if cfg!(target_os = "macos") && path == Path::new("/usr/bin/git") => mac_has_command_line_tools(),
        Some(_) => true,
        None => false,
    }
}

/* ------------------------------------------------------------------------------------------------
 * The same, on a server (0.21.1): Node and the coding CLIs into `~/.sdc/tools` on the host, over SDC's
 * own SSH connection, without `sudo` - the doctor's Install on a host row used to open a terminal there.
 * ---------------------------------------------------------------------------------------------- */

/// What every command SDC runs on a host puts first on its `PATH`: the tools SDC installed there. A
/// folder that does not exist costs nothing.
pub const REMOTE_PATH: &str = "PATH=\"$HOME/.sdc/tools/npm-global/bin:$HOME/.sdc/tools/node/bin:$PATH\"; export PATH; ";

/// The tools SDC can install on a host.
pub fn remote_installable(id: &str) -> bool {
    matches!(id, "node" | "claude" | "codex" | "gemini")
}

/// The POSIX `sh` script that installs `id` on a host, with Node `version` (`v24.21.0`) when the host has
/// no `npm`. Linux and macOS hosts, x64 and arm64; `curl` or `wget`; nothing that needs root.
pub fn remote_install_script(id: &str, version: &str) -> Option<String> {
    let package = if id == "node" { "" } else { npm_package(id)? };

    Some(format!(
        r#"set -e
T="$HOME/.sdc/tools"; mkdir -p "$T"
{path}
case "$(uname -m)" in x86_64|amd64) A=x64;; aarch64|arm64) A=arm64;; *) echo "SDC: this CPU ($(uname -m)) has no Node.js build"; exit 3;; esac
case "$(uname -s)" in Linux) O=linux;; Darwin) O=darwin;; *) echo "SDC: this system ($(uname -s)) has no Node.js build"; exit 3;; esac
fetch() {{ if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2"; elif command -v wget >/dev/null 2>&1; then wget -q "$1" -O "$2"; else echo "SDC: the host has neither curl nor wget"; exit 4; fi; }}
if ! command -v npm >/dev/null 2>&1 || [ "{id}" = node ]; then
  F="node-{version}-$O-$A.tar.gz"
  echo "Downloading Node.js {version}"
  fetch "https://nodejs.org/dist/{version}/$F" "$T/$F"
  rm -rf "$T/.node-tmp"; mkdir -p "$T/.node-tmp"; tar -xzf "$T/$F" -C "$T/.node-tmp"
  rm -rf "$T/node"; mv "$T/.node-tmp/node-{version}-$O-$A" "$T/node"; rm -rf "$T/.node-tmp" "$T/$F"
fi
if [ -n "{package}" ]; then
  echo "Installing {package}"
  npm install --global --no-fund --no-audit --loglevel=error --prefix "$T/npm-global" "{package}@latest"
fi
echo SDC-INSTALLED"#,
        path = REMOTE_PATH.trim(),
    ))
}

/// Installs `id` on the host `ssh` reaches, as a job.
pub fn install_remote(ssh: crate::ssh::Ssh, host_id: &str, id: &str) -> Result<Job, String> {
    let tool = tool(id).filter(|tool| remote_installable(tool.id)).ok_or_else(|| format!("SDC cannot install `{id}` on a server"))?;
    let key = format!("{host_id}:{}", tool.id);

    Ok(spawn_job(&key, tool.label, move |progress| {
        progress.step("Finding the newest Node.js LTS");

        let index: Value = serde_json::from_str(&get_text("https://nodejs.org/dist/index.json")?).map_err(|error| format!("nodejs.org's release list could not be read: {error}"))?;
        let version = index
            .as_array()
            .and_then(|releases| releases.iter().find(|release| release["lts"].as_str().is_some()))
            .and_then(|release| release["version"].as_str())
            .ok_or("nodejs.org lists no LTS release")?
            .to_string();
        let script = remote_install_script(tool.id, &version).ok_or("no script for this tool")?;

        progress.step(&format!("Installing {} on the server", tool.label));

        let output = ssh.run(&script, Duration::from_secs(15 * 60)).map_err(|error| error.message)?;

        for line in output.stdout.lines().chain(output.stderr.lines()) {
            progress.line(line);
        }

        if !output.stdout.contains("SDC-INSTALLED") {
            let tail: Vec<&str> = output.stdout.lines().chain(output.stderr.lines()).filter(|line| !line.trim().is_empty()).rev().take(3).collect();

            return Err(if tail.is_empty() { "the install on the server did not finish".to_string() } else { tail.into_iter().rev().collect::<Vec<_>>().join(" · ") });
        }

        Ok(format!("{} installed on the server", tool.label))
    }))
}

/// Where the browser SDC installs lives: `<tools>/browser/chrome-headless-shell[.exe]`.
pub fn managed_browser() -> PathBuf {
    root().join("browser").join(if cfg!(windows) { "chrome-headless-shell.exe" } else { "chrome-headless-shell" })
}

/// Chrome for Testing's name for this machine.
pub fn chrome_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86") => "win32",
        ("windows", _) => "win64",
        ("macos", "aarch64") => "mac-arm64",
        ("macos", _) => "mac-x64",
        (_, "aarch64") => "linux-arm64",
        _ => "linux64",
    }
}

/// Google's own headless Chromium for automation (Chrome for Testing), into `<tools>/browser`.
fn install_browser(root: &Path, progress: &Progress) -> Result<String, String> {
    progress.step("Finding the newest Chrome for Testing");

    let index: Value = serde_json::from_str(&get_text("https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json")?)
        .map_err(|error| format!("the Chrome for Testing list could not be read: {error}"))?;
    let stable = &index["channels"]["Stable"];
    let version = stable["version"].as_str().unwrap_or("stable").to_string();
    let platform = chrome_platform();
    let url = stable["downloads"]["chrome-headless-shell"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["platform"].as_str() == Some(platform)))
        .and_then(|row| row["url"].as_str())
        .ok_or_else(|| format!("Chrome for Testing has no build for {platform}"))?
        .to_string();
    let archive = download(&url, &root.join("downloads").join(format!("chrome-headless-shell-{platform}.zip")), progress, &format!("Downloading the browser ({version})"))?;
    let staging = fresh_dir(&root.join(".staging-browser"))?;

    progress.step("Unpacking the browser");
    extract(&archive, &staging)?;
    replace_dir(&single_child(&staging)?, &root.join("browser"))?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);
    make_executable(&managed_browser());

    if !managed_browser().is_file() {
        return Err("the browser archive did not hold chrome-headless-shell".to_string());
    }

    Ok(format!("Browser {version} installed"))
}


/// Claude Code on Windows runs its commands in Git Bash and looks for it at `CLAUDE_CODE_GIT_BASH_PATH`
/// when it is not where Git for Windows' installer puts it. With SDC's own PortableGit as the machine's only
/// Git, that variable is set for the daemon - and so for every `claude` it starts. Windows only; a variable
/// the person set is never replaced.
pub fn point_claude_at_bash() {
    if !cfg!(windows) || std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").is_some() {
        return;
    }

    let bash = root().join("git").join("bin").join("bash.exe");
    let installed = ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .any(|base| PathBuf::from(&base).join("Git").join("bin").join("bash.exe").is_file() || PathBuf::from(base).join("Programs").join("Git").join("bin").join("bash.exe").is_file());

    if bash.is_file() && !installed {
        /* Windows guards its environment block with a lock, so this is safe while threads run. */
        std::env::set_var("CLAUDE_CODE_GIT_BASH_PATH", bash);
    }
}

/// The first background job of a fresh machine: Git, which checkpoints need, when the machine has none - on
/// every platform. Everything else waits for the person to press Install.
pub fn provision_essentials() {
    point_claude_at_bash();

    /* 0.22: every platform - Windows (PortableGit), macOS and Linux (conda-forge Git) - silently. */
    if !has_working_git() {
        let _ = install("git");
    }
}

/* ------------------------------------------------------------------------------------------------
 * Ollama, run and fed by SDC.
 * ---------------------------------------------------------------------------------------------- */

/// Whether Ollama answers on its port.
pub fn ollama_running() -> bool {
    std::net::TcpStream::connect_timeout(&"127.0.0.1:11434".parse().expect("a socket address"), Duration::from_millis(400)).is_ok()
}

/// Starts `ollama serve` when Ollama is installed and not running, and waits until it answers. Answers
/// whether it is running now.
pub fn ensure_ollama() -> bool {
    if ollama_running() {
        return true;
    }

    let Some(program) = crate::host::program::resolve("ollama") else {
        return false;
    };
    let mut command = crate::host::program::command_for(&program);

    command.arg("serve").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    hide_window(&mut command);

    if command.spawn().is_err() {
        return false;
    }

    for _ in 0..40 {
        if ollama_running() {
            return true;
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    false
}

/// Downloads a model into Ollama (`/api/pull`), as a job with byte progress - what `ollama pull` did.
pub fn pull_model(model: &str) -> Job {
    let model = model.trim().to_string();
    let key = format!("ollama-pull:{model}");

    spawn_job(&key, &model.clone(), move |progress| {
        if !ensure_ollama() {
            return Err("Ollama is not installed, or it would not start - install it from Settings → Environment".to_string());
        }

        progress.step(&format!("Downloading {model}"));

        let response = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(600))
            .build()
            .post("http://127.0.0.1:11434/api/pull")
            .send_string(&json!({ "model": model, "stream": true }).to_string())
            .map_err(|error| format!("Ollama refused the download: {error}"))?;
        let reader = std::io::BufReader::new(response.into_reader());

        for line in std::io::BufRead::lines(reader) {
            let line = line.map_err(|error| error.to_string())?;
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };

            if let Some(error) = event["error"].as_str() {
                return Err(error.to_string());
            }

            if let (Some(done), Some(total)) = (event["completed"].as_u64(), event["total"].as_u64()) {
                progress.bytes(done, Some(total));
            } else if let Some(status) = event["status"].as_str() {
                progress.line(status);
            }
        }

        Ok(format!("{model} is ready"))
    })
}

/* ------------------------------------------------------------------------------------------------
 * A busy port, freed from the window (the doctor's `Kill process`).
 * ---------------------------------------------------------------------------------------------- */

/// The processes listening on `port` on this machine - never this daemon itself.
pub fn listeners(port: u16) -> Vec<u32> {
    let own = std::process::id();
    let mut pids: Vec<u32> = Vec::new();

    if cfg!(windows) {
        let mut command = Command::new("netstat");

        command.args(["-ano", "-p", "tcp"]);
        hide_window(&mut command);

        if let Ok(output) = command.output() {
            pids = listening_pids_netstat(&String::from_utf8_lossy(&output.stdout), port);
        }
    } else {
        let output = Command::new("lsof").args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"]).output();

        if let Ok(output) = output {
            pids = String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| line.trim().parse().ok()).collect();
        }
    }

    pids.retain(|pid| *pid != own && *pid != 0);
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// The PIDs `netstat -ano` shows listening on `port` (any local address).
pub fn listening_pids_netstat(text: &str, port: u16) -> Vec<u32> {
    let suffix = format!(":{port}");

    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();

            match fields.as_slice() {
                [proto, local, _, state, pid] if proto.eq_ignore_ascii_case("TCP") && local.ends_with(&suffix) && state.eq_ignore_ascii_case("LISTENING") => pid.parse().ok(),
                _ => None,
            }
        })
        .collect()
}

/// Stops whatever listens on `port`. Answers how many processes were stopped.
pub fn free_port(port: u16) -> Result<usize, String> {
    let pids = listeners(port);

    if pids.is_empty() {
        /* Linux without lsof: `fuser` knows the port too. */
        if !cfg!(windows) && crate::host::program::resolve("fuser").is_some() {
            let status = Command::new("fuser").args(["-k", &format!("{port}/tcp")]).stdout(Stdio::null()).stderr(Stdio::null()).status();

            return Ok(usize::from(status.is_ok_and(|status| status.success())));
        }

        return Ok(0);
    }

    for pid in &pids {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("taskkill");

            command.args(["/PID", &pid.to_string(), "/T", "/F"]);
            command
        } else {
            let mut command = Command::new("kill");

            command.arg(pid.to_string());
            command
        };

        hide_window(&mut command);

        let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
    }

    Ok(pids.len())
}

/* ------------------------------------------------------------------------------------------------
 * Plumbing.
 * ---------------------------------------------------------------------------------------------- */

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(120))
        .user_agent(&format!("sdcd/{}", crate::VERSION))
        .build()
}

fn get_text(url: &str) -> Result<String, String> {
    let response = agent().get(url).set("accept", "application/json").call().map_err(|error| format!("{url}: {error}"))?;
    let mut body = String::new();

    response.into_reader().take(20_000_000).read_to_string(&mut body).map_err(|error| error.to_string())?;

    Ok(body)
}

/// Downloads `url` to `dest` (through a `.part` file, so a cut download is never mistaken for a whole one).
fn download(url: &str, dest: &Path, progress: &Progress, label: &str) -> Result<PathBuf, String> {
    progress.step(label);

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let response = agent().get(url).call().map_err(|error| format!("{url}: {error}"))?;
    let total = response.header("content-length").and_then(|value| value.parse::<u64>().ok());
    let partial = dest.with_extension("part");
    let mut file = std::fs::File::create(&partial).map_err(|error| format!("cannot write {}: {error}", partial.display()))?;
    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut reported = std::time::Instant::now();

    loop {
        let read = reader.read(&mut buffer).map_err(|error| format!("the download was cut: {error}"))?;

        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read]).map_err(|error| format!("cannot write the download: {error}"))?;
        done += read as u64;

        if reported.elapsed() > Duration::from_millis(200) {
            progress.bytes(done, total);
            reported = std::time::Instant::now();
        }
    }

    file.flush().map_err(|error| error.to_string())?;
    drop(file);
    progress.bytes(done, total);

    if total.is_some_and(|total| total != done) {
        return Err(format!("the download stopped at {done} of {} bytes", total.unwrap_or(0)));
    }

    std::fs::rename(&partial, dest).map_err(|error| error.to_string())?;

    Ok(dest.to_path_buf())
}

/// Unpacks a `.zip`, `.tar.gz`/`.tgz` or `.tar.zst` into `into`.
pub fn extract(archive: &Path, into: &Path) -> Result<(), String> {
    let name = archive.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_ascii_lowercase();
    let file = std::fs::File::open(archive).map_err(|error| format!("cannot open {}: {error}", archive.display()))?;
    let failed = |error: std::io::Error| format!("cannot unpack {name}: {error}");

    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file).map_err(|error| format!("cannot unpack {name}: {error}"))?;

        return zip.extract(into).map_err(|error| format!("cannot unpack {name}: {error}"));
    }

    let reader: Box<dyn Read> = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Box::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file)))
    } else if name.ends_with(".tar.zst") {
        Box::new(ruzstd::decoding::StreamingDecoder::new(std::io::BufReader::new(file)).map_err(|error| format!("cannot unpack {name}: {error}"))?)
    } else {
        return Err(format!("SDC cannot unpack {name}"));
    };
    let mut tar = tar::Archive::new(reader);

    tar.set_preserve_permissions(true);
    tar.unpack(into).map_err(failed)
}

fn fresh_dir(dir: &Path) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(dir);

    std::fs::create_dir_all(dir).map_err(|error| format!("cannot create {}: {error}", dir.display()))?;

    Ok(dir.to_path_buf())
}

/// The one folder an archive unpacked into (`node-v24…/`), or the folder itself when it holds more.
fn single_child(dir: &Path) -> Result<PathBuf, String> {
    let entries: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|error| error.to_string())?.filter_map(Result::ok).map(|entry| entry.path()).collect();

    match entries.as_slice() {
        [only] if only.is_dir() => Ok(only.clone()),
        _ => Ok(dir.to_path_buf()),
    }
}

/// Puts `from` where `to` is, replacing an older copy.
fn replace_dir(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        std::fs::remove_dir_all(to).map_err(|error| format!("the old copy in {} is in use - close what runs from it and try again ({error})", to.display()))?;
    }

    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }

    copy_dir(from, to)?;
    let _ = std::fs::remove_dir_all(from);

    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|error| error.to_string())?;

    for entry in std::fs::read_dir(from).map_err(|error| error.to_string())?.filter_map(Result::ok) {
        let target = to.join(entry.file_name());

        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.filter_map(Result::ok) {
        let path = entry.path();

        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|file| file == name) {
            return Some(path);
        }
    }

    None
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    /* CREATE_NO_WINDOW: an installer step must not flash a console in front of the person. */
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

/// Runs an installer step, its output going to the job's log, and fails with its last lines.
fn run_logged(mut command: Command, progress: &Progress) -> Result<(), String> {
    hide_window(&mut command);

    let output = command.stdin(Stdio::null()).output().map_err(|error| format!("could not start the installer: {error}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));

    for line in text.lines() {
        progress.line(line);
    }

    if output.status.success() {
        return Ok(());
    }

    let tail: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).rev().take(4).collect();

    Err(if tail.is_empty() {
        format!("the installer ended with {}", output.status)
    } else {
        tail.into_iter().rev().collect::<Vec<_>>().join(" · ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_finds_its_own_download() {
        let names = [
            "ollama-darwin.tgz",
            "Ollama-darwin.zip",
            "ollama-linux-amd64.tar.zst",
            "ollama-linux-amd64-rocm.tar.zst",
            "ollama-windows-amd64.zip",
            "ollama-windows-amd64-rocm.zip",
        ];
        let picked: Vec<&str> = names.iter().copied().filter(|name| ollama_asset(name)).collect();

        assert_eq!(picked.len(), 1, "exactly one Ollama build for this machine: {picked:?}");
        assert!(!picked[0].contains("rocm"));

        let rg = ["ripgrep-15.2.0-x86_64-pc-windows-msvc.zip", "ripgrep-15.2.0-x86_64-pc-windows-gnu.zip", "ripgrep-15.2.0-aarch64-apple-darwin.tar.gz", "ripgrep-15.2.0-x86_64-apple-darwin.tar.gz", "ripgrep-15.2.0-x86_64-unknown-linux-musl.tar.gz", "ripgrep-15.2.0-aarch64-unknown-linux-gnu.tar.gz"];

        assert_eq!(rg.iter().filter(|name| ripgrep_asset(name)).count(), 1);
        assert!(portable_git_asset("PortableGit-2.56.0.2-64-bit.7z.exe") || cfg!(target_arch = "aarch64"));
        assert!(!portable_git_asset("MinGit-2.56.0.2-64-bit.zip"));
        assert!(node_archive("v24.21.0").starts_with("node-v24.21.0-"));
    }

    #[test]
    fn the_tool_folders_are_under_the_root_and_always_listed() {
        let root = PathBuf::from("/tmp/sdc-tools-test");
        let dirs = path_dirs(&root);

        assert!(dirs.iter().all(|dir| dir.starts_with(&root)));
        assert!(dirs.iter().any(|dir| dir.ends_with("bin") || dir.ends_with("node")));
        assert!(managed(&root.join("node").join("node")) == root.starts_with(super::root()));
    }

    #[test]
    fn archives_of_every_kind_unpack() {
        let dir = std::env::temp_dir().join(format!("sdc-extract-{}", uuid::Uuid::new_v4().simple()));
        let into = dir.join("out");

        std::fs::create_dir_all(&dir).unwrap();

        /* A .tar.gz with one file in a folder. */
        let tgz = dir.join("a.tar.gz");
        {
            let encoder = flate2::write::GzEncoder::new(std::fs::File::create(&tgz).unwrap(), flate2::Compression::fast());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();

            header.set_size(5);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, "pkg/hello", &b"hello"[..]).unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }

        extract(&tgz, &into).unwrap();
        assert_eq!(std::fs::read_to_string(into.join("pkg").join("hello")).unwrap(), "hello");
        assert_eq!(single_child(&into).unwrap(), into.join("pkg"));

        /* A .zip. */
        let zip_path = dir.join("b.zip");
        {
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());

            writer.start_file("rg.exe", zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(b"binary").unwrap();
            writer.finish().unwrap();
        }

        let zipped = dir.join("zipped");

        extract(&zip_path, &zipped).unwrap();
        assert_eq!(find_file(&zipped, "rg.exe"), Some(zipped.join("rg.exe")));

        /* A .tar.zst - the only shape Ollama ships for Linux: bin/ollama and a library beside it. */
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);

            for (path, body) in [("bin/ollama", &b"#!/bin/sh\necho ollama\n"[..]), ("lib/ollama/libx.so", &b"lib"[..])] {
                let mut header = tar::Header::new_gnu();

                header.set_size(body.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, path, body).unwrap();
            }

            builder.finish().unwrap();
        }

        let zst = dir.join("ollama-linux-amd64.tar.zst");

        std::fs::write(&zst, ruzstd::encoding::compress_to_vec(&tar_bytes[..], ruzstd::encoding::CompressionLevel::Fastest)).unwrap();

        let unpacked = dir.join("zst");

        extract(&zst, &unpacked).unwrap();
        assert!(std::fs::read_to_string(unpacked.join("bin").join("ollama")).unwrap().contains("echo ollama"));
        assert!(unpacked.join("lib").join("ollama").join("libx.so").is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(std::fs::metadata(unpacked.join("bin").join("ollama")).unwrap().permissions().mode() & 0o111, 0o111, "the program stays runnable");
        }

        assert!(extract(&dir.join("c.rar"), &into).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real thing, over the network: `cargo test --lib live_install -- --ignored --nocapture`. Installs
    /// ripgrep, Node and the Codex CLI into a throwaway folder and runs each one.
    #[test]
    #[ignore]
    fn live_install() {
        let dir = std::env::temp_dir().join(format!("sdc-tools-live-{}", uuid::Uuid::new_v4().simple()));

        std::env::set_var("SDC_TOOLS_DIR", &dir);

        let mut path: Vec<PathBuf> = path_dirs(&dir);

        path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).filter(|entry| {
            /* The machine's own Node must not be what makes the test pass. */
            !crate::host::program::candidates("node").iter().any(|name| entry.join(name).is_file())
        }));
        std::env::set_var("PATH", std::env::join_paths(path).unwrap());

        let wanted = std::env::var("SDC_LIVE_TOOLS").unwrap_or_else(|_| "ripgrep,node,codex".to_string());

        for id in wanted.split(',') {
            let job = install(id).unwrap();

            loop {
                let now = status(&job.id).unwrap();

                if now.state != "running" {
                    println!("{id}: {} {:?}", now.step, now.error);
                    assert_eq!(now.state, "done", "{id}: {:?} {:?}", now.error, now.log);
                    break;
                }

                std::thread::sleep(Duration::from_millis(500));
            }
        }

        for id in wanted.split(',') {
            if id == "browser" {
                assert!(managed_browser().is_file(), "the browser SDC installed is not there");

                /* And it does what the agent needs: a page opened, read and photographed over DevTools. */
                std::env::set_var("SDC_BROWSER", managed_browser());

                let page = dir.join("page.html");

                std::fs::write(&page, "<h1>SDC browser check</h1>").unwrap();

                let mut browser = crate::agent::browser::Browser::launch(800, 600).expect("the installed browser starts");

                browser.open(&format!("file:///{}", page.display().to_string().replace('\\', "/"))).expect("a page opens");
                assert!(browser.read().expect("the page reads").contains("SDC browser check"));
                assert!(browser.screenshot().expect("a screenshot").len() > 1000);
                drop(browser);
                continue;
            }

            let program = tool(id).unwrap().program;

            assert!(crate::host::program::resolve(program).is_some_and(|found| found.starts_with(&dir)), "{program} is not the one SDC installed");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_server_install_script_is_valid_sh_and_installs_into_the_users_home() {
        assert!(remote_install_script("git", "v24.21.0").is_none(), "git on a server needs root - not offered");

        for id in ["node", "claude", "codex", "gemini"] {
            let script = remote_install_script(id, "v24.21.0").unwrap();

            assert!(script.contains("$HOME/.sdc/tools"));
            assert!(!script.contains("sudo"));
            assert!(script.contains("node-v24.21.0-$O-$A.tar.gz"));

            if id != "node" {
                assert!(script.contains(&format!("{}@latest", npm_package(id).unwrap())));
            }

            /* A real `sh` parses it, where one is on this machine (Git Bash on Windows, every Unix). */
            if let Some(sh) = crate::host::program::resolve("sh") {
                let output = Command::new(sh).arg("-n").arg("-c").arg(&script).output().unwrap();

                assert!(output.status.success(), "{id}: {}", String::from_utf8_lossy(&output.stderr));
            }
        }

        assert!(REMOTE_PATH.starts_with("PATH=\"$HOME/.sdc/tools/npm-global/bin:$HOME/.sdc/tools/node/bin:$PATH\""));
    }

    #[test]
    fn netstat_lines_name_only_the_listeners_on_that_port() {
        let text = "\
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       4120
  TCP    127.0.0.1:30001        0.0.0.0:0              LISTENING       77
  TCP    127.0.0.1:3000         127.0.0.1:51000        ESTABLISHED     4120
  TCP    [::]:3000              [::]:0                 LISTENING       4121
";

        assert_eq!(listening_pids_netstat(text, 3000), vec![4120, 4121]);
        assert!(listening_pids_netstat(text, 80).is_empty());
    }

    #[test]
    fn a_second_install_of_the_same_tool_joins_the_running_one() {
        let first = spawn_job("test-tool", "Test", |_| {
            std::thread::sleep(Duration::from_millis(300));
            Ok("done".to_string())
        });
        let second = spawn_job("test-tool", "Test", |_| Ok("never".to_string()));

        assert_eq!(first.id, second.id);

        std::thread::sleep(Duration::from_millis(600));

        let finished = status(&first.id).unwrap();

        assert_eq!(finished.state, "done");
        assert_eq!(finished.step, "done");
    }
}
