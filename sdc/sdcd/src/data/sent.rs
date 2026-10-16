//! `.sdc/audit/sent.jsonl` (P0-5): what left this computer, and what was taken out before it did.
//!
//! One JSON object per line, one line per thing that was sent to a model. Written **append-only**: the file is
//! opened for append, every line ends in `\n` and is flushed before the call it describes is allowed to happen. A
//! later line never rewrites an earlier one, so the file can only grow - and a log that can only grow is a log that
//! cannot be used to hide what it already recorded.
//!
//! # Why this exists
//!
//! The question a person asks about an agent is "what did it send, and to whom?" - and the honest answer needs to
//! include the parts that were *removed*, because "we redacted your key" is only worth believing if the record says
//! which rule fired. So each line carries the redaction summary and a hash of the capsule that was sent, which is
//! enough to tell two sends apart and to prove a send happened, without keeping a second copy of what was sent.
//!
//! # What is deliberately not here
//!
//! The text itself. The audit line records the *shape* and the *fingerprint* of a send - model, token count, rules
//! that fired, hash - because a log that quoted the payload would be a second place for a secret to leak from. The
//! preview a person reads in the window is a different thing and lives in the session, not here.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One thing that was sent to a model.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Sent {
    /// When, in RFC 3339 (`2026-10-17T09:15:00Z`), which is what the rest of the daemon writes.
    pub at: String,
    /// The model the text went to, as `provider:model`.
    pub model: String,
    /// Which step sent it (`plan`, `edit`, …), so a line can be read next to a transcript.
    pub step: String,
    /// The turn and session this belongs to, so two lines can be read as one turn.
    pub turn: String,
    pub session: String,
    /// Tokens in the text that was sent, as counted before sending.
    pub tokens: usize,
    /// A short word for each redaction rule that fired, in the order it fired: `api-key`, `email`, `vault`. Empty
    /// when nothing was removed, which is a fact worth recording too - it is the difference between "nothing
    /// sensitive was there" and "we did not look".
    pub redactions: Vec<String>,
    /// SHA-256 of the exact text that was sent. A fingerprint, not the text: enough to tell whether two sends were
    /// the same without a second copy of the content.
    pub hash: String,
    /// Whether the send left this computer. `false` for a local model, and the field exists so that "did anything
    /// leave?" is a query rather than an inference from the model's name.
    pub left_the_machine: bool,
}

/// The fingerprint of a capsule: SHA-256 of the bytes that were sent.
///
/// The plan asks the cache to be keyed by BLAKE3; BLAKE3 is not in this build (see `data`), so SHA-256 is used -
/// the same digest `v3::cas` already uses for content addressing.
pub fn hash(capsule: &str) -> String {
    hex::encode(Sha256::digest(capsule.as_bytes()))
}

pub fn dir(root: &Path) -> PathBuf {
    super::audit(root)
}

pub fn path(root: &Path) -> PathBuf {
    dir(root).join("sent.jsonl")
}

/// Append one line, and do not return success until it is on the disk.
///
/// `flush` after the write is not enough on its own for the same reason the journal needed `synchronous = FULL`
/// (P0-4): the bytes are in the operating system's cache until the file is synced. A send whose record is still in
/// that cache when the power goes is a send that happened and was never recorded, which is the one failure an audit
/// trail may not have. So the handle is opened, written, flushed **and** synced before this returns.
pub fn append(root: &Path, entry: &Sent) -> Result<(), String> {
    let file = path(root);

    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }

    /* Append, and create only if it is not there yet: `create(true)` without `truncate` never shortens an existing
       log, which is the property the tests pin. */
    let mut handle = std::fs::OpenOptions::new().create(true).append(true).open(&file).map_err(|error| format!("{}: {error}", file.display()))?;

    let line = serde_json::to_string(entry).map_err(|error| error.to_string())?;

    handle.write_all(format!("{line}\n").as_bytes()).map_err(|error| format!("{}: {error}", file.display()))?;
    handle.flush().map_err(|error| format!("{}: {error}", file.display()))?;
    handle.sync_all().map_err(|error| format!("{}: {error}", file.display()))?;

    Ok(())
}

/// Every line that reads back, oldest first. A line that does not parse is counted as `damaged` and skipped, for the
/// same reason the journal does it: a log with one torn line is still a log, and refusing to read it would destroy
/// exactly the evidence it exists to keep.
pub fn read(root: &Path) -> (Vec<Sent>, usize) {
    let Ok(text) = std::fs::read_to_string(path(root)) else {
        return (Vec::new(), 0);
    };

    let mut sent = Vec::new();
    let mut damaged = 0;

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<Sent>(line) {
            Ok(entry) => sent.push(entry),
            Err(_) => damaged += 1,
        }
    }

    (sent, damaged)
}

/// Just the lines that left this computer - the query the field exists for.
pub fn left_the_machine(root: &Path) -> Vec<Sent> {
    read(root).0.into_iter().filter(|entry| entry.left_the_machine).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(model: &str, hash: &str, left: bool) -> Sent {
        Sent {
            at: "2026-10-17T09:15:00Z".into(),
            model: model.into(),
            step: "edit".into(),
            turn: "t-1".into(),
            session: "s-1".into(),
            tokens: 1200,
            redactions: vec!["api-key".into()],
            hash: hash.into(),
            left_the_machine: left,
        }
    }

    #[test]
    fn a_line_records_the_five_things_the_plan_names() {
        let dir = tempfile::tempdir().unwrap();

        append(dir.path(), &entry("deepseek:deepseek-chat", &hash("the capsule"), true)).unwrap();

        let (sent, damaged) = read(dir.path());

        assert_eq!(damaged, 0);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].model, "deepseek:deepseek-chat");
        assert_eq!(sent[0].tokens, 1200);
        assert_eq!(sent[0].redactions, ["api-key"]);
        assert_eq!(sent[0].hash, hash("the capsule"));
        assert!(sent[0].left_the_machine);
    }

    #[test]
    fn the_log_is_never_truncated() {
        let dir = tempfile::tempdir().unwrap();

        for index in 0..5 {
            append(dir.path(), &entry(&format!("ollama:qwen3:{index}"), &hash(&index.to_string()), false)).unwrap();
        }

        /* A line written by hand, the way an older build's line would be: it must still be there afterwards. */
        let before = std::fs::read_to_string(path(dir.path())).unwrap();

        append(dir.path(), &entry("deepseek:deepseek-chat", &hash("new"), true)).unwrap();

        let after = std::fs::read_to_string(path(dir.path())).unwrap();

        assert!(after.starts_with(&before), "a later line never rewrites an earlier one");
        assert_eq!(after.lines().count(), 6);
        assert_eq!(read(dir.path()).0.len(), 6);
    }

    #[test]
    fn a_torn_line_is_counted_and_the_rest_still_reads() {
        let dir = tempfile::tempdir().unwrap();

        append(dir.path(), &entry("ollama:qwen3:8b", &hash("one"), false)).unwrap();

        /* A kill in the middle of the last write leaves a line that does not parse. */
        let mut handle = std::fs::OpenOptions::new().append(true).open(path(dir.path())).unwrap();
        handle.write_all(b"{\"at\": \"2026-10-17T09:1").unwrap();
        drop(handle);

        let (sent, damaged) = read(dir.path());

        assert_eq!(sent.len(), 1, "the good line before the tear is still evidence");
        assert_eq!(damaged, 1, "and the tear is counted rather than hidden");
    }

    #[test]
    fn the_hash_is_a_fingerprint_of_the_capsule_and_not_the_capsule() {
        let digest = hash("secret text");

        assert_eq!(digest.len(), 64);
        assert_eq!(digest, hash("secret text"), "the same text is the same fingerprint");
        assert_ne!(digest, hash("secret text "), "and a different one is a different fingerprint");
        assert!(!digest.contains("secret"), "the text itself cannot come back out of it");
    }

    #[test]
    fn only_the_sends_that_left_the_machine_are_returned() {
        let dir = tempfile::tempdir().unwrap();

        append(dir.path(), &entry("ollama:qwen3:8b", &hash("local"), false)).unwrap();
        append(dir.path(), &entry("deepseek:deepseek-chat", &hash("remote"), true)).unwrap();

        let left = left_the_machine(dir.path());

        assert_eq!(left.len(), 1);
        assert_eq!(left[0].model, "deepseek:deepseek-chat");
    }
}
