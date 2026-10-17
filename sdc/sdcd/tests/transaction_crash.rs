//! **`kill -9` in the middle of a commit (P0-14)**: a child process commits a step and is made to die abruptly - no
//! destructors, no flushing - at every instant the commit protocol has, and after each death `recover` must leave the
//! project as it was (a commit that died before it finished is undone), or as the step made it (one that died after the
//! journal said `committed`), and never a mixture.
//!
//! The child is this very test binary run with `SDC_TX_CHILD=1`; the failpoints are `runtime::transaction`'s own, which
//! abort the process where a `kill -9` could land.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use sdcd::runtime::transaction::{recover, Transaction};

const FILES: usize = 6;

fn seed(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();

    for index in 0..FILES {
        std::fs::write(root.join(format!("src/f{index}.rs")), format!("fn f{index}() {{}} // original\n")).unwrap();
    }

    std::fs::write(root.join("keep.txt"), "untouched\n").unwrap();
}

fn tree(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();

    for dir in [root.to_path_buf(), root.join("src")] {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            if entry.path().is_file() {
                out.insert(entry.path().strip_prefix(root).unwrap().to_string_lossy().to_string(), std::fs::read_to_string(entry.path()).unwrap());
            }
        }
    }

    out
}

/// The step: modify four files, create one, delete one.
fn run_step(root: &Path) {
    let mut tx = Transaction::begin(root, "crash-step").unwrap();

    for index in 0..4 {
        tx.write(&format!("src/f{index}.rs"), format!("fn f{index}() {{}} // CHANGED\n").as_bytes()).unwrap();
    }

    tx.write("src/created.rs", b"fn created() {}\n").unwrap();
    tx.delete("src/f5.rs").unwrap();
    tx.finish(true).unwrap();
}

/// What the child does: not a test of its own - it only runs when the parent launches it.
#[test]
fn child_commit() {
    if let Ok(root) = std::env::var("SDC_TX_CHILD_ROOT") {
        run_step(Path::new(&root));
    }
}

#[test]
fn a_commit_killed_at_every_instant_leaves_the_old_tree_or_the_new_one_never_a_mixture() {
    let reference = tempfile::tempdir().unwrap();

    seed(reference.path());

    let original = tree(reference.path());

    run_step(reference.path());

    let finished = tree(reference.path());

    assert_ne!(original, finished);

    /* 6 pre-image points (one per touched file), the journal, 6 apply points, and the instant after "committed". */
    let mut points: Vec<String> = (0..6).map(|index| format!("pre-image:{index}")).collect();

    points.push("journal:0".into());
    points.extend((1..=6).map(|index| format!("apply:{index}")));
    points.push("committed:0".into());

    assert!(points.len() >= 14);

    for point in &points {
        let dir = tempfile::tempdir().unwrap();

        seed(dir.path());

        let status = Command::new(std::env::current_exe().unwrap())
            .args(["child_commit", "--exact", "--nocapture", "--test-threads", "1"])
            .env("SDC_TX_CHILD_ROOT", dir.path())
            .env("SDC_TX_FAILPOINT", point)
            .status()
            .unwrap();

        assert!(!status.success(), "{point}: the child was meant to die");

        /* The control: before recovery a kill in the middle of applying really does leave a mixture, so what recovery
           repairs below is a real problem and not an easy one. */
        if let Some(applied) = point.strip_prefix("apply:").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..6).contains(n)) {
            let mixed = tree(dir.path());

            assert!(mixed != original && mixed != finished, "{point}: {applied} of 6 applied should be a mixture");
        }

        let recovered = recover(dir.path());
        let now = tree(dir.path());

        if point == "committed:0" {
            assert_eq!(now, finished, "{point}: the journal said committed, so the step stands: {recovered:?}");
        } else {
            assert_eq!(now, original, "{point}: a commit that did not finish is undone completely: {recovered:?}");
        }

        assert!(recover(dir.path()).is_empty(), "{point}: nothing left for a second recovery");
        assert!(!dir.path().join(".sdc/tx/crash-step").exists(), "{point}: no leftovers");
    }
}

#[test]
fn a_commit_that_is_not_killed_is_the_new_tree() {
    let dir = tempfile::tempdir().unwrap();

    seed(dir.path());
    run_step(dir.path());

    assert_eq!(std::fs::read_to_string(dir.path().join("src/f0.rs")).unwrap(), "fn f0() {} // CHANGED\n");
    assert!(!dir.path().join("src/f5.rs").exists() && dir.path().join("src/created.rs").exists());
    assert_eq!(std::fs::read_to_string(dir.path().join("keep.txt")).unwrap(), "untouched\n");
}
