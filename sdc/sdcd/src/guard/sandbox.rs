//! **The Sandbox Runner (P0-14, switch `exec.sandbox`)**: code that SDC runs to *check* a change - a test suite, a smoke
//! run, a script the model wrote - runs where it cannot write outside the project, cannot reach the network unless the
//! project says so, and cannot run away with the CPU, the memory or the clock.
//!
//! The mechanism is the operating system's own, and it is not the same on the three systems, so [`guarantees`] says -
//! per system, and checked against what is actually installed - exactly what is enforced. A claim of isolation that the
//! platform cannot back is worse than none: it is how a person decides it is safe to open a stranger's repository.
//!
//! | system | filesystem writes | network | limits | mechanism |
//! |---|---|---|---|---|
//! | macOS | outside the project and the build tools' own caches (`TOOL_CACHES`): refused | refused (loopback optional) | CPU and file size by `ulimit`; wall time by the runner | `sandbox-exec` with a Seatbelt profile |
//! | Linux | outside the project: refused (read-only root) | refused (own network namespace) | as above, plus memory by `ulimit -v` | `bwrap` (bubblewrap) when installed, else `unshare -n` for the network alone |
//! | Windows | **not isolated** | **not isolated** | wall time, 256 processes, 4 GiB, kill-on-close, no clipboard/desktop | a Job Object (`winjob`) around every Command Runner child |
//!
//! Windows is still the honest gap for files and the network: isolating those needs an AppContainer or Windows Sandbox, which
//! cannot be built and proven without a Windows machine. What 3.1 adds is a Job Object (`winjob`): limits that the kernel enforces
//! (process count, memory, kill-on-close, UI) - reported as `limits: Enforced`, while [`guarantees`] still reports `Level::None` for
//! files and network, and a caller may refuse to run untrusted code there.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The platform refuses the operation.
    Enforced,
    /// Some of it is refused (named in `notes`).
    Partial,
    /// Nothing is isolated.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guarantees {
    pub os: &'static str,
    pub mechanism: &'static str,
    pub filesystem: Level,
    pub network: Level,
    pub limits: Level,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Network {
    Off,
    /// Loopback and these `ip:port` targets (the macOS profile cannot name hosts, only addresses).
    Allow(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// The only places a command may write: the project (or its shadow copy) and the temp folder.
    pub writable: Vec<PathBuf>,
    pub network: Network,
    /// CPU seconds (`ulimit -t`); 0 = no limit.
    pub cpu_seconds: u64,
    /// Largest file a command may write, in MiB (`ulimit -f`); 0 = no limit.
    pub file_mib: u64,
    /// Address space in MiB (`ulimit -v`); enforced on Linux only; 0 = no limit.
    pub memory_mib: u64,
}

impl Policy {
    /// The default for verification: the project and the temp folder are writable, the network is off, and a command
    /// gets ten CPU minutes and a 1 GiB file.
    pub fn for_project(root: &Path) -> Self {
        let mut writable = vec![root.to_path_buf(), std::env::temp_dir()];

        /* A build tool writes its own caches: `cargo` takes a lock in ~/.cargo, `npm` logs to ~/.npm. Refusing that would
           make every honest check fail, so these folders - and only the ones that exist - are writable too. They are the
           one place a hostile check could leave something behind for later, which `guarantees` says. */
        if let Some(home) = dirs::home_dir() {
            for cache in TOOL_CACHES {
                let path = home.join(cache);

                if path.is_dir() {
                    writable.push(path);
                }
            }
        }

        Self { writable, network: Network::Off, cpu_seconds: 600, file_mib: 1024, memory_mib: 0 }
    }
}

fn exists(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())) || Path::new("/usr/bin").join(program).is_file()
}

/// The build tools' own cache folders under the home directory (those that exist are writable in the sandbox).
pub const TOOL_CACHES: [&str; 12] = [".cargo", ".rustup", ".npm", ".pnpm-store", ".yarn", ".cache", ".gradle", ".m2", "go", "Library/Caches", ".local/share/pnpm", ".nuget"];

/// What is enforced on *this* machine, now.
pub fn guarantees() -> Guarantees {
    guarantees_for(std::env::consts::OS, exists("sandbox-exec") || Path::new("/usr/bin/sandbox-exec").is_file(), exists("bwrap"), exists("unshare"))
}

/// The same, for a named system and tool set - so every row of the table above is tested without that system.
pub fn guarantees_for(os: &str, has_sandbox_exec: bool, has_bwrap: bool, has_unshare: bool) -> Guarantees {
    match os {
        "macos" if has_sandbox_exec => Guarantees {
            os: "macos",
            mechanism: "sandbox-exec (Seatbelt)",
            filesystem: Level::Enforced,
            network: Level::Enforced,
            limits: Level::Partial,
            notes: vec!["writes outside the project and the temp folder are refused; reads are open except ~/.ssh, ~/.aws, ~/.gnupg and the Keychain".into(), "memory is not limited on macOS (the kernel ignores ulimit -v); CPU time and file size are".into()],
        },
        "macos" => Guarantees { os: "macos", mechanism: "none (sandbox-exec not found)", filesystem: Level::None, network: Level::None, limits: Level::Partial, notes: vec!["sandbox-exec is missing; only CPU time, file size and wall time are limited".into()] },
        "linux" if has_bwrap => Guarantees {
            os: "linux",
            mechanism: "bwrap (bubblewrap)",
            filesystem: Level::Enforced,
            network: Level::Enforced,
            limits: Level::Enforced,
            notes: vec!["the root is read-only; only the project and a private /tmp are writable; the network namespace is empty".into()],
        },
        "linux" if has_unshare => Guarantees {
            os: "linux",
            mechanism: "unshare -n",
            filesystem: Level::None,
            network: Level::Enforced,
            limits: Level::Enforced,
            notes: vec!["the network is cut, but writes outside the project are NOT refused: install bubblewrap (bwrap) for that".into()],
        },
        "linux" => Guarantees { os: "linux", mechanism: "none (bwrap and unshare not found)", filesystem: Level::None, network: Level::None, limits: Level::Enforced, notes: vec!["only limits apply: install bubblewrap".into()] },
        "windows" => Guarantees {
            os: "windows",
            mechanism: "Job Object (process cap, memory cap, kill-on-close, UI limits)",
            filesystem: Level::None,
            network: Level::None,
            limits: Level::Enforced,
            notes: vec![
                "no filesystem or network isolation on this system: do not run code you do not trust".into(),
                "a command run by the Command Runner is contained in a Job Object: at most 256 processes, 4 GiB of memory, no clipboard or desktop access, and its whole process tree ends with it. It can still write outside the project and use the network".into(),
                "for real isolation run checks inside WSL2 (install bubblewrap there); SDC does not do that for you yet".into(),
            ],
        },
        other => Guarantees {
            os: "other",
            mechanism: "runner deadline and process-tree kill only",
            filesystem: Level::None,
            network: Level::None,
            limits: Level::Partial,
            notes: vec!["no filesystem or network isolation on this system: do not run code you do not trust".into()],
        },
    }
}

/// A path as the kernel sees it: Seatbelt and bubblewrap match resolved paths, and `/tmp` is a link on macOS.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn scheme(path: &Path) -> String {
    format!("(subpath \"{}\")", resolved(path).to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""))
}

/// The Seatbelt profile: everything is allowed except writing anywhere but the named folders, the network, and reading
/// the places credentials live. Pure, so the profile is tested as text and the OS tests prove it as behaviour.
pub fn macos_profile(policy: &Policy, home: Option<&Path>) -> String {
    let mut profile = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");

    profile.push_str("(allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (regex #\"^/dev/(fd|tty)\")");

    for path in &policy.writable {
        profile.push(' ');
        profile.push_str(&scheme(path));
    }

    profile.push_str(")\n(deny network*)\n");

    if let Network::Allow(targets) = &policy.network {
        profile.push_str("(allow network* (local ip) (remote ip \"localhost:*\")");

        for target in targets {
            profile.push_str(&format!(" (remote ip \"{}\")", target.replace('"', "")));
        }

        profile.push_str(")\n");
    }

    if let Some(home) = home {
        for secret in [".ssh", ".aws", ".gnupg", ".config/gcloud", "Library/Keychains"] {
            profile.push_str(&format!("(deny file-read* {})\n", scheme(&home.join(secret))));
        }
    }

    profile
}

/// The `ulimit` prefix of a wrapped line: CPU time, file size and (where the kernel honours it) memory.
fn limits(policy: &Policy, linux: bool) -> String {
    let mut out = String::new();

    if policy.cpu_seconds > 0 {
        out.push_str(&format!("ulimit -t {}; ", policy.cpu_seconds));
    }

    if policy.file_mib > 0 {
        /* `ulimit -f` counts 512-byte blocks on most shells. */
        out.push_str(&format!("ulimit -f {}; ", policy.file_mib * 2048));
    }

    if linux && policy.memory_mib > 0 {
        out.push_str(&format!("ulimit -v {}; ", policy.memory_mib * 1024));
    }

    out
}

/// The arguments of `bwrap`: a read-only root, the writable folders bound back writable, a private `/tmp`, and an
/// empty network namespace unless the project allowed the network.
pub fn bwrap_args(policy: &Policy) -> Vec<String> {
    let mut args: Vec<String> = ["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp"].iter().map(|arg| arg.to_string()).collect();

    for path in policy.writable.iter().filter(|path| **path != std::env::temp_dir()) {
        let path = resolved(path).to_string_lossy().to_string();

        args.extend(["--bind".to_string(), path.clone(), path]);
    }

    if policy.network == Network::Off {
        args.push("--unshare-net".into());
    }

    args.push("--die-with-parent".into());

    args
}

/// A shell line that runs `line` inside the sandbox this machine has, or `Err` saying why it cannot (the caller
/// decides whether to run unsandboxed - and must say so).
pub fn wrap_line(line: &str, policy: &Policy) -> Result<String, String> {
    wrap_line_for(std::env::consts::OS, line, policy, &guarantees())
}

pub fn wrap_line_for(os: &str, line: &str, policy: &Policy, available: &Guarantees) -> Result<String, String> {
    let linux = os == "linux";
    let inner = format!("{}{}", limits(policy, linux), line);

    match (os, available.mechanism) {
        ("macos", "sandbox-exec (Seatbelt)") => {
            let profile = macos_profile(policy, dirs::home_dir().as_deref());

            Ok(format!("/usr/bin/sandbox-exec -p {} /bin/sh -c {}", quote(&profile), quote(&inner)))
        }
        ("linux", "bwrap (bubblewrap)") => Ok(format!("bwrap {} /bin/sh -c {}", bwrap_args(policy).iter().map(|arg| quote(arg)).collect::<Vec<_>>().join(" "), quote(&inner))),
        ("linux", "unshare -n") if policy.network == Network::Off => Ok(format!("unshare -rn /bin/sh -c {}", quote(&inner))),
        ("linux", "unshare -n") => Ok(format!("/bin/sh -c {}", quote(&inner))),
        _ => Err(format!("no sandbox on this system: {}", available.notes.first().cloned().unwrap_or_else(|| available.mechanism.to_string()))),
    }
}

#[cfg(test)]
mod tests;
