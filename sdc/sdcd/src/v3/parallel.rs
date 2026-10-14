//! PAR: several agents at once.
//!
//! Each agent works in its own git worktree, so two of them never reach for the same file, and a small board
//! (`.sdc/agents/`, one card per agent) is what the person watches: what each one was asked, where it works,
//! and how it ended.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How an agent ended up.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// It is working now.
    Running,
    /// It finished and the person has not read it yet.
    Done,
    /// It stopped on an error.
    Failed,
    /// The person stopped it.
    Stopped,
}

fn running() -> State {
    State::Running
}

/// One agent on the board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    /// What the person asked for.
    pub request: String,
    /// The worktree it works in, relative to the project.
    pub worktree: String,
    /// When it started (unix seconds).
    pub started_at: i64,
    #[serde(default = "running")]
    pub state: State,
    /// The last thing worth saying about it.
    #[serde(default)]
    pub note: String,
    /// The chat the agent's turn runs in, and that turn - what `agents.stop` stops. Absent on a card written
    /// before 2.0, which is why both are optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

/// Where the cards live.
pub fn dir(root: &Path) -> PathBuf {
    root.join(".sdc/agents")
}

fn file_of(root: &Path, id: &str) -> PathBuf {
    dir(root).join(format!("{id}.json"))
}

/// A name for a new agent that nobody is using: the request's first words, or `agent`.
pub fn id_for(root: &Path, request: &str, now: i64) -> String {
    let words: Vec<String> = request
        .split_whitespace()
        .filter(|word| word.chars().any(|letter| letter.is_ascii_alphanumeric()))
        .take(3)
        .map(|word| word.chars().filter(|letter| letter.is_ascii_alphanumeric()).collect::<String>().to_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    let base = if words.is_empty() { "agent".to_string() } else { words.join("-") };
    let mut id = base.clone();
    let mut counter = 2;

    while file_of(root, &id).exists() {
        id = format!("{base}-{counter}-{now}");
        counter += 1;
    }

    id
}

/// Where an agent works: a checkout of the project's own repository, kept outside the project so the person's
/// folder never collects a pile of worktrees (the same place the session checkpoints live).
pub fn worktree_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    let digest = crate::fs::hash(root.display().to_string().as_bytes());
    let base = crate::paths::shadow_git_dir().map_err(|error| error.to_string())?.join(format!("{}-agents", &digest[..16]));

    Ok(base.join(id))
}

/// Start an agent: its own worktree, and a card on the board.
pub fn start(root: &Path, request: &str, id: &str, now: i64) -> Result<Agent, String> {
    if !root.join(".git").exists() {
        return Err(format!("{} keeps its files outside git, so two agents cannot be kept apart - an agent needs a repository it can branch from", root.display()));
    }

    let path = worktree_path(root, id)?;
    let text = path.display().to_string();

    /* Detached: two agents never fight over one branch, and the person's own branch is never moved. */
    let added = std::process::Command::new("git").arg("-C").arg(root).args(["worktree", "add", "--detach"]).arg(&text).arg("HEAD").output().map_err(|error| format!("git: {error}"))?;

    if !added.status.success() {
        return Err(format!("git could not make a worktree for `{id}`: {}", String::from_utf8_lossy(&added.stderr).trim()));
    }

    let agent = Agent { id: id.to_string(), request: request.to_string(), worktree: text, started_at: now, state: State::Running, note: String::new(), session_id: None, turn_id: None };

    save(root, &agent)?;

    Ok(agent)
}

/// Write one card.
pub fn save(root: &Path, agent: &Agent) -> Result<(), String> {
    let path = file_of(root, &agent.id);

    std::fs::create_dir_all(dir(root)).map_err(|error| format!("{}: {error}", dir(root).display()))?;

    let text = serde_json::to_string_pretty(agent).map_err(|error| error.to_string())?;

    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// One card, if it is there.
pub fn load(root: &Path, id: &str) -> Option<Agent> {
    std::fs::read_to_string(file_of(root, id)).ok().and_then(|text| serde_json::from_str(&text).ok())
}

/// The board: every agent, newest first.
pub fn board(root: &Path) -> Vec<Agent> {
    let Ok(entries) = std::fs::read_dir(dir(root)) else {
        return Vec::new();
    };
    let mut agents: Vec<Agent> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok().and_then(|text| serde_json::from_str::<Agent>(&text).ok()))
        .collect();

    agents.sort_by_key(|agent| std::cmp::Reverse(agent.started_at));

    agents
}

/// Say how an agent ended.
pub fn finish(root: &Path, id: &str, state: State, note: &str) -> Result<Agent, String> {
    let mut agent = load(root, id).ok_or_else(|| format!("no agent `{id}` on the board"))?;

    agent.state = state;
    agent.note = note.to_string();
    save(root, &agent)?;

    Ok(agent)
}

/// Take a card off the board (its worktree is left alone - the person's work is never deleted by this).
pub fn forget(root: &Path, id: &str) -> Result<(), String> {
    let path = file_of(root, id);

    if !path.exists() {
        return Err(format!("no agent `{id}` on the board"));
    }

    std::fs::remove_file(&path).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-par-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::process::Command::new("git").args(["init", "-q"]).current_dir(&root).status().unwrap();
        std::fs::write(root.join("readme.md"), "hello\n").unwrap();
        std::process::Command::new("git").args(["add", "-A"]).current_dir(&root).status().unwrap();
        std::process::Command::new("git").args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "first"]).current_dir(&root).status().unwrap();
        std::fs::create_dir_all(root.join(".sdc")).unwrap();

        root
    }

    #[test]
    fn an_agent_gets_its_own_worktree_and_the_board_shows_it() {
        let root = project("board");
        let id = id_for(&root, "Fix the failing inventory tests please", 1_760_000_000);

        assert_eq!(id, "fix-the-failing");

        let agent = start(&root, "Fix the failing inventory tests please", &id, 1_760_000_000).unwrap();

        assert!(PathBuf::from(&agent.worktree).join("readme.md").exists(), "the worktree is a real checkout: {}", agent.worktree);
        assert_eq!(agent.state, State::Running);
        assert_eq!(load(&root, &id).unwrap().request, agent.request);

        /* A second agent does not land on the first one's name, nor in its worktree. */
        let second = id_for(&root, "Fix the failing inventory tests please", 1_760_000_000);

        assert_ne!(second, id, "the name was free the first time and is not now");

        let other = start(&root, "Fix the failing inventory tests please", &second, 1_760_000_100).unwrap();

        assert_ne!(other.worktree, agent.worktree, "two agents never share a directory");

        finish(&root, &other.id, State::Done, "all green").unwrap();

        let cards = board(&root);

        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].id, second, "the newest is first");
        assert_eq!(cards[0].state, State::Done);
        assert_eq!(cards[0].note, "all green");
        assert_eq!(cards[1].state, State::Running);

        forget(&root, &second).unwrap();

        assert_eq!(board(&root).len(), 1);
        assert!(PathBuf::from(&other.worktree).exists(), "taking a card off the board does not throw the work away");
        assert!(finish(&root, "nobody", State::Failed, "x").is_err(), "an agent that is not on the board is said so");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_request_with_no_words_still_gets_a_name() {
        let root = project("noname");

        std::fs::create_dir_all(dir(&root)).unwrap();

        assert_eq!(id_for(&root, "!!! ???", 5), "agent");

        let _ = std::fs::remove_dir_all(&root);
    }
}
