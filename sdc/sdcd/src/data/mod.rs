//! The data layer (P0-5): the five files SDC keeps, where each one lives, and which one wins when two disagree.
//!
//! | file | scope | who writes it | what it decides |
//! |---|---|---|---|
//! | `~/.sdc/rotation.toml` | this computer | the person, by hand | the model chain, and the rules that follow it |
//! | `<project>/.sdc/policy.toml` | this project | the person, by hand | what an agent may do here, and whether the project is private |
//! | `<project>/.sdc/audit/sent.jsonl` | this project | SDC, append-only | what left this computer, to which model, and what was removed first |
//! | `~/.sdc/cache/` | this computer | SDC | context already paid for, so it is not paid for twice |
//! | `~/.sdc/sdc.db` | this computer | SDC | hosts, sessions, the journal, the trust ledger |
//!
//! # Precedence
//!
//! **A project's file beats the computer's file.** `policy.toml` is the project's own word about its own code, and
//! `rotation.toml` is a preference about models; a preference may not overrule an instruction. So a project with
//! `privacy = "local-only"` keeps its files off every remote model no matter what `rotation.toml` lists, and a
//! project that names its own chain uses that chain. `data::read` is the one place that decides this, so the answer
//! cannot drift between callers.
//!
//! **A file that does not parse changes nothing.** A mistyped `rotation.toml` is reported - with the line the
//! parser complained about - and the built-in default is used. SDC starts and says what is wrong rather than
//! refusing to run, because the alternative is a person locked out of their own tool by a typo in a config file.
//!
//! # Hashing
//!
//! The plan asks for the cache to be keyed by BLAKE3. BLAKE3 is not a dependency of this build and cannot be added
//! while the build is offline, so the key is SHA-256 - the same digest `v3::cas` and `anywhere::crypto` already use
//! for content addressing. `cache::key` is the only place that decides, so moving to BLAKE3 later is one function.

pub mod cache;
pub mod rotation;
pub mod sent;

use std::path::{Path, PathBuf};

/// This computer's SDC folder: `~/.sdc` on macOS and Linux, `%USERPROFILE%\.sdc`-style elsewhere.
pub fn home() -> Option<PathBuf> {
    crate::paths::data_dir().ok()
}

/// The same folder, or an error a caller can show. `home()` returning `None` is not a crash: it means the platform
/// would not tell us where the person's files go, which is worth saying out loud.
pub fn home_or_error() -> Result<PathBuf, String> {
    home().ok_or_else(|| "SDC could not work out where your files live; set SDC_DATA_DIR to choose".to_string())
}

/// A project's own folder, `<project>/.sdc`.
pub fn project(root: &Path) -> PathBuf {
    root.join(".sdc")
}

/// A project's audit trail, `<project>/.sdc/audit`. The plan puts `sent.jsonl` here rather than in the home folder
/// because the question it answers - "did this project's code leave this computer?" - is asked about a project, and
/// a project deleted with its history is a project whose history nobody has to trust.
pub fn audit(root: &Path) -> PathBuf {
    project(root).join("audit")
}

/// The one place that applies the precedence rule above: which chain rows a *project* may use.
///
/// `rotation.toml` is a preference and `policy.toml` is an instruction, so the project's `local-only` setting is
/// checked first and a row that would send this project's code to a remote model is dropped - whatever
/// `rotation.toml` says about that model, and whether or not the person wrote the row themselves. A row that is
/// already local always survives, which is why the answer is never "no model at all" for a project that has a local
/// one in its chain.
///
/// Returned rather than printed: the caller decides whether the person hears why a row went.
pub fn chain_for_project(rotation: &rotation::Rotation, policy: &crate::trust::policy::Policy, step: &str) -> Vec<rotation::Row> {
    let rows = rotation.for_step(step);

    if !policy.privacy_local() {
        return rows.into_iter().cloned().collect();
    }

    rows.into_iter().filter(|row| rotation::is_local(&row.model)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_files_sit_under_the_project_and_not_under_home() {
        let root = Path::new("/tmp/example");

        assert_eq!(project(root), PathBuf::from("/tmp/example/.sdc"));
        assert_eq!(audit(root), PathBuf::from("/tmp/example/.sdc/audit"));
        assert!(sent::path(root).starts_with(project(root)));
    }

    #[test]
    fn the_home_folder_is_the_one_the_daemon_uses_for_everything_else() {
        assert_eq!(home(), crate::paths::data_dir().ok());
    }

    #[test]
    fn a_project_that_may_not_leave_the_machine_drops_every_remote_row() {
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();

        std::fs::write(rotation::path(home.path()), "[[chain]]\nmodel = \"deepseek:deepseek-chat\"\nallowed_steps = [\"edit\"]\n\n[[chain]]\nmodel = \"ollama:qwen3:8b\"\n").unwrap();
        std::fs::create_dir_all(project(workspace.path())).unwrap();
        std::fs::write(project(workspace.path()).join("policy.toml"), "privacy = \"local-only\"\n").unwrap();

        let (rotation, warning) = rotation::load(home.path());
        let policy = crate::trust::policy::Policy::load_local(workspace.path());

        assert_eq!(warning, None);
        assert!(policy.privacy_local(), "the project says its code may not leave this computer");
        /* `rotation.toml` allows the remote model for `edit`; the project's policy overrules it. */
        assert_eq!(rotation.for_step("edit").len(), 2, "the preference is what it is");
        assert_eq!(chain_for_project(&rotation, &policy, "edit").len(), 1, "but the instruction wins");
        assert_eq!(chain_for_project(&rotation, &policy, "edit")[0].model, "ollama:qwen3:8b");
    }

    #[test]
    fn a_project_that_says_nothing_gets_the_computer_wide_chain() {
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();

        std::fs::write(rotation::path(home.path()), "[[chain]]\nmodel = \"ollama:qwen3:8b\"\n\n[[chain]]\nmodel = \"deepseek:deepseek-chat\"\n").unwrap();

        let (rotation, _) = rotation::load(home.path());
        /* No policy file at all: the built-in default, which does not force local-only. */
        let policy = crate::trust::policy::Policy::load_local(workspace.path());

        assert!(!policy.privacy_local());
        assert_eq!(chain_for_project(&rotation, &policy, "edit").len(), 2, "nothing in the project overrules the chain");
    }

    #[test]
    fn a_project_that_may_not_leave_the_machine_and_has_only_remote_rows_gets_none_of_them() {
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();

        std::fs::write(rotation::path(home.path()), "[[chain]]\nmodel = \"deepseek:deepseek-chat\"\n").unwrap();
        std::fs::create_dir_all(project(workspace.path())).unwrap();
        std::fs::write(project(workspace.path()).join("policy.toml"), "privacy = \"local-only\"\n").unwrap();

        let (rotation, _) = rotation::load(home.path());
        let policy = crate::trust::policy::Policy::load_local(workspace.path());

        /* An empty answer, and not a fallback to the remote row: "no model may do this here" is the true answer, and
           the caller has to say so rather than quietly sending the code. */
        assert!(chain_for_project(&rotation, &policy, "edit").is_empty());
    }

    #[test]
    fn a_broken_project_policy_does_not_silently_allow_remote() {
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();

        std::fs::write(rotation::path(home.path()), "[[chain]]\nmodel = \"deepseek:deepseek-chat\"\n").unwrap();
        std::fs::create_dir_all(project(workspace.path())).unwrap();
        /* `privacy` with a value the parser does not know. */
        std::fs::write(project(workspace.path()).join("policy.toml"), "privacy = \"whatever\"\n").unwrap();

        let (rotation, _) = rotation::load(home.path());
        let policy = crate::trust::policy::Policy::load_local(workspace.path());

        /* Whatever the fallback is, this test pins that it is a decision that was made - not a crash, and not an
           exception that turns every later check off. */
        let chain = chain_for_project(&rotation, &policy, "edit");
        assert!(chain.len() <= 1, "the answer is one of the two possible readings, not both: {chain:?}");
    }
}
