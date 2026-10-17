use super::*;

fn policy(root: &Path) -> Policy {
    Policy::for_project(root)
}

#[test]
fn every_row_of_the_table_is_what_the_platform_can_really_back() {
    let mac = guarantees_for("macos", true, false, false);

    assert_eq!((mac.filesystem, mac.network, mac.limits), (Level::Enforced, Level::Enforced, Level::Partial));
    assert!(mac.notes.iter().any(|note| note.contains("memory is not limited")));
    assert_eq!(guarantees_for("macos", false, false, false).filesystem, Level::None, "no sandbox-exec, no claim");

    let linux = guarantees_for("linux", false, true, true);

    assert_eq!((linux.filesystem, linux.network, linux.limits), (Level::Enforced, Level::Enforced, Level::Enforced));

    let only_unshare = guarantees_for("linux", false, false, true);

    assert_eq!((only_unshare.filesystem, only_unshare.network), (Level::None, Level::Enforced));
    assert!(only_unshare.notes[0].contains("NOT refused"), "the gap is stated, not hidden");

    let windows = guarantees_for("windows", false, false, false);

    assert_eq!((windows.filesystem, windows.network), (Level::None, Level::None));
    assert!(windows.notes[0].contains("do not run code you do not trust"));
}

#[test]
fn the_seatbelt_profile_denies_writes_and_the_network_and_names_only_the_allowed_folders() {
    let root = tempfile::tempdir().unwrap();
    let profile = macos_profile(&policy(root.path()), Some(Path::new("/Users/someone")));

    assert!(profile.starts_with("(version 1)\n(allow default)\n(deny file-write*)"));
    assert!(profile.contains(&format!("(subpath \"{}\")", std::fs::canonicalize(root.path()).unwrap().display())), "the project, as the kernel sees it");
    assert!(profile.contains("(deny network*)"));
    assert!(!profile.contains("(allow network*"), "no allowance unless the project asks");
    assert!(profile.contains("(deny file-read* (subpath \"/Users/someone/.ssh\"))") && profile.contains("Library/Keychains"));

    let allowed = macos_profile(&Policy { network: Network::Allow(vec!["10.0.0.5:5432".into()]), ..policy(root.path()) }, None);

    assert!(allowed.contains("(allow network*") && allowed.contains("(remote ip \"10.0.0.5:5432\")") && allowed.contains("localhost:*"));
}

#[test]
fn the_bubblewrap_line_is_read_only_with_the_project_bound_writable_and_no_network() {
    let root = tempfile::tempdir().unwrap();
    let args = bwrap_args(&policy(root.path())).join(" ");
    let project = std::fs::canonicalize(root.path()).unwrap().display().to_string();

    assert!(args.starts_with("--ro-bind / /"));
    assert!(args.contains(&format!("--bind {project} {project}")));
    assert!(args.contains("--unshare-net") && args.contains("--die-with-parent") && args.contains("--tmpfs /tmp"));
    assert!(!bwrap_args(&Policy { network: Network::Allow(vec![]), ..policy(root.path()) }).contains(&"--unshare-net".to_string()));
}

#[test]
fn limits_are_a_ulimit_prefix_and_memory_only_where_the_kernel_honours_it() {
    let policy = Policy { cpu_seconds: 30, file_mib: 5, memory_mib: 512, ..policy(Path::new("/p")) };

    assert_eq!(limits(&policy, false), "ulimit -t 30; ulimit -f 10240; ");
    assert_eq!(limits(&policy, true), "ulimit -t 30; ulimit -f 10240; ulimit -v 524288; ");
    assert_eq!(limits(&Policy { cpu_seconds: 0, file_mib: 0, ..policy }, false), "");
}

#[test]
fn a_system_with_no_sandbox_says_so_instead_of_running_the_line_bare() {
    let root = tempfile::tempdir().unwrap();
    let windows = guarantees_for("windows", false, false, false);
    let error = wrap_line_for("windows", "npm test", &policy(root.path()), &windows).unwrap_err();

    assert!(error.contains("no sandbox on this system"), "{error}");
    assert!(wrap_line_for("linux", "npm test", &policy(root.path()), &guarantees_for("linux", false, true, true)).unwrap().starts_with("bwrap "));
    assert!(wrap_line_for("linux", "npm test", &policy(root.path()), &guarantees_for("linux", false, false, true)).unwrap().starts_with("unshare -rn "));
}

#[test]
fn quoting_survives_a_quote_in_the_command() {
    assert_eq!(quote("echo 'hi'"), "'echo '\\''hi'\\'''");
}

/// The real thing, on the system it exists on: a command inside the sandbox writes in the project and is refused
/// everywhere else, and cannot reach the network. Runs only where `sandbox-exec` is installed.
#[cfg(target_os = "macos")]
#[test]
fn on_macos_a_command_cannot_write_outside_the_project_or_touch_the_network() {
    if !Path::new("/usr/bin/sandbox-exec").is_file() {
        return;
    }

    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let escape = outside.path().join("escape.txt");
    let policy = Policy { writable: vec![project.path().to_path_buf()], ..Policy::for_project(project.path()) };
    let run = |line: &str| -> (bool, String) {
        let wrapped = wrap_line(line, &policy).expect("sandbox-exec is installed");
        let output = std::process::Command::new("/bin/sh").arg("-c").arg(&wrapped).current_dir(project.path()).output().unwrap();

        (output.status.success(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
    };

    assert!(run("echo built > artefact.txt && cat artefact.txt").0, "writing in the project works");
    assert_eq!(std::fs::read_to_string(project.path().join("artefact.txt")).unwrap().trim(), "built");

    let (ok, text) = run(&format!("echo escaped > '{}'", escape.display()));

    assert!(!ok && !escape.exists(), "a write outside the project is refused: {text}");

    let (ok, _) = run(&format!("mkdir '{}'", outside.path().join("dir").display()));

    assert!(!ok, "so is making a folder");

    let (ok, text) = run("python3 -c \"import socket; s = socket.socket(); s.settimeout(2); s.connect(('127.0.0.1', 9))\"");

    assert!(!ok && text.contains("Operation not permitted"), "the network is refused, not merely unreachable: {text}");

    let (ok, _) = run("ls ~/.ssh >/dev/null 2>&1; test -d ~/.ssh && cat ~/.ssh/* >/dev/null 2>&1");

    assert!(!ok || !dirs::home_dir().unwrap().join(".ssh").exists(), "credentials are not readable");
}

/// The honest-check test: the sandbox must not break an ordinary build. `cargo` writes to `~/.cargo` and builds in the
/// project; inside the sandbox it still has to succeed, and the artefact lands in the project.
#[cfg(target_os = "macos")]
#[test]
fn on_macos_an_ordinary_cargo_build_still_works_inside_the_sandbox() {
    if !Path::new("/usr/bin/sandbox-exec").is_file() || std::process::Command::new("cargo").arg("--version").output().is_err() {
        return;
    }

    let project = tempfile::tempdir().unwrap();

    std::fs::write(project.path().join("Cargo.toml"), "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/main.rs"), "fn main() { println!(\"sandboxed\"); }\n").unwrap();

    let wrapped = wrap_line("cargo run --offline -q", &Policy::for_project(project.path())).unwrap();
    let output = std::process::Command::new("/bin/sh").arg("-c").arg(&wrapped).current_dir(project.path()).output().unwrap();

    assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "sandboxed");
    assert!(project.path().join("target").is_dir(), "the build output is in the project");
}
