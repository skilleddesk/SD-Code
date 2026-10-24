use super::*;

fn sh(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git").current_dir(root).args(args).output().unwrap();

    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// A repository with a committed file, a staged change, an unstaged change and an untracked file: a deliberately dirty tree.
fn dirty_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    sh(root, &["init", "-q", "-b", "main"]);
    sh(root, &["config", "user.email", "t@t.t"]);
    sh(root, &["config", "user.name", "t"]);
    std::fs::write(root.join(".gitignore"), "ignored.log\n").unwrap();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    std::fs::write(root.join("b.txt"), "two\n").unwrap();
    sh(root, &["add", "-A"]);
    sh(root, &["commit", "-q", "-m", "first"]);
    std::fs::write(root.join("a.txt"), "one staged\n").unwrap();
    sh(root, &["add", "a.txt"]);
    std::fs::write(root.join("b.txt"), "two edited\n").unwrap();
    std::fs::write(root.join("c.txt"), "untracked\n").unwrap();
    std::fs::write(root.join("ignored.log"), "noise\n").unwrap();
    dir
}

fn status(root: &Path) -> String {
    sh(root, &["status", "--porcelain=v1", "--untracked-files=all"])
}

#[test]
fn a_failed_step_returns_the_tree_to_its_exact_prior_state_and_git_status_is_identical() {
    crate::v3::forcing(&["nr.git_shadow"], || {
        let dir = dirty_repo();
        let root = dir.path();
        let before = status(root);
        let head = sh(root, &["rev-parse", "HEAD"]);
        let branches = sh(root, &["branch", "--list"]);
        let ran = run_step(root, "s1", &mut || {
            std::fs::write(root.join("a.txt"), "broken by the step\n").unwrap();
            std::fs::remove_file(root.join("b.txt")).unwrap();
            std::fs::write(root.join("new.txt"), "created by the step\n").unwrap();
            std::fs::create_dir_all(root.join("sub")).unwrap();
            std::fs::write(root.join("sub/deep.txt"), "x\n").unwrap();

            Err("the checks failed".to_string())
        })
        .unwrap();

        assert_eq!(ran, Ran::Failed("the checks failed".into()));
        assert_eq!(status(root), before, "git status is identical");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one staged\n");
        assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "two edited\n");
        assert_eq!(std::fs::read_to_string(root.join("c.txt")).unwrap(), "untracked\n");
        assert!(!root.join("new.txt").exists() && !root.join("sub/deep.txt").exists());
        assert_eq!(std::fs::read_to_string(root.join("ignored.log")).unwrap(), "noise\n", "ignored files are not touched");
        assert_eq!(sh(root, &["rev-parse", "HEAD"]), head, "no visible commit");
        assert_eq!(sh(root, &["branch", "--list"]), branches);
        assert_eq!(sh(root, &["for-each-ref", "refs/sdc/shadow/"]), "", "the hidden ref is dropped afterwards");
        assert!(!journal_path(root).exists() || Journal::load(root).steps.is_empty());
    });
}

#[test]
fn the_snapshot_is_a_hidden_ref_and_leaves_the_branch_and_the_index_alone() {
    let dir = dirty_repo();
    let root = dir.path();
    let before = status(root);
    let head = sh(root, &["rev-parse", "HEAD"]);
    let commit = snapshot(root, "s9").unwrap();

    assert_eq!(sh(root, &["rev-parse", "refs/sdc/shadow/s9"]).trim(), commit);
    assert_eq!(status(root), before);
    assert_eq!(sh(root, &["rev-parse", "HEAD"]), head);
    assert!(sh(root, &["log", "--oneline", "main"]).lines().count() == 1, "main has only its own commit");
    assert!(sh(root, &["ls-tree", "-r", "--name-only", &commit]).contains("c.txt"), "untracked files are in the snapshot");
    assert!(!sh(root, &["ls-tree", "-r", "--name-only", &commit]).contains("ignored.log"));
}

#[test]
fn a_crash_in_the_middle_leaves_no_half_written_file_and_a_restart_repeats_no_cloud_call() {
    crate::v3::forcing(&["nr.git_shadow"], || {
        let dir = dirty_repo();
        let root = dir.path();
        let mut cloud_calls = 0;

        /* step 1 completes */
        assert_eq!(run_step(root, "s1", &mut || { std::fs::write(root.join("one.txt"), "done\n").map_err(|e| e.to_string()) }).unwrap(), Ran::Done);

        /* step 2: the cloud answers (kept), the file is half written, then the process "dies" (a panic is caught here as the stand-in) */
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_step(root, "s2", &mut || {
                cloud_calls += 1;
                keep_answer(root, "s2", "src/two.rs", "the cloud's answer").unwrap();
                std::fs::write(root.join("two.txt"), "half wri").unwrap();
                panic!("power loss");
            })
        }));

        assert!(crashed.is_err());
        assert_eq!(Journal::load(root).steps.get("s2"), Some(&State::Started));

        /* restart */
        let touched = recover(root);

        assert_eq!(touched, vec!["s2".to_string()]);
        assert!(!root.join("two.txt").exists(), "no half-written file");
        assert!(root.join("one.txt").exists(), "the finished step stays");
        assert_eq!(kept_answer(root, "s2", "src/two.rs").as_deref(), Some("the cloud's answer"), "the answer survived, so it is not asked again");

        let ran = run_step(root, "s1", &mut || panic!("a done step is not run again")).unwrap();

        assert_eq!(ran, Ran::Skipped);

        let ran = run_step(root, "s2", &mut || {
            let answer = kept_answer(root, "s2", "src/two.rs").unwrap_or_else(|| {
                cloud_calls += 1;

                "asked again".into()
            });

            std::fs::write(root.join("two.txt"), answer).map_err(|e| e.to_string())
        })
        .unwrap();

        assert_eq!(ran, Ran::Done);
        assert_eq!(cloud_calls, 1, "one cloud call in all");
        assert_eq!(std::fs::read_to_string(root.join("two.txt")).unwrap(), "the cloud's answer");
    });
}

#[test]
fn a_retry_replaces_its_answer_and_never_appends() {
    let dir = tempfile::tempdir().unwrap();

    keep_answer(dir.path(), "s1", "t", "first").unwrap();
    keep_answer(dir.path(), "s1", "t", "second").unwrap();
    keep_answer(dir.path(), "s1", "other", "x").unwrap();

    assert_eq!(kept_answer(dir.path(), "s1", "t").as_deref(), Some("second"));
    assert_eq!(kept_answer(dir.path(), "s1", "other").as_deref(), Some("x"));
    assert!(keep_answer(dir.path(), "../s1", "t", "x").is_err());
}

#[test]
fn off_by_default_and_outside_a_repository_the_body_just_runs() {
    let dir = tempfile::tempdir().unwrap();
    let mut ran_body = 0;

    assert_eq!(run_step(dir.path(), "s1", &mut || { ran_body += 1; Ok(()) }).unwrap(), Ran::Done);
    crate::v3::forcing(&["nr.git_shadow"], || assert_eq!(run_step(dir.path(), "s1", &mut || { ran_body += 1; Ok(()) }).unwrap(), Ran::Done));
    assert_eq!(ran_body, 2);
    assert!(!dir.path().join(".sdc/shadow/journal.json").exists(), "nothing was journalled");
}
