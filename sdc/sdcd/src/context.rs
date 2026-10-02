//! The model's context - what a turn is sent, and how it stays inside what the model holds (0.13).
//!
//! Before 0.13 every turn was handed the **whole** chat: a CLI got the transcript pasted onto stdin, one
//! line per message and no word about who said what, and an API model got every earlier message. A long
//! chat therefore grew slower and dearer with every turn and then simply broke at the model's limit -
//! and the CLI, restarted each turn, had forgotten every file it had read and every command it had run.
//!
//! Three things replace that, the way Claude Code keeps its own sessions:
//!
//! * **resume** - a CLI that can continue its own conversation by id (Claude Code, Codex) is resumed,
//!   and is sent only what it has not seen: the person's new words, and any turn another engine took in
//!   between. The record says which turns the CLI's conversation holds, so a rewound turn (whose work is
//!   gone from disk) ends the resume rather than haunting it.
//! * **compact** - `/compact` asks the chat's own model for a summary of the conversation; later turns
//!   get that summary and the turns after it.
//! * **fit** - whatever is sent is fitted to the model's window: the newest turns verbatim, the older
//!   ones as a digest, and the turn is told the digest is one.

use serde_json::{json, Value};

use crate::engines::Message;
use crate::store::Store;

/// The share of the model's window the conversation may take; the rest is the system prompt, the
/// tools, the files the turn reads and the answer.
const HISTORY_SHARE: f64 = 0.45;

/// What a `/compact` turn asks the chat's model for. The answer is kept as the chat's summary.
pub const COMPACT_PROMPT: &str = "[SDC: compact this conversation - a note from SDC, not from the person]\n\
Write a summary of our whole conversation so far, detailed enough that you (or another model) could continue the work from it alone, without the transcript. Include, under short headings:\n\
- Goal: what the person wants overall, and the language they write in.\n\
- Decisions: what was agreed or chosen, and why.\n\
- Files: every file created or changed (its path) and what it now does.\n\
- Commands: how to build, test and run the project, as they were used.\n\
- State: what is done, what is still open, and any error not yet fixed.\n\
- Remember: anything the person asked to be remembered, their preferences.\n\
Be specific - names, paths, values, URLs - and leave out pleasantries. Write it in English, under 900 words. Do not call any tools and do not change any file.";

/// One earlier turn, as the conversation needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnRecord {
    pub id: String,
    pub engine: String,
    pub prompt: String,
    pub answer: String,
}

/// The turns of a chat that still count: not rewound, not `/compact` commands, not the one now starting.
pub fn live_turns(store: &Store, session_id: &str, exclude: &str) -> Vec<TurnRecord> {
    store
        .turns(session_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|turn| turn["state"] != "rewound" && turn["turnId"] != exclude)
        .map(|turn| TurnRecord {
            id: turn["turnId"].as_str().unwrap_or_default().to_string(),
            engine: turn["engine"].as_str().unwrap_or_default().to_string(),
            prompt: turn["prompt"].as_str().unwrap_or_default().to_string(),
            answer: turn["answer"].as_str().unwrap_or_default().to_string(),
        })
        .collect()
}

/// A rough token count: about four bytes a token for Latin text, and about one token per character of
/// Bengali, Devanagari and other scripts (their UTF-8 is three bytes a character, and tokenizers split
/// them finely). Rough on purpose - it decides when to fold, not what to bill.
pub fn tokens_of(text: &str) -> u64 {
    let (mut ascii, mut other) = (0u64, 0u64);

    for c in text.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }

    ascii / 4 + other + 1
}

/// How many tokens the model holds: the catalogue's `ctx` for the model, or what the CLI's models have.
pub fn window_tokens(engine: &str, provider: Option<&str>, model: &str) -> u64 {
    /* A local model runs with the context SDC asks Ollama for (`num_ctx`), never its card's 128K (0.16.1). */
    if engine == "ollama" || provider == Some("ollama") {
        return crate::engines::ollama::context_for(model);
    }

    let api_model = crate::engines::native_api::api_model(model);
    /* "default" is this build's word for "whatever the CLI picks": the engine's own window, not a row's. */
    let named = !model.trim().is_empty() && model != "default";

    for block in crate::providers::models::blocked() {
        if !named || provider.is_some_and(|provider| provider != block.id) {
            continue;
        }

        if let Some(ctx) = block
            .models
            .iter()
            .find(|row| row["id"].as_str().is_some_and(|id| id == model || id == api_model))
            .and_then(|row| row["ctx"].as_u64())
            .filter(|ctx| *ctx > 0)
        {
            return ctx;
        }
    }

    match engine {
        "claude_code" => 200_000,
        "codex" => 272_000,
        "gemini" => 1_000_000,
        "ollama" => 32_000,
        _ => 128_000,
    }
}

/// The budget the conversation may use for a model with `window` tokens.
pub fn history_budget(window: u64) -> u64 {
    (window as f64 * HISTORY_SHARE) as u64
}

/// The summary `/compact` left, and the turn it covers up to.
fn compacted(store: &Store, session_id: &str) -> Option<(String, String)> {
    let raw = store.setting(&format!("compact.{session_id}")).ok().flatten()?;
    let value: Value = serde_json::from_str(&raw).ok()?;

    Some((value["upto"].as_str()?.to_string(), value["summary"].as_str()?.to_string()))
        .filter(|(_, summary)| !summary.trim().is_empty())
}

/// Keeps `/compact`'s summary: every later turn starts from it.
pub fn save_compaction(store: &Store, session_id: &str, upto: &str, summary: &str) {
    let _ = store.set_setting(&format!("compact.{session_id}"), &json!({ "upto": upto, "summary": summary }).to_string());
}

/// What the conversation is sent as, and how it was fitted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fitted {
    pub messages: Vec<Message>,
    /// The tokens those messages come to (the estimate above).
    pub tokens: u64,
    /// Older turns were folded into a summary or a digest.
    pub compacted: bool,
}

/// The conversation for a turn, from a list of turns: a `/compact` summary first when there is one,
/// then the turns - the newest verbatim, the older folded into a digest when they do not fit `budget`.
pub fn fit(turns: &[TurnRecord], summary: Option<(&str, &str)>, budget: u64) -> Fitted {
    /* A summary counts only while the turn it ends at is still there (a rewind past it undoes it). */
    let (start, summary) = match summary {
        Some((upto, text)) => match turns.iter().position(|turn| turn.id == upto) {
            Some(index) => (index + 1, Some(text)),
            None => (0, None),
        },
        None => (0, None),
    };
    let turns = &turns[start..];
    let cost = |turn: &TurnRecord| tokens_of(&turn.prompt) + tokens_of(&turn.answer) + 8;
    let summary_tokens = summary.map(tokens_of).unwrap_or(0);
    let total: u64 = turns.iter().map(cost).sum::<u64>() + summary_tokens;

    let pair = |turn: &TurnRecord, out: &mut Vec<Message>| {
        if !turn.prompt.trim().is_empty() {
            out.push(Message::user(turn.prompt.clone()));
        }

        if !turn.answer.trim().is_empty() {
            out.push(Message::assistant(turn.answer.clone()));
        }
    };

    if total <= budget {
        let mut messages = Vec::new();

        if let Some(text) = summary {
            messages.push(Message::user(summary_block(text, None)));
            messages.push(Message::assistant("Understood - I have the earlier part of this chat in mind."));
        }

        for turn in turns {
            pair(turn, &mut messages);
        }

        return Fitted { messages, tokens: total, compacted: summary.is_some() };
    }

    /* Newest first, verbatim, while they fit in most of the budget. */
    let verbatim_budget = budget * 7 / 10;
    let mut kept = 0usize;
    let mut used = 0u64;

    for turn in turns.iter().rev() {
        let size = cost(turn);

        if used + size > verbatim_budget && kept > 0 {
            break;
        }

        /* A single turn larger than the whole budget is cut, not dropped: its end is what matters. */
        used += size.min(verbatim_budget);
        kept += 1;
    }

    let split = turns.len() - kept;
    let digest = digest(&turns[..split], budget.saturating_sub(used).saturating_sub(summary_tokens).max(budget / 10));
    let mut messages = vec![
        Message::user(summary_block(summary.unwrap_or(""), Some(&digest))),
        Message::assistant("Understood - I have the earlier part of this chat in mind."),
    ];

    for turn in &turns[split..] {
        let mut fitted = turn.clone();
        let limit = (verbatim_budget as usize).saturating_mul(3);

        if fitted.answer.len() > limit {
            fitted.answer = format!("[…the start of this answer was cut to fit the context…]\n{}", tail(&fitted.answer, limit));
        }

        pair(&fitted, &mut messages);
    }

    let tokens = messages.iter().map(|message| tokens_of(&message.text)).sum();

    Fitted { messages, tokens, compacted: true }
}

/// The block that stands in for the folded part of the chat.
fn summary_block(summary: &str, digest: Option<&str>) -> String {
    let mut text = String::from("[The earlier part of this chat, folded by SDC to fit the model's context]\n");

    if !summary.trim().is_empty() {
        text.push_str("Summary written when the chat was compacted:\n");
        text.push_str(summary.trim());
        text.push('\n');
    }

    if let Some(digest) = digest.filter(|digest| !digest.trim().is_empty()) {
        text.push_str("\nOlder turns, in short (oldest first):\n");
        text.push_str(digest);
    }

    text.push_str("\n[End of the folded part - the turns after it follow in full]");
    text
}

/// One line per older turn - what was asked, and how the answer began - newest kept when space runs out.
fn digest(turns: &[TurnRecord], budget: u64) -> String {
    let mut lines: Vec<String> = turns
        .iter()
        .map(|turn| {
            let tools = turn.answer.lines().filter(|line| line.starts_with("- ") && turn.answer.contains("[Tool calls in this turn")).count();
            let answer = turn.answer.split("\n\n[Tool calls in this turn").next().unwrap_or_default();

            format!(
                "- Person: {} → {}{}",
                head(&turn.prompt, 240),
                head(answer, 360),
                if tools > 0 { format!(" ({tools} tool calls)") } else { String::new() }
            )
        })
        .collect();

    while lines.len() > 1 && lines.iter().map(|line| tokens_of(line)).sum::<u64>() > budget {
        lines.remove(0);
    }

    lines.join("\n")
}

fn head(text: &str, chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");

    if flat.chars().count() <= chars {
        flat
    } else {
        format!("{}…", flat.chars().take(chars).collect::<String>())
    }
}

fn tail(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        return text;
    }

    let mut start = text.len() - bytes;

    while !text.is_char_boundary(start) {
        start += 1;
    }

    &text[start..]
}

/// The conversation for a turn of `session_id`, fitted to `budget` tokens.
pub fn history(store: &Store, session_id: &str, exclude: &str, budget: u64) -> Fitted {
    let turns = live_turns(store, session_id, exclude);
    let summary = compacted(store, session_id);

    fit(&turns, summary.as_ref().map(|(upto, text)| (upto.as_str(), text.as_str())), budget)
}

// ---------------------------------------------------------------------------------------------------
// Resume
// ---------------------------------------------------------------------------------------------------

/// Which conversation of a CLI a chat continues, and what it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct ResumeRecord {
    pub id: String,
    /// `local` or the host id - a conversation lives on the machine the CLI ran on.
    pub place: String,
    pub root: String,
    /// The chat's turns the conversation has seen, oldest first.
    pub turns: Vec<String>,
}

fn resume_key(session_id: &str, engine: &str) -> String {
    format!("resume.{session_id}.{engine}")
}

pub fn load_resume(store: &Store, session_id: &str, engine: &str) -> Option<ResumeRecord> {
    let raw = store.setting(&resume_key(session_id, engine)).ok().flatten()?;
    let value: Value = serde_json::from_str(&raw).ok()?;

    Some(ResumeRecord {
        id: value["id"].as_str()?.to_string(),
        place: value["place"].as_str().unwrap_or_default().to_string(),
        root: value["root"].as_str().unwrap_or_default().to_string(),
        turns: value["turns"].as_array()?.iter().filter_map(|id| id.as_str().map(str::to_string)).collect(),
    })
}

pub fn save_resume(store: &Store, session_id: &str, engine: &str, record: &ResumeRecord) {
    let _ = store.set_setting(
        &resume_key(session_id, engine),
        &json!({ "id": record.id, "place": record.place, "root": record.root, "turns": record.turns }).to_string(),
    );
}

/// Ends every CLI conversation of a chat - after `/compact`, `/clear`, or a rewind.
pub fn forget_resume(store: &Store, session_id: &str) {
    for engine in ["claude_code", "codex", "gemini"] {
        let _ = store.set_setting(&resume_key(session_id, engine), "");
    }
}

/// Whether a turn can continue `record`: same machine, same folder, and every turn it holds is still a
/// live turn of the chat. `Some(missed)` is the turns it has not seen (another engine's), oldest first.
pub fn resumable<'a>(record: &ResumeRecord, place: &str, root: &str, turns: &'a [TurnRecord]) -> Option<Vec<&'a TurnRecord>> {
    if record.place != place || record.root != root || record.id.is_empty() {
        return None;
    }

    if !record.turns.iter().all(|id| turns.iter().any(|turn| &turn.id == id)) {
        return None;
    }

    Some(turns.iter().filter(|turn| !record.turns.contains(&turn.id)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(id: &str, prompt: &str, answer: &str) -> TurnRecord {
        TurnRecord { id: id.into(), engine: "claude_code".into(), prompt: prompt.into(), answer: answer.into() }
    }

    #[test]
    fn a_short_chat_is_sent_whole_with_its_roles() {
        let turns = vec![turn("t1", "make a page", "Made index.html"), turn("t2", "add a form", "Added it")];
        let fitted = fit(&turns, None, 10_000);

        assert!(!fitted.compacted);
        assert_eq!(fitted.messages.len(), 4);
        assert_eq!(fitted.messages[0], Message::user("make a page"));
        assert_eq!(fitted.messages[3], Message::assistant("Added it"));
    }

    #[test]
    fn a_long_chat_keeps_its_newest_turns_and_folds_the_rest() {
        let long = "x".repeat(4_000);
        let turns: Vec<TurnRecord> = (1..=40).map(|n| turn(&format!("t{n}"), &format!("request {n}"), &long)).collect();
        let fitted = fit(&turns, None, 8_000);

        assert!(fitted.compacted);
        assert!(fitted.messages[0].text.contains("folded by SDC"));
        assert!(fitted.messages[0].text.contains("request 2"), "the digest names older requests");
        assert_eq!(fitted.messages.last().unwrap().text, long, "the newest answer is verbatim");
        assert!(fitted.tokens <= 8_000 + 1_000, "{}", fitted.tokens);
    }

    #[test]
    fn a_compaction_summary_replaces_the_turns_it_covers() {
        let turns = vec![turn("t1", "old", "old answer"), turn("t2", "/compact", "SUMMARY"), turn("t3", "new", "new answer")];
        let fitted = fit(&turns, Some(("t2", "SUMMARY")), 10_000);

        assert!(fitted.messages[0].text.contains("SUMMARY"));
        assert!(!fitted.messages.iter().any(|message| message.text == "old"));
        assert_eq!(fitted.messages[2], Message::user("new"));

        /* Rewound past the compaction: the summary no longer holds. */
        let fitted = fit(&turns[..1], Some(("t2", "SUMMARY")), 10_000);

        assert_eq!(fitted.messages[0], Message::user("old"));
    }

    #[test]
    fn a_resume_holds_only_while_its_turns_are_live() {
        let record = ResumeRecord { id: "abc".into(), place: "local".into(), root: "/p".into(), turns: vec!["t1".into(), "t2".into()] };
        let turns = vec![turn("t1", "a", "b"), turn("t2", "c", "d"), turn("t3", "e", "f")];

        let missed = resumable(&record, "local", "/p", &turns).unwrap();
        assert_eq!(missed.iter().map(|turn| turn.id.as_str()).collect::<Vec<_>>(), ["t3"]);

        assert!(resumable(&record, "h1", "/p", &turns).is_none(), "another machine");
        assert!(resumable(&record, "local", "/q", &turns).is_none(), "another folder");
        assert!(resumable(&record, "local", "/p", &turns[1..]).is_none(), "t1 was rewound");
    }

    #[test]
    fn bengali_counts_heavier_than_english() {
        assert!(tokens_of("আমার সাইট কেন লোড হচ্ছে না") > tokens_of("why is my site not loading"));
    }

    #[test]
    fn the_window_comes_from_the_catalogue_or_the_engine() {
        /* 0.16.1: a local model's window is the context SDC asks Ollama for - its card says 128K, the
           ceiling (16 384 unless Settings says more) is what the graphics card is asked to hold. */
        assert_eq!(window_tokens("ollama", Some("ollama"), "llama3.2:3b"), crate::engines::ollama::context_cap().min(128_000));
        assert_eq!(window_tokens("native_api", Some("ollama"), "qwen3.5:9b"), crate::engines::ollama::context_cap().min(16_384));
        assert_eq!(window_tokens("claude_code", None, "default"), 200_000);
    }
}
