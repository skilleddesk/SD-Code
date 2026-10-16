//! **Claude credential guard (P0-2)**: SDC only launches the official `claude` CLI, where the person signed in
//! themselves. It never reads, copies or proxies the CLI's OAuth token. This test fails when any source file names a
//! place where that token lives, or the token's own variable - the only allowed mention is `crash.rs`'s
//! "is the sign-in file there" check, which asks the file system whether a path exists and never opens it.

use std::path::{Path, PathBuf};

const FORBIDDEN: [&str; 4] = [".credentials.json", "Claude Code-credentials", "CLAUDE_CODE_OAUTH_TOKEN", "claudeAiOauth"];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();

        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") && !path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("._")) {
            out.push(path);
        }
    }
}

#[test]
fn no_code_path_reads_a_cli_sign_in() {
    let mut files = Vec::new();

    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);

    let mut offences = Vec::new();

    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let existence_check_only = file.ends_with("crash.rs");

        for (number, line) in text.lines().enumerate() {
            if FORBIDDEN.iter().any(|needle| line.contains(needle)) && !(existence_check_only && !line.contains("read")) {
                offences.push(format!("{}:{}: {}", file.display(), number + 1, line.trim()));
            }
        }
    }

    assert!(offences.is_empty(), "a CLI's sign-in must never be read by SDC:\n{}", offences.join("\n"));
}

#[test]
fn crash_rs_only_checks_that_a_sign_in_file_exists() {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/crash.rs")).unwrap();

    assert!(!text.contains("read_to_string(file") && !text.contains("File::open(file"), "crash.rs must not open a sign-in file");
    assert!(text.contains("files.iter().any(|file| file.exists())"));
}
