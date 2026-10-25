//! **Tool hints (P0-8, switch `padma.tool_hints`)**: a file error says what it really is, where, and what is near. "os error 2" becomes
//! "this file does not exist", with the full path SDC looked at, the working folder, up to three names that do exist and are close to it,
//! and the `list_dir` call that shows the rest. A command that fails on a path with a space is told to quote it.

use std::path::{Path, PathBuf};

/// What an operating-system error number or message means for a file tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Windows 2, POSIX ENOENT.
    NoFile,
    /// Windows 3: a folder on the way is missing.
    NoFolder,
    /// Windows 5, POSIX EACCES / EPERM.
    Denied,
    /// Windows 32, POSIX EBUSY / ETXTBSY.
    Busy,
}

/// Which of the known errors a message holds, if any.
pub fn kind_of(message: &str) -> Option<Kind> {
    let lower = message.to_lowercase();
    let has = |needle: &str| lower.contains(needle);

    if has("os error 3)") || has("cannot find the path specified") || has("enotdir") {
        Some(Kind::NoFolder)
    } else if has("os error 2)") || has("cannot find the file specified") || has("no such file") || has("enoent") {
        Some(Kind::NoFile)
    } else if has("os error 5)") || has("access is denied") || has("permission denied") || has("eacces") || has("eperm") || has("operation not permitted") {
        Some(Kind::Denied)
    } else if has("os error 32)") || has("being used by another process") || has("ebusy") || has("etxtbsy") || has("text file busy") {
        Some(Kind::Busy)
    } else {
        None
    }
}

fn squash(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();

    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];

        row[0] = i + 1;

        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];

            row[j + 1] = (row[j + 1] + 1).min(row[j] + 1).min(diagonal + usize::from(x != y));
            diagonal = above;
        }
    }

    row[b.len()]
}

/// The nearest folder on the way to `path` that exists.
fn nearest_existing(path: &Path) -> PathBuf {
    let mut here = path.to_path_buf();

    while !here.is_dir() {
        if !here.pop() {
            break;
        }
    }

    here
}

/// Up to three names in `folder` that look like `wanted` (the same letters with other punctuation or case, or a small spelling change).
pub fn nearest(folder: &Path, wanted: &str) -> Vec<String> {
    let target = squash(wanted);
    let mut ranked: Vec<(usize, String)> = Vec::new();

    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let near = squash(&name);
        let cost = if near == target { 0 } else { distance(&near, &target) };
        let allowed = (target.chars().count() / 4).clamp(1, 3);

        if cost <= allowed || (target.len() >= 3 && (near.starts_with(&target) || target.starts_with(&near)) && near.len() >= 3) {
            ranked.push((cost, name));
        }
    }

    ranked.sort();
    ranked.into_iter().take(3).map(|(_, name)| name).collect()
}

fn shown(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).map(|rest| rest.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| path.to_string_lossy().to_string());

    if relative.is_empty() { ".".to_string() } else { relative }
}

/// The sentence a file tool's error becomes. `resolved` is the absolute path SDC looked at; `message` is what the operating system said.
/// An error this does not know is returned as it was.
pub fn explain(root: &str, resolved: &str, message: &str) -> String {
    let Some(kind) = kind_of(message) else {
        return message.to_string();
    };
    let (root_path, path) = (Path::new(root), Path::new(resolved));
    let cwd = root.trim_end_matches(['/', '\\']);

    match kind {
        Kind::NoFile | Kind::NoFolder => {
            let existing = nearest_existing(path);
            let missing_folder = kind == Kind::NoFolder || path.parent().is_some_and(|parent| !parent.is_dir());
            let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
            let mut text = if missing_folder {
                format!("The folder does not exist: {}. (It stopped at {}.)", path.parent().unwrap_or(path).display(), existing.display())
            } else {
                format!("The file does not exist: {}.", path.display())
            };

            text.push_str(&format!(" Working folder: {cwd}."));

            let mut near = if missing_folder { Vec::new() } else { nearest(&existing, &name) };

            if missing_folder {
                /* the folder the model meant may be spelled a little differently one level down */
                if let Some(next) = path.strip_prefix(&existing).ok().and_then(|rest| rest.components().next()) {
                    let wrong = next.as_os_str().to_string_lossy().to_string();

                    near = nearest(&existing, &wrong).into_iter().map(|found| shown(root_path, &existing.join(found))).collect();
                }
            } else {
                near = near.into_iter().map(|found| shown(root_path, &existing.join(found))).collect();
            }

            if !near.is_empty() {
                text.push_str(&format!(" Names that do exist and are close: {}.", near.join(", ")));
            }

            text.push_str(&format!(" Use list_dir with path \"{}\" to see what is there.", shown(root_path, &existing)));
            text.push_str(&format!(" [{}]", message.trim()));

            text
        }
        Kind::Denied => format!(
            "Access to {} was denied by the operating system. The file may be read-only, open in another program, or owned by another user. SDC does not change permissions; ask the person, or choose another path. [{}]",
            path.display(),
            message.trim()
        ),
        Kind::Busy => format!("{} is in use by another program, so it cannot be changed right now. Close what holds it (or wait), then try again. [{}]", path.display(), message.trim()),
    }
}

/// A line to add to a failed command's output when the failure is a path with a space that was not quoted. `None` when nothing points to it.
pub fn quote_hint(root: &str, command: &str, output: &str) -> Option<String> {
    let lower = output.to_lowercase();
    let lost = ["cannot find the path", "cannot find the file", "no such file or directory", "is not recognized", "command not found", "not a directory"].iter().any(|sign| lower.contains(sign));

    if !lost {
        return None;
    }

    /* a path with a space in it that the command names without quotes: the part before the first space is all the shell saw */
    let spaced = |path: &str| -> Option<String> {
        let head = path.split(' ').next().unwrap_or_default();

        (path.contains(' ') && head.len() >= 4 && command.contains(head) && !command.contains(&format!("\"{path}")) && !command.contains(&format!("'{path}"))).then(|| path.to_string())
    };

    let found = spaced(root.trim_end_matches(['/', '\\']));

    found.map(|path| format!("[SDC hint] A path with a space in it must be quoted: \"{path}\". The shell cut the command at the space."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-hints-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/my_file.rs"), "x").unwrap();
        std::fs::write(root.join("src/main.rs"), "x").unwrap();

        root
    }

    #[test]
    fn the_known_errors_are_told_apart() {
        assert_eq!(kind_of("a: The system cannot find the file specified. (os error 2)"), Some(Kind::NoFile));
        assert_eq!(kind_of("a: The system cannot find the path specified. (os error 3)"), Some(Kind::NoFolder));
        assert_eq!(kind_of("a: Access is denied. (os error 5)"), Some(Kind::Denied));
        assert_eq!(kind_of("EACCES: permission denied, open 'a'"), Some(Kind::Denied));
        assert_eq!(kind_of("a: The process cannot access the file because it is being used by another process. (os error 32)"), Some(Kind::Busy));
        assert_eq!(kind_of("ENOENT: no such file or directory"), Some(Kind::NoFile));
        assert_eq!(kind_of("old_text was not found in the file"), None);
    }

    #[test]
    fn a_missing_file_names_the_path_the_folder_and_what_is_close() {
        let root = folder("file");
        let resolved = root.join("src").join("my file.rs");
        let said = explain(root.to_str().unwrap(), resolved.to_str().unwrap(), "x: The system cannot find the file specified. (os error 2)");

        assert!(said.contains("does not exist") && said.contains("my file.rs") && said.contains("src/my_file.rs") && said.contains("list_dir"), "{said}");
        assert!(!said.contains("main.rs"), "only close names: {said}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_folder_says_folder() {
        let root = folder("folder");
        let resolved = root.join("no such dir").join("x.txt");
        let said = explain(root.to_str().unwrap(), resolved.to_str().unwrap(), "x: The system cannot find the path specified. (os error 3)");

        assert!(said.to_lowercase().contains("folder") && said.contains("does not exist") && said.contains("no such dir"), "{said}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn denied_and_busy_are_said_in_words_and_an_unknown_error_is_left_alone() {
        assert!(explain("/p", "/p/a", "a: Access is denied. (os error 5)").contains("denied"));
        assert!(explain("/p", "/p/a", "a: being used by another process. (os error 32)").contains("in use"));
        assert_eq!(explain("/p", "/p/a", "something else"), "something else");
    }

    #[test]
    fn a_command_cut_at_a_space_is_told_to_quote() {
        let hint = quote_hint("C:\\Users\\me\\My Project", "cd C:\\Users\\me\\My Project && dir", "The system cannot find the path specified.");

        assert!(hint.is_some_and(|text| text.contains("quoted")));
        assert!(quote_hint("C:\\a\\My Project", "cd \"C:\\a\\My Project\" && dir", "The system cannot find the path specified.").is_none());
        assert!(quote_hint("C:\\a\\My Project", "dir", "all fine").is_none());
    }
}
