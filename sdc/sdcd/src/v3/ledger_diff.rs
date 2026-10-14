//! **Changed re-reads as diffs** (v3 I1, switch `v3.ledger_diff`, a token technique - off until it passes Q3).
//!
//! v1 already answers an *unchanged* re-read with one line. A file that changed since the turn read it (the agent
//! edited it, a command rewrote it) was sent whole again; with this switch it goes back as a unified diff against
//! the earlier read - which is still in the conversation - when the diff is clearly smaller.
//!
//! Only whole-file reads take part (a range has its own numbering), and the earlier reads are forgotten whenever
//! the conversation folds old output away, exactly like the unchanged-read stub.

use std::collections::HashMap;

use similar::TextDiff;

/// A diff is sent only when it is at most this share of the whole file.
const MAX_SHARE: f64 = 0.6;

/// The read's text without SDC's `    12\t` line-number column.
fn plain(numbered: &str) -> String {
    numbered.lines().map(|line| line.split_once('\t').map(|(_, text)| text).unwrap_or(line)).collect::<Vec<_>>().join("\n") + "\n"
}

fn whole_read(input: &serde_json::Value) -> Option<String> {
    (input["offset"].is_null() && input["limit"].is_null()).then(|| input["path"].as_str().map(|path| path.trim().replace('\\', "/").trim_start_matches("./").to_string())).flatten()
}

/// Replaces each changed whole-file re-read with a diff against the turn's earlier read of it; remembers every
/// whole read. Answers the tokens saved (SDC's count).
pub fn apply(prior: &mut HashMap<String, String>, uses: &[crate::agent::dialect::ToolUse], results: &mut [crate::agent::dialect::ToolResult]) -> u64 {
    let mut saved = 0;

    for (call, result) in uses.iter().zip(results.iter_mut()) {
        if call.name != "read_file" || result.2 || result.1.starts_with('[') {
            continue;
        }

        let Some(path) = whole_read(&call.input) else {
            continue;
        };
        let now = plain(&result.1);

        if let Some(before) = prior.get(&path).filter(|before| **before != now) {
            let diff = TextDiff::from_lines(before.as_str(), now.as_str()).unified_diff().context_radius(2).header("as read earlier", "now").to_string();
            let text = format!("[{path} changed since you read it earlier in this turn. The diff from that read (@@ numbers are line numbers):]\n{diff}");

            if (text.len() as f64) <= result.1.len() as f64 * MAX_SHARE {
                saved += crate::context::tokens_of(&result.1).saturating_sub(crate::context::tokens_of(&text));
                result.1 = text;
            }
        }

        prior.insert(path, now);
    }

    saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn numbered(text: &str) -> String {
        text.lines().enumerate().map(|(index, line)| format!("{:>5}\t{line}", index + 1)).collect::<Vec<_>>().join("\n")
    }

    fn read(id: &str, input: serde_json::Value) -> crate::agent::dialect::ToolUse {
        crate::agent::dialect::ToolUse { id: id.into(), name: "read_file".into(), input }
    }

    #[test]
    fn a_changed_reread_becomes_a_small_diff() {
        let first: String = (1..=80).map(|n| format!("line {n}\n")).collect();
        let second = first.replace("line 40\n", "line forty\n");
        let mut prior = HashMap::new();
        let mut results = vec![("c1".to_string(), numbered(&first), false, None)];

        assert_eq!(apply(&mut prior, &[read("c1", json!({ "path": "a.txt" }))], &mut results), 0, "the first read is whole");

        let mut again = vec![("c2".to_string(), numbered(&second), false, None)];
        let saved = apply(&mut prior, &[read("c2", json!({ "path": "./a.txt" }))], &mut again);

        assert!(saved > 0);
        assert!(again[0].1.contains("-line 40") && again[0].1.contains("+line forty") && again[0].1.contains("@@ -38,5 +38,5 @@"), "{}", again[0].1);
    }

    #[test]
    fn ranges_rewrites_and_errors_are_left_alone() {
        let mut prior = HashMap::new();
        let mut results = vec![("c1".to_string(), numbered("a\nb\n"), false, None)];

        apply(&mut prior, &[read("c1", json!({ "path": "s.txt" }))], &mut results);

        let mut rewritten = vec![("c2".to_string(), numbered("x\ny\n"), false, None)];

        apply(&mut prior, &[read("c2", json!({ "path": "s.txt" }))], &mut rewritten);
        assert_eq!(rewritten[0].1, numbered("x\ny\n"), "a diff bigger than the file is not sent");

        let mut range = vec![("c3".to_string(), numbered("z\n"), false, None)];

        apply(&mut prior, &[read("c3", json!({ "path": "s.txt", "offset": 2 }))], &mut range);
        assert_eq!(range[0].1, numbered("z\n"));
    }
}
