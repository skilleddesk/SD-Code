use super::*;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    std::fs::create_dir_all(root.join("src/deep")).unwrap();
    std::fs::create_dir_all(root.join("node_modules/dep")).unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join(".sdc")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("src/b.rs"), "fn b() {}\n").unwrap();
    std::fs::write(root.join("src/deep/c.rs"), "fn c() {}\n").unwrap();
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    std::fs::write(root.join("node_modules/dep/index.js"), "module.exports = 1;\n").unwrap();
    std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

    dir
}

fn read(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

/// Every source file of a project with its content, for "the tree is exactly as it was".
fn tree(root: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().to_string();

            if rel.starts_with(".sdc") {
                continue;
            }

            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.insert(rel, std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }

    let mut out = BTreeMap::new();

    walk(root, root, &mut out);

    out
}

#[test]
fn the_shadow_is_a_working_copy_with_dependencies_linked_and_git_and_sdc_left_out() {
    let dir = project();
    let tx = Transaction::begin(dir.path(), "step-1").unwrap();

    assert_eq!(read(tx.shadow(), "src/a.rs").unwrap(), "fn a() {}\n");
    assert_eq!(read(tx.shadow(), "src/deep/c.rs").unwrap(), "fn c() {}\n");
    assert!(read(tx.shadow(), "node_modules/dep/index.js").is_some(), "a check needs its dependencies");
    assert!(!tx.shadow().join(".git").exists() && !tx.shadow().join(".sdc").exists());
}

#[test]
fn a_verified_step_reaches_the_real_tree_all_at_once() {
    let dir = project();
    let mut tx = Transaction::begin(dir.path(), "step-ok").unwrap();

    tx.write("src/a.rs", b"fn a() { 1 }\n").unwrap();
    tx.write("src/new/mod.rs", b"pub fn n() {}\n").unwrap();
    tx.delete("src/b.rs").unwrap();

    assert_eq!(read(dir.path(), "src/a.rs").unwrap(), "fn a() {}\n", "nothing is in the real tree before the commit");
    assert_eq!(read(tx.shadow(), "src/a.rs").unwrap(), "fn a() { 1 }\n", "but the shadow has it, for Verify");

    let receipt = tx.finish(true).unwrap();

    assert_eq!(receipt.outcome, Outcome::Committed);
    assert_eq!(receipt.changed, vec!["src/a.rs".to_string(), "src/b.rs".into(), "src/new/mod.rs".into()]);
    assert_eq!(read(dir.path(), "src/a.rs").unwrap(), "fn a() { 1 }\n");
    assert_eq!(read(dir.path(), "src/new/mod.rs").unwrap(), "pub fn n() {}\n");
    assert!(read(dir.path(), "src/b.rs").is_none());
    assert!(!dir.path().join(DIR).join("step-ok").exists(), "no leftovers");
}

/// The plan's test: a multi-file step that fails Verify leaves the repository exactly as it was.
#[test]
fn a_step_that_fails_verify_leaves_the_repository_unchanged() {
    let dir = project();
    let before = tree(dir.path());
    let mut tx = Transaction::begin(dir.path(), "step-red").unwrap();

    for (path, content) in [("src/a.rs", "broken 1"), ("src/b.rs", "broken 2"), ("src/deep/c.rs", "broken 3"), ("README.md", "broken 4"), ("src/fresh.rs", "broken 5")] {
        tx.write(path, content.as_bytes()).unwrap();
    }

    let receipt = tx.finish(false).unwrap();

    assert_eq!(receipt.outcome, Outcome::RolledBack);
    assert_eq!(tree(dir.path()), before, "five files edited in the shadow, not one byte in the real tree");
    assert!(!dir.path().join(DIR).join("step-red").exists());
}

#[test]
fn a_file_changed_meanwhile_is_a_conflict_and_nothing_at_all_is_written() {
    let dir = project();
    let mut tx = Transaction::begin(dir.path(), "step-conflict").unwrap();

    tx.write("src/a.rs", b"fn a() { step }\n").unwrap();
    tx.write("src/b.rs", b"fn b() { step }\n").unwrap();

    /* The person edits b.rs while the step is running. */
    std::fs::write(dir.path().join("src/b.rs"), "fn b() { the person's work }\n").unwrap();

    let before = tree(dir.path());
    let error = tx.finish(true).unwrap_err();

    assert_eq!(error, TxError::Conflict(vec!["src/b.rs".to_string()]));
    assert_eq!(tree(dir.path()), before, "neither file was written - the person's work is not overwritten");
}

#[test]
fn paths_that_leave_the_project_or_touch_sdc_or_git_are_refused() {
    let dir = project();
    let mut tx = Transaction::begin(dir.path(), "step-paths").unwrap();

    for bad in ["../escape.txt", "/etc/passwd", "src/../../x", ".sdc/policy.toml", ".git/config", "", "."] {
        assert!(matches!(tx.write(bad, b"x"), Err(TxError::BadPath(_))), "{bad:?}");
    }

    assert!(tx.changed().is_empty());
}

#[test]
fn a_step_with_no_edits_commits_nothing() {
    let dir = project();
    let receipt = Transaction::begin(dir.path(), "step-empty").unwrap().finish(true).unwrap();

    assert_eq!(receipt.outcome, Outcome::RolledBack);
}

#[test]
fn a_commit_is_a_time_machine_point_and_a_rollback_is_not() {
    let dir = project();
    let mut tx = Transaction::begin(dir.path(), "step-point").unwrap();

    tx.write("src/a.rs", b"x\n").unwrap();

    let receipt = tx.finish_with(true, |receipt| Some(format!("point-for-{}", receipt.id))).unwrap();

    assert_eq!(receipt.point.as_deref(), Some("point-for-step-point"));

    let mut red = Transaction::begin(dir.path(), "step-red-point").unwrap();

    red.write("src/a.rs", b"y\n").unwrap();

    assert_eq!(red.finish_with(false, |_| panic!("no point for a rolled-back step")).unwrap().point, None);
}

fn plant(root: &Path, id: &str, state: &str, entries: Vec<Entry>, pre: &[(usize, &str)]) {
    let dir = root.join(DIR).join(id);

    std::fs::create_dir_all(dir.join("pre")).unwrap();

    for (index, content) in pre {
        std::fs::write(dir.join("pre").join(index.to_string()), content).unwrap();
    }

    write_journal(&dir, &Journal { id: id.into(), state: state.into(), entries }).unwrap();
}

#[test]
fn recovery_rolls_back_a_half_applied_commit_finishes_a_done_one_and_discards_a_shadow() {
    let dir = project();
    let root = dir.path();

    /* A commit killed after it applied src/a.rs (modified) and src/new.rs (created) but before src/b.rs. */
    std::fs::write(root.join("src/a.rs"), "NEW a\n").unwrap();
    std::fs::write(root.join("src/new.rs"), "NEW file\n").unwrap();
    plant(
        root,
        "half",
        "committing",
        vec![
            Entry { path: "src/a.rs".into(), pre: Some("x".into()), post: Some("y".into()) },
            Entry { path: "src/b.rs".into(), pre: Some("x".into()), post: Some("y".into()) },
            Entry { path: "src/new.rs".into(), pre: None, post: Some("y".into()) },
        ],
        &[(0, "fn a() {}\n"), (1, "fn b() {}\n")],
    );

    /* A commit that finished but whose directory was not removed yet. */
    plant(root, "done", "committed", vec![], &[]);

    /* A transaction killed before its journal: only a shadow. */
    std::fs::create_dir_all(root.join(DIR).join("shadow-only/shadow")).unwrap();

    let mut recovered = recover(root);

    recovered.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(recovered[0], ("done".to_string(), Recovered::CleanedUp));
    assert!(matches!(&recovered[1], (id, Recovered::RolledBack(paths)) if id == "half" && paths.len() == 3), "{recovered:?}");
    assert_eq!(recovered[2], ("shadow-only".to_string(), Recovered::Discarded));

    assert_eq!(read(root, "src/a.rs").unwrap(), "fn a() {}\n", "the modified file is back");
    assert!(read(root, "src/new.rs").is_none(), "the created file is gone");
    assert_eq!(read(root, "src/b.rs").unwrap(), "fn b() {}\n");
    assert!(std::fs::read_dir(root.join(DIR)).unwrap().next().is_none(), "nothing left behind");
    assert!(recover(root).is_empty(), "recovery is idempotent");
}

#[test]
fn recovery_with_nothing_to_do_is_silent() {
    assert!(recover(project().path()).is_empty());
}
