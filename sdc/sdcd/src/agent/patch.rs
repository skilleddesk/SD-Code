//! `apply_patch` (0.14): Codex's own edit format, for the models trained on it.
//!
//! GPT and Codex models edit files by writing one patch that can add, change, move and delete several files
//! at once. Given only `edit_file` they spend a call per change and often reach for the patch anyway. This
//! reads that format and applies it hunk by hunk: each hunk's context and removed lines must be found in
//! the file as they are, so a patch written against an old version fails with the hunk it could not place,
//! and the model re-reads the file rather than SDC guessing.
//!
//! ```text
//! *** Begin Patch
//! *** Update File: src/math.js
//! @@ function add
//! -  return a - b;
//! +  return a + b;
//! *** Add File: src/mul.js
//! +module.exports = (a, b) => a * b;
//! *** Delete File: old.js
//! *** End Patch
//! ```

/// One file's change.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Add { path: String, text: String },
    Delete { path: String },
    Update { path: String, move_to: Option<String>, hunks: Vec<Hunk> },
}

/// Lines to find (context and removed) and what they become (context and added).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hunk {
    pub old: Vec<String>,
    pub new: Vec<String>,
}

/// The patch, parsed; a malformed one is a sentence the model can act on.
pub fn parse(patch: &str) -> Result<Vec<Change>, String> {
    let lines: Vec<&str> = patch.lines().map(|line| line.strip_suffix('\r').unwrap_or(line)).collect();
    let start = lines.iter().position(|line| line.trim() == "*** Begin Patch").ok_or("the patch must start with `*** Begin Patch`")?;
    let mut changes = Vec::new();
    let mut index = start + 1;

    while index < lines.len() {
        let line = lines[index];

        if line.trim() == "*** End Patch" {
            return Ok(changes);
        }

        if let Some(path) = line.strip_prefix("*** Add File: ") {
            let mut text = Vec::new();

            index += 1;

            while index < lines.len() && !lines[index].starts_with("*** ") {
                text.push(lines[index].strip_prefix('+').ok_or_else(|| format!("in `Add File: {path}` every line starts with +; this one does not: {}", lines[index]))?.to_string());
                index += 1;
            }

            let mut text = text.join("\n");

            text.push('\n');
            changes.push(Change::Add { path: path.trim().to_string(), text });
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            changes.push(Change::Delete { path: path.trim().to_string() });
            index += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Update File: ") {
            let mut move_to = None;
            let mut hunks: Vec<Hunk> = Vec::new();
            let mut current = Hunk::default();

            index += 1;

            if let Some(target) = lines.get(index).and_then(|line| line.strip_prefix("*** Move to: ")) {
                move_to = Some(target.trim().to_string());
                index += 1;
            }

            while index < lines.len() && !(lines[index].starts_with("*** ") && lines[index].trim() != "*** End of File") {
                let line = lines[index];

                if line.starts_with("@@") || line.trim() == "*** End of File" {
                    if !current.old.is_empty() || !current.new.is_empty() {
                        hunks.push(std::mem::take(&mut current));
                    }
                } else if let Some(rest) = line.strip_prefix('+') {
                    current.new.push(rest.to_string());
                } else if let Some(rest) = line.strip_prefix('-') {
                    current.old.push(rest.to_string());
                } else {
                    let rest = line.strip_prefix(' ').unwrap_or(line);

                    current.old.push(rest.to_string());
                    current.new.push(rest.to_string());
                }

                index += 1;
            }

            if !current.old.is_empty() || !current.new.is_empty() {
                hunks.push(current);
            }

            if hunks.is_empty() && move_to.is_none() {
                return Err(format!("`Update File: {path}` has no hunks"));
            }

            changes.push(Change::Update { path: path.trim().to_string(), move_to, hunks });
            continue;
        }

        if line.trim().is_empty() {
            index += 1;
            continue;
        }

        return Err(format!("the patch has a line SDC cannot place: `{line}` - each file starts with `*** Add File:`, `*** Update File:` or `*** Delete File:`"));
    }

    Err("the patch must end with `*** End Patch`".to_string())
}

/// `text` with every hunk applied in order, or the hunk that does not fit.
pub fn apply(text: &str, hunks: &[Hunk]) -> Result<String, String> {
    let ends_with_newline = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut from = 0;

    for (number, hunk) in hunks.iter().enumerate() {
        if hunk.old.is_empty() {
            /* Only added lines: they go at the end of the file. */
            lines.extend(hunk.new.iter().cloned());
            continue;
        }

        let place = find(&lines, &hunk.old, from, |a, b| a == b)
            .or_else(|| find(&lines, &hunk.old, 0, |a, b| a == b))
            .or_else(|| find(&lines, &hunk.old, 0, |a, b| a.trim_end() == b.trim_end()))
            .or_else(|| find(&lines, &hunk.old, 0, |a, b| a.trim() == b.trim()))
            .ok_or_else(|| {
                format!(
                    "hunk {} does not match the file - these lines were not found as written:\n{}\nRead the file again and send a patch against what is there now.",
                    number + 1,
                    hunk.old.iter().take(12).map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n")
                )
            })?;

        lines.splice(place..place + hunk.old.len(), hunk.new.iter().cloned());
        from = place + hunk.new.len();
    }

    let mut joined = lines.join("\n");

    if ends_with_newline || text.is_empty() {
        joined.push('\n');
    }

    Ok(joined)
}

fn find(lines: &[String], wanted: &[String], from: usize, same: impl Fn(&str, &str) -> bool) -> Option<usize> {
    if wanted.len() > lines.len() {
        return None;
    }

    (from..=lines.len() - wanted.len()).find(|&start| wanted.iter().enumerate().all(|(offset, line)| same(&lines[start + offset], line)))
}

/// The models that write this format: OpenAI's GPT, o-series and Codex.
pub fn speaks_patch(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    let bare = model.rsplit('/').next().unwrap_or(&model);

    bare.starts_with("gpt-") || bare.contains("codex") || ["o1", "o3", "o4"].iter().any(|prefix| bare.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "*** Begin Patch\n*** Update File: src/math.js\n@@ function add(a, b) {\n-  return a - b;\n+  return a + b;\n }\n*** Add File: src/mul.js\n+module.exports = (a, b) => a * b;\n*** Delete File: old.js\n*** End Patch\n";

    #[test]
    fn a_codex_patch_is_read_file_by_file() {
        let changes = parse(PATCH).unwrap();

        assert_eq!(changes.len(), 3);
        assert_eq!(changes[1], Change::Add { path: "src/mul.js".into(), text: "module.exports = (a, b) => a * b;\n".into() });
        assert_eq!(changes[2], Change::Delete { path: "old.js".into() });

        let Change::Update { path, hunks, .. } = &changes[0] else { panic!("an update") };

        assert_eq!(path, "src/math.js");
        assert_eq!(hunks[0].old, vec!["  return a - b;", "}"]);
        assert_eq!(hunks[0].new, vec!["  return a + b;", "}"]);
    }

    #[test]
    fn a_hunk_applies_where_its_lines_are_and_refuses_where_they_are_not() {
        let text = "function add(a, b) {\n  return a - b;\n}\n\nmodule.exports = { add };\n";
        let Change::Update { hunks, .. } = &parse(PATCH).unwrap()[0] else { panic!() };

        assert_eq!(apply(text, hunks).unwrap(), "function add(a, b) {\n  return a + b;\n}\n\nmodule.exports = { add };\n");

        let error = apply("something else\n", hunks).unwrap_err();

        assert!(error.contains("hunk 1 does not match") && error.contains("return a - b;"), "{error}");
    }

    #[test]
    fn a_broken_patch_says_what_is_wrong() {
        assert!(parse("*** Update File: a").unwrap_err().contains("Begin Patch"));
        assert!(parse("*** Begin Patch\n*** Update File: a\n-x\n+y\n").unwrap_err().contains("End Patch"));
        assert!(parse("*** Begin Patch\nhello\n*** End Patch").unwrap_err().contains("cannot place"));
    }

    #[test]
    fn only_the_models_that_write_patches_are_given_the_tool() {
        assert!(speaks_patch("gpt-5.1-codex"));
        assert!(speaks_patch("openai/o4-mini"));
        assert!(!speaks_patch("deepseek-chat"));
        assert!(!speaks_patch("claude-opus-5-5"));
    }
}
