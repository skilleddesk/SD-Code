//! R4: the VPS a project's turns run on.
//!
//! A person names their hosts once, in `.sdc/runners.toml`, and picks one with `.sdc/runner` (a single name)
//! or with the saved `runner.default`. Every turn in that project then reaches its workspace over `ssh`
//! instead of this disk, and the daemon can ask the host to answer one command before a turn is handed to it,
//! so a dead VPS is a clear message rather than a turn whose every tool quietly fails.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

pub const SETTING: &str = "runner.default";

/// One named host a project's turns can be handed to.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Runner {
    /// The name the person writes in `.sdc/runner`.
    pub name: String,
    /// `user@host` - exactly what `ssh` takes.
    pub host: String,
    /// Where the project lives on that host.
    pub dir: String,
}

#[derive(Debug, Default, Deserialize)]
struct All {
    #[serde(default, rename = "runner")]
    runners: Vec<Runner>,
}

fn slot() -> &'static std::sync::Mutex<Option<String>> {
    static SLOT: std::sync::OnceLock<std::sync::Mutex<Option<String>>> = std::sync::OnceLock::new();

    SLOT.get_or_init(|| std::sync::Mutex::new(None))
}

/// Remember the person's default runner (the `runner.default` setting), if they named one.
pub fn configure(store: &crate::store::sqlite::Store) {
    let default = store.setting(SETTING).ok().flatten().map(|text| text.trim().to_string()).filter(|name| !name.is_empty());

    if let Ok(mut slot) = slot().lock() {
        *slot = default;
    }
}

/// The runner in force (tests set their own).
pub fn default_name() -> Option<String> {
    #[cfg(test)]
    if let Some(default) = TEST_DEFAULT.with(|default| default.borrow().clone()) {
        return Some(default);
    }

    slot().lock().ok().and_then(|slot| slot.clone())
}

#[cfg(test)]
thread_local! {
    /// A test's own default runner, so one test's project does not route another test's turn.
    static TEST_DEFAULT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub fn set_default_for_test(default: Option<&str>) {
    TEST_DEFAULT.with(|slot| *slot.borrow_mut() = default.map(str::to_string));
}

/// The file that names the hosts (`[[runner]]` entries).
pub fn file(root: &Path) -> PathBuf {
    root.join(".sdc/runners.toml")
}

/// The file that picks one of them (its whole content is a name).
pub fn chosen_name(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(".sdc/runner")).ok()?;
    let name = text.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#'))?.to_string();

    (!name.is_empty()).then_some(name)
}

/// Every host the project names, in the order the file lists them.
pub fn load(root: &Path) -> Result<Vec<Runner>, String> {
    let path = file(root);

    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let all: All = toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;

    Ok(all.runners)
}

/// The host this project's turns go to: `.sdc/runner` wins over the saved default.
pub fn chosen(root: &Path, default: Option<&str>) -> Result<Option<Runner>, String> {
    let Some(name) = chosen_name(root).or_else(|| default.map(|name| name.trim().to_string())) else {
        return Ok(None);
    };

    load(root)?.into_iter().find(|runner| runner.name == name).map(Some).ok_or_else(|| format!("`.sdc/runner` names `{name}`, but `.sdc/runners.toml` has no host by that name"))
}

/// A named host as the ssh target a turn's tools run behind.
pub fn ssh(runner: &Runner) -> Result<crate::ssh::Ssh, String> {
    crate::ssh::Ssh::parse(&runner.host).map_err(|error| format!("`{}`: {}", runner.host, error.message))
}

/// The ssh target this project's turns run behind, when the person named a host.
pub fn remote_for(root: &Path, default: Option<&str>) -> Result<Option<crate::ssh::Ssh>, String> {
    chosen(root, default)?.map(|runner| ssh(&runner)).transpose()
}

/// One command on the host (`uname -s` in the project's directory): the check that it answers at all.
pub fn probe(runner: &Runner) -> Result<String, String> {
    let ssh = ssh(runner)?;
    let script = format!("cd {} && uname -s", quote(&runner.dir));

    ssh.run(&script, Duration::from_secs(20)).map(|output| output.stdout.trim().to_string()).map_err(|error| format!("{}: {}", runner.host, error.message))
}

/// Whether the machine is there **now**, before a turn starts against it (R4).
///
/// `Ok(None)` means it is up and this key fits. `Ok(Some(why))` is a machine that plainly cannot take a turn this minute -
/// asleep, off the network, or a key that no longer fits - and the turn is better off on this machine, with the person told
/// why. `Err` is only for a runner whose connection cannot be described at all (a bad target, or a missing key file).
pub fn reachable(ssh: &crate::ssh::Ssh) -> Result<Option<String>, String> {
    match ssh.run("true", Duration::from_secs(10)) {
        Ok(output) if output.ok() => Ok(None),
        Ok(output) => Ok(Some(format!("it answered {:?}", output.stderr.trim().chars().take(120).collect::<String>()))),
        Err(error) => Ok(Some(error.message)),
    }
}

/// A path for a shell: single-quoted, so a space or a quote in it cannot become a second command.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-runner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sdc")).unwrap();
        root
    }

    #[test]
    fn the_project_names_its_hosts_and_picks_one() {
        let root = project("pick");

        std::fs::write(file(&root), "[[runner]]\nname = \"build\"\nhost = \"me@build.example\"\ndir = \"/srv/app\"\n\n[[runner]]\nname = \"fast\"\nhost = \"me@fast.example\"\ndir = \"/srv/app\"\n").unwrap();

        assert_eq!(load(&root).unwrap().len(), 2);
        assert!(chosen(&root, None).unwrap().is_none(), "nobody has picked one yet");
        assert_eq!(chosen(&root, Some("fast")).unwrap().unwrap().name, "fast", "the saved default is enough");
        assert!(chosen(&root, Some("nowhere")).is_err(), "a saved default that names no host is said out loud");

        std::fs::write(root.join(".sdc/runner"), "# our build box\nbuild\n").unwrap();

        let picked = chosen(&root, Some("fast")).unwrap().unwrap();

        assert_eq!(picked.name, "build", "the project's own file wins over the saved default");
        assert_eq!(picked.dir, "/srv/app");

        let remote = remote_for(&root, None).unwrap().unwrap();

        assert_eq!(remote.target.user_host, "me@build.example");
        assert_eq!(remote.target.port, None);

        std::fs::write(root.join(".sdc/runner"), "typo\n").unwrap();

        assert!(chosen(&root, None).is_err(), "a name that is not in the file is said out loud, not ignored");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nothing_is_chosen_when_the_project_names_no_host() {
        let root = project("none");

        assert!(load(&root).unwrap().is_empty());
        assert!(chosen(&root, None).unwrap().is_none());
        assert!(remote_for(&root, None).unwrap().is_none(), "a project with no runner keeps this machine");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_with_a_space_stays_one_argument() {
        assert_eq!(quote("/srv/my app"), "'/srv/my app'");
        assert_eq!(quote("/srv/it's"), r"'/srv/it'\''s'");
    }
}
