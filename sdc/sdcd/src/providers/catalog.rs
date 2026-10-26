//! **One row per model, with a name a person can read** - the canonical catalogue (3.2).
//!
//! Every source of model rows (a provider's `/v1/models`, a CLI's own program files, the cache, the bundle) answers
//! with *ids*, and ids are not a menu. The same model arrives under several of them:
//!
//! ```text
//! claude-sonnet-5-5   claude-sonnet-5-5-20260918   sonnet            -> Claude Sonnet 5.5
//! gemini-3.1-pro      gemini-3.1-pro-preview                        -> Gemini 3.1 Pro
//! claude-opus-4       claude-opus-4-0              claude-opus-4-v1  -> Claude Opus 4
//! ```
//!
//! Until 3.2 each of those was its own line in the model menu, so the Claude card offered `Claude Sonnet 5.5 · plan`
//! and `Claude Sonnet · plan` as if they were two models (the report: *"never show both Sonnet and Sonnet 5.5"*), and
//! a live row for `claude-sonnet-5-5` was spelled **"Claude Sonnet 5 5"** - the old spelling rule only knew the
//! version pairs somebody had written into it.
//!
//! This module is the fix, and it is three pure functions a test can read:
//!
//! * [`key`] - the **canonical key** of an id: the id with the parts that do not name a different model taken off
//!   (a snapshot date, `-preview`, `-latest`, `-v1`, `-001`, a trailing `-0` minor). Two ids with the same key are
//!   the same model;
//! * [`collapse`] - rows grouped by that key: one row per model, the plainest id as the one to send, the others kept
//!   as `aliases` so nothing is hidden and a stored choice still resolves. A bare line word (`sonnet`, `opus`) is
//!   folded into the newest model of its line rather than standing as a model of its own;
//! * [`name`] - the id as a name: `claude-sonnet-5-5` -> `Claude Sonnet 5.5`, `glm-4.6` -> `GLM-4.6`. Generic, so a
//!   model released after this build is spelled right too.
//!
//! Two filters go with them, and they are applied **only to ids scanned out of a CLI's own files** ([`plausible`]) or
//! to any source when the row would not be a chat model at all ([`chatty`]): a scan finds the real ids and also finds
//! a test fixture and a documentation filename, and a model picker for coding has no use for a text-to-speech or an
//! image model.

use serde_json::{json, Value};

/// Words that mark a *release channel* rather than a different model.
const CHANNELS: &[&str] = &["preview", "latest", "exp", "experimental", "stable", "ga", "beta", "rc", "default"];

/// The model lines whose bare name a CLI accepts as "the newest one of this line".
///
/// `claude --model sonnet` and `claude --model claude-sonnet-5-5` run the same model, so the menu must offer it once.
const LINES: &[&str] = &["opus", "sonnet", "haiku", "fable", "mythos", "pro", "flash", "max", "mini", "plus", "turbo", "air", "lite"];

/// Kinds of model a coding chat cannot run, by the word that names them in an id.
const NOT_CHAT: &[&str] = &[
    "tts", "stt", "transcribe", "transcription", "whisper", "audio", "speech", "voice", "music", "lyria", "realtime",
    "live", "embed", "embedding", "embeddings", "rerank", "reranker", "moderation", "guard", "image", "imagen",
    "banana", "veo", "video", "dall", "ocr", "base", "edit", "upscale", "vision",
    /* Not chat models either, and both turned up in the installed Gemini CLI's own bundle: a computer-use agent and
       an internal tool-calling variant. */
    "computer", "customtools",
];

/// File extensions an id is never allowed to end in - a scan of a CLI's own files finds `gemini-3.md` too.
const EXTENSIONS: &[&str] = &[".js", ".ts", ".mjs", ".cjs", ".md", ".json", ".txt", ".py", ".go", ".rs", ".html", ".map"];

/// Is every byte of `part` an ASCII digit? (`"4"`, `"20250805"` - but not `"4b"`.)
fn digits(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
}

/// Could this number be a date rather than a version?
///
/// A version is small; a date is a year or a month-day, and the difference is what keeps `gemini-9001-super-duper` (a
/// fixture in the Gemini CLI's own files) out of the menu while `kimi-k2-0905-preview` still folds into `kimi-k2`:
///
/// * eight or six digits beginning `20` - `20250805`, `202508`;
/// * four digits beginning `20` - a year;
/// * four digits whose first two are a month - `0905`, `1231`.
fn date_like(part: &str) -> bool {
    if !digits(part) {
        return false;
    }

    let month = || part[..2].parse::<u32>().is_ok_and(|month| (1..=12).contains(&month));

    match part.len() {
        8 | 6 => part.starts_with("20"),
        4 => part.starts_with("20") || month(),
        _ => false,
    }
}

/// A date at the end of an id, as the number of dash-separated parts it takes up.
///
/// The four shapes the providers actually ship: `-20250805`, `-2026-01-23`, `-12-2025` and `-0905`.
fn trailing_date(parts: &[&str]) -> usize {
    let last = |back: usize| parts.len().checked_sub(back).map(|at| parts[at]);
    let is = |back: usize, len: usize| last(back).is_some_and(|part| digits(part) && part.len() == len);
    let dated = |back: usize| last(back).is_some_and(date_like);

    if is(1, 8) && dated(1) {
        return 1;
    }

    if is(3, 4) && dated(3) && is(2, 2) && is(1, 2) {
        return 3;
    }

    if is(2, 2) && is(1, 4) && dated(1) {
        return 2;
    }

    if is(1, 4) && dated(1) {
        return 1;
    }

    0
}

/// The **canonical key** of a model id: what is left when the parts that do not name a different model are taken off.
///
/// What is taken off, repeatedly until nothing more is:
///
/// * a release channel (`-preview`, `-latest`, `-exp`, `-beta`, ...);
/// * a snapshot date in any of the four shapes providers ship ([`trailing_date`]);
/// * a build counter (`-v1`, `-001`);
/// * a trailing zero minor version, so `claude-opus-4-0` and `claude-opus-4` are one model.
///
/// What is **never** taken off: a size or a variant (`-mini`, `-flash`, `-coder`, `-turbo`, `-lite`, `-us`), because
/// those are different models with different prices. An OpenRouter-style `vendor/` prefix and an Ollama `:tag` stay
/// too - `qwen2.5-coder:7b` and `qwen2.5-coder:32b` are not the same weights.
pub fn key(id: &str) -> String {
    let lowered = id.trim().to_ascii_lowercase();
    let (prefix, rest) = match lowered.rfind('/') {
        Some(at) => (lowered[..=at].to_string(), lowered[at + 1..].to_string()),
        None => (String::new(), lowered.clone()),
    };
    let (body, tag) = match rest.split_once(':') {
        Some((body, tag)) => (body.to_string(), format!(":{tag}")),
        None => (rest, String::new()),
    };
    let mut parts: Vec<&str> = body.split('-').filter(|part| !part.is_empty()).collect();

    loop {
        let before = parts.len();

        if let Some(last) = parts.last() {
            let channel = CHANNELS.contains(last);
            let counter = digits(last) && last.len() == 3;
            let build = last.len() >= 2 && last.starts_with('v') && digits(&last[1..]);

            if (channel || counter || build) && parts.len() > 1 {
                parts.pop();
            }
        }

        let date = trailing_date(&parts);

        if date > 0 && parts.len() > date {
            parts.truncate(parts.len() - date);
        }

        /* `claude-opus-4-0` is `claude-opus-4`: a zero minor is the first release of the major, not another model. */
        if parts.len() > 2 && parts[parts.len() - 1] == "0" && digits(parts[parts.len() - 2]) {
            parts.pop();
        }

        if parts.len() == before {
            break;
        }
    }

    format!("{prefix}{}{tag}", parts.join("-"))
}

/// The version in a key, as `(major, minor)`, for "which of these is the newest".
///
/// It reads both spellings providers use - `claude-sonnet-5-5` (dashes) and `gemini-3.1-pro` (a point) - and a version
/// glued to the vendor (`qwen3.8-max`). `None` when the id carries no version at all (`deepseek-chat`, `sonnet`).
pub fn version(key: &str) -> Option<(u32, u32)> {
    let body = key.rsplit('/').next().unwrap_or(key).split(':').next().unwrap_or(key);
    let parts: Vec<&str> = body.split('-').collect();

    for (index, part) in parts.iter().enumerate() {
        /* `3.1`, or a vendor with its version glued on (`qwen3.8`, `glm`+`4.6` is two parts and caught below). */
        if let Some((head, tail)) = part.split_once('.') {
            let major = head.trim_start_matches(|character: char| character.is_ascii_alphabetic());

            if digits(major) && digits(tail) {
                return Some((major.parse().unwrap_or(0), tail.parse().unwrap_or(0)));
            }
        }

        if digits(part) {
            let minor = parts.get(index + 1).filter(|next| digits(next) && next.len() <= 2);

            return Some((part.parse().unwrap_or(0), minor.map_or(0, |next| next.parse().unwrap_or(0))));
        }

        /* `qwen3` - a vendor word with the version on its tail. */
        let tail = part.trim_start_matches(|character: char| character.is_ascii_alphabetic());

        if digits(tail) && tail.len() <= 2 && tail.len() < part.len() {
            return Some((tail.parse().unwrap_or(0), 0));
        }
    }

    None
}

/// The model's **line**: the vendor word and the size word, for grouping a menu (`claude sonnet`, `gemini pro`).
///
/// Anthropic's own Models API carries a `line` field and its documentation says to read it rather than infer it; when a
/// row brings one, [`collapse`] keeps it and this is only the fallback for every provider that sends none.
pub fn line(key: &str) -> String {
    let body = key.rsplit('/').next().unwrap_or(key).split(':').next().unwrap_or(key);
    let parts: Vec<&str> = body.split('-').filter(|part| !part.is_empty()).collect();
    let vendor = parts
        .first()
        .map(|part| part.trim_end_matches(|character: char| character.is_ascii_digit() || character == '.'))
        .unwrap_or_default();
    let size = parts.iter().find(|part| LINES.contains(&part.to_ascii_lowercase().as_str()));

    match size {
        Some(size) if !vendor.is_empty() && vendor != *size => format!("{vendor} {size}"),
        Some(size) => (*size).to_string(),
        None => vendor.to_string(),
    }
}

/// Could a coding chat run this model? `false` for the kinds that are not text in, text out.
///
/// A provider's list holds its whole catalogue - text-to-speech, image, embedding, live-audio and base (pre-instruct)
/// models included - and none of those can take a turn in SDC. They are left out of the menu rather than offered and
/// then failing on the first message.
pub fn chatty(id: &str) -> bool {
    let body = id.to_ascii_lowercase();
    let body = body.rsplit('/').next().unwrap_or(&body);

    !body
        .split(['-', '.', ':', '_'])
        .any(|part| NOT_CHAT.contains(&part))
}

/// Could this string be a model id at all? The filter for ids **scanned out of a CLI's own program files**.
///
/// A scan is the only way to learn which models the installed CLI accepts (none of the three has a `models list`
/// command), and it reads every string that looks like an id - which includes the CLI's own test fixtures
/// (`gemini-9001-super-duper`), its documentation filenames (`gemini-3.md`) and the odd sentence turned into a slug
/// (`gemini-3-flash-is-now-available-in-gemini-cli`). So:
///
/// * an id ends in no file extension;
/// * no part of it is a number above 100 - a version, a size and a date are the only numbers in a model id, and a
///   major version of 9001 is somebody's joke;
/// * at most seven dash-separated parts, which every real id of every provider in the catalogue fits inside;
/// * nothing but `[a-z0-9.:/_-]`, and no empty part.
///
/// It is deliberately **not** applied to a row a provider's own HTTP endpoint answered with: that list is the provider
/// speaking for itself, and a filter over it could only hide a real model.
pub fn plausible(id: &str) -> bool {
    let body = id.trim();

    if body.len() < 3 || body.len() > 60 {
        return false;
    }

    if EXTENSIONS.iter().any(|extension| body.to_ascii_lowercase().ends_with(extension)) {
        return false;
    }

    if !body.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".:/_-".contains(&byte)) {
        return false;
    }

    let parts: Vec<&str> = body.split('-').collect();

    if parts.len() > 7 || parts.iter().any(|part| part.is_empty()) {
        return false;
    }

    body.split(['-', '.', ':', '/', '_'])
        .filter(|part| digits(part))
        /* Either it is a date, or it is a version - and a version is small. `9001` is neither. */
        .all(|part| date_like(part) || part.parse::<u32>().unwrap_or(u32::MAX) <= 100)
}

/// A model id as a name a person can read.
///
/// Generic on purpose: a model released after this build gets a right answer rather than its raw id. The rules are the
/// ones a reader expects - known words keep their own spelling (`GPT`, `GLM`, `DeepSeek`), two numbers side by side are
/// one version (`5-5` is 5.5), and a vendor written in capitals takes a dash before its version (`GLM-4.6`).
pub fn name(id: &str) -> String {
    let body = id.trim();
    let (vendor, body) = match body.rsplit_once('/') {
        Some((vendor, rest)) => (Some(vendor.to_string()), rest.to_string()),
        None => (None, body.to_string()),
    };
    let (body, tag) = match body.split_once(':') {
        Some((body, tag)) => (body.to_string(), Some(tag.to_string())),
        None => (body, None),
    };
    let mut words: Vec<String> = Vec::new();

    for part in body.split(['-', '_']).filter(|part| !part.is_empty()) {
        /* Two numbers in a row are one version: `claude-sonnet-5-5` is Sonnet 5.5, not Sonnet 5 5. */
        if digits(part) && words.last().is_some_and(|last| version_word(last)) {
            let joined = format!("{}.{part}", words.pop().unwrap_or_default());

            words.push(joined);

            continue;
        }

        words.push(word(part));
    }

    let mut text = words.join(" ");

    /* `GPT 5` is written `GPT-5`, and `GLM 4.6` is `GLM-4.6` - the two vendors that hyphenate their own versions. */
    for capitals in ["GPT", "GLM"] {
        text = text.replace(&format!("{capitals} "), &format!("{capitals}-"));
    }

    let text = match tag {
        Some(tag) if !tag.eq_ignore_ascii_case("latest") => format!("{text} ({tag})"),
        _ => text,
    };

    match vendor {
        Some(vendor) => format!("{} {text}", word(&vendor)),
        None => text,
    }
}

/// Is this word a version a following number belongs to? (`5`, `4.5`, `Qwen3` - but not `Sonnet`.)
fn version_word(word: &str) -> bool {
    let tail = word.trim_start_matches(|character: char| character.is_ascii_alphabetic());

    !tail.is_empty() && tail.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.')
}

/// One part of an id, spelled the way its vendor spells it.
fn word(part: &str) -> String {
    let known = match part.to_ascii_lowercase().as_str() {
        "gpt" => Some("GPT"),
        "glm" => Some("GLM"),
        "api" => Some("API"),
        "cli" => Some("CLI"),
        "gguf" => Some("GGUF"),
        "it" => Some("IT"),
        "vl" => Some("VL"),
        "r1" => Some("R1"),
        "v3" => Some("V3"),
        "deepseek" => Some("DeepSeek"),
        "openai" => Some("OpenAI"),
        "openrouter" => Some("OpenRouter"),
        "anthropic" => Some("Anthropic"),
        "nvidia" => Some("NVIDIA"),
        "github" => Some("GitHub"),
        "minimax" => Some("MiniMax"),
        "xai" => Some("xAI"),
        "zai" => Some("Z.ai"),
        "moonshot" => Some("Moonshot"),
        "codex" => Some("Codex"),
        "oss" => Some("OSS"),
        "b" => Some("B"),
        _ => None,
    };

    if let Some(known) = known {
        return known.to_string();
    }

    /* `70b`, `6.7b`, `9b` - a size keeps its capital B. */
    let lowered = part.to_ascii_lowercase();

    if lowered.ends_with('b') && lowered[..lowered.len() - 1].bytes().all(|byte| byte.is_ascii_digit() || byte == b'.') && lowered.len() > 1 {
        return format!("{}B", &lowered[..lowered.len() - 1]);
    }

    let mut characters = part.chars();

    match characters.next() {
        Some(first) if part.chars().any(|character| character.is_ascii_uppercase()) => {
            /* A vendor that writes its own id in capitals (`MiniMax-M2.5`) keeps them. */
            first.to_string() + characters.as_str()
        }
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

/// Rows grouped into one per model: the plainest id to send, the rest as `aliases`, and a readable `name`.
///
/// The order of the answer is the order the rows came in, by the first row of each group, so a provider's own ranking
/// (and the bundle's curation) survives. Within a group the row that *wins* is the one whose id is the canonical key
/// itself, then the shortest id - the plainest spelling is the one a provider documents and a person recognises.
///
/// A row whose id is a bare line word (`sonnet`, `opus`) is not a model: it is the provider's pointer at the newest one
/// of that line. It is folded into that model as an alias, and when no model of the line is in the list it keeps its own
/// row, because an alias that resolves to nothing visible would be a model the person cannot pick.
pub fn collapse(rows: Vec<Value>) -> Vec<Value> {
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();

    for row in rows {
        let id = row["id"].as_str().unwrap_or_default().to_string();

        if id.is_empty() {
            continue;
        }

        let canonical = key(&id);

        match groups.iter_mut().find(|(group, _)| *group == canonical) {
            Some((_, members)) => members.push(row),
            None => groups.push((canonical, vec![row])),
        }
    }

    /* A bare line word joins the newest model of its line - `sonnet` is Claude Sonnet 5.5, not a model called Sonnet. */
    let mut folded: Vec<(String, Vec<Value>)> = Vec::new();

    for (group, members) in groups {
        let bare = version(&group).is_none() && LINES.contains(&group.as_str());
        let newest = bare
            .then(|| {
                folded
                    .iter_mut()
                    .filter(|(other, _)| other.split(['-', '.']).any(|part| part == group) && version(other).is_some())
                    .max_by_key(|(other, _)| version(other).unwrap_or((0, 0)))
            })
            .flatten();

        match newest {
            Some((_, into)) => into.extend(members),
            None => folded.push((group, members)),
        }
    }

    folded
        .into_iter()
        .map(|(group, members)| {
            let ids: Vec<String> = members.iter().filter_map(|row| row["id"].as_str()).map(str::to_string).collect();
            /* The canonical spelling, then the shortest: `claude-sonnet-5-5` over `claude-sonnet-5-5-20260918`. */
            let best = ids
                .iter()
                .enumerate()
                .min_by_key(|(index, id)| (id.to_ascii_lowercase() != group, id.len(), *index))
                .map(|(index, _)| index)
                .unwrap_or(0);
            let mut winner = members[best].clone();
            let aliases: Vec<String> = ids
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != best)
                .map(|(_, id)| id.clone())
                .collect();

            /* Every field the winner left empty is taken from a sibling that has it: the curated row of a group carries
               the price and the window, and the live row carries the id the provider answers to today. */
            for field in ["name", "tier", "ctx", "cost", "size", "line", "ctxLabel"] {
                let empty = match winner.get(field) {
                    None | Some(Value::Null) => true,
                    Some(Value::String(text)) => text.is_empty(),
                    Some(Value::Number(number)) => number.as_i64() == Some(0),
                    _ => false,
                };

                if empty {
                    if let Some(value) = members.iter().find_map(|row| {
                        row.get(field).filter(|value| match value {
                            Value::Null => false,
                            Value::String(text) => !text.is_empty(),
                            Value::Number(number) => number.as_i64() != Some(0),
                            _ => true,
                        })
                    }) {
                        winner[field] = value.clone();
                    }
                }
            }

            let id = winner["id"].as_str().unwrap_or_default().to_string();

            if winner.get("name").and_then(Value::as_str).unwrap_or_default().is_empty() {
                winner["name"] = json!(name(&id));
            }

            if winner.get("line").and_then(Value::as_str).unwrap_or_default().is_empty() {
                winner["line"] = json!(line(&group));
            }

            winner["aliases"] = json!(aliases);

            winner
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids that used to be several rows for one model, and the ones that must stay apart.
    #[test]
    fn the_same_model_under_several_ids_has_one_key() {
        for (left, right) in [
            ("claude-sonnet-5-5", "claude-sonnet-5-5-20260918"),
            ("claude-opus-4-1", "claude-opus-4-1-20250805"),
            ("claude-opus-4", "claude-opus-4-0"),
            ("claude-sonnet-4-5", "claude-sonnet-4-5-v1"),
            ("gemini-3.1-pro", "gemini-3.1-pro-preview"),
            ("gemini-2.0-flash", "gemini-2.0-flash-001"),
            ("kimi-k2", "kimi-k2-0905-preview"),
            ("qwen3-max", "qwen3-max-2026-01-23"),
            ("gemini-2.5-computer-use", "gemini-2.5-computer-use-preview-10-2025"),
            ("mistral-large", "mistral-large-latest"),
        ] {
            assert_eq!(key(left), key(right), "{left} and {right} are the same model");
        }

        /* A size, a variant and a region are different models, whatever else they share. */
        for (left, right) in [
            ("gpt-5", "gpt-5-mini"),
            ("gemini-3-pro", "gemini-3.1-pro"),
            ("kimi-k2", "kimi-k2-turbo"),
            ("deepseek-v4-flash", "deepseek-v4-flash-us"),
            ("qwen2.5-coder:7b", "qwen2.5-coder:32b"),
            ("claude-sonnet-5", "claude-sonnet-5-5"),
            ("anthropic/claude-sonnet-4-5", "claude-sonnet-4-5"),
        ] {
            assert_ne!(key(left), key(right), "{left} and {right} are not the same model");
        }
    }

    /// The report this module exists for: one Sonnet, under the id the CLI documents, with the alias kept.
    #[test]
    fn a_line_alias_is_not_a_model_of_its_own() {
        let rows = collapse(vec![
            json!({ "id": "claude-opus-5-5", "name": "Claude Opus 5.5" }),
            json!({ "id": "claude-sonnet-5-5", "name": "Claude Sonnet 5.5" }),
            json!({ "id": "claude-sonnet-5", "name": "Claude Sonnet 5" }),
            json!({ "id": "sonnet" }),
            json!({ "id": "opus" }),
        ]);
        let ids: Vec<&str> = rows.iter().filter_map(|row| row["id"].as_str()).collect();

        assert_eq!(ids, ["claude-opus-5-5", "claude-sonnet-5-5", "claude-sonnet-5"], "no row called Sonnet");

        let sonnet = rows.iter().find(|row| row["id"] == json!("claude-sonnet-5-5")).unwrap();

        assert_eq!(sonnet["aliases"], json!(["sonnet"]), "the newest Sonnet answers to `sonnet`");
        assert_eq!(rows[0]["aliases"], json!(["opus"]));
        assert_eq!(rows[2]["aliases"], json!([]), "the older Sonnet keeps no alias");
    }

    /// A bare alias with nothing of its line in the list is still offered: it is the only way to pick that model.
    #[test]
    fn an_alias_with_no_model_of_its_line_keeps_its_row() {
        let rows = collapse(vec![json!({ "id": "sonnet" }), json!({ "id": "gpt-5" })]);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], json!("sonnet"));
        assert_eq!(rows[0]["name"], json!("Sonnet"));
    }

    /// The curated fields of one row and the live id of another end up on the same model.
    #[test]
    fn a_group_keeps_the_price_of_whichever_row_carried_it() {
        let rows = collapse(vec![
            json!({ "id": "claude-sonnet-5-5-20260918", "tier": "balanced", "ctx": 0, "cost": "" }),
            json!({ "id": "claude-sonnet-5-5", "tier": "balanced", "ctx": 1_000_000, "cost": "$2 / $10", "name": "Claude Sonnet 5.5" }),
        ]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], json!("claude-sonnet-5-5"), "the plain id is the one to send");
        assert_eq!(rows[0]["cost"], json!("$2 / $10"));
        assert_eq!(rows[0]["ctx"], json!(1_000_000));
        assert_eq!(rows[0]["aliases"], json!(["claude-sonnet-5-5-20260918"]));
    }

    /// The spelling bug a person reads in the menu: `claude-sonnet-5-5` was "Claude Sonnet 5 5".
    #[test]
    fn an_id_becomes_a_name_a_person_can_read() {
        for (id, expected) in [
            ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
            ("claude-opus-5-5", "Claude Opus 5.5"),
            ("claude-fable-5-1", "Claude Fable 5.1"),
            ("claude-haiku-4-5", "Claude Haiku 4.5"),
            ("claude-opus-4", "Claude Opus 4"),
            ("gpt-5-mini", "GPT-5 Mini"),
            ("gpt-5.6-codex", "GPT-5.6 Codex"),
            ("deepseek-reasoner", "DeepSeek Reasoner"),
            ("gemini-3.1-pro", "Gemini 3.1 Pro"),
            ("gemini-4-argon", "Gemini 4 Argon"),
            ("glm-4.6", "GLM-4.6"),
            ("MiniMax-M2.5", "MiniMax M2.5"),
            ("llama-3.3-70b-versatile", "Llama 3.3 70B Versatile"),
            ("anthropic/claude-sonnet-4-5", "Anthropic Claude Sonnet 4.5"),
            ("qwen2.5-coder:7b", "Qwen2.5 Coder (7b)"),
            ("qwen3.5:9b", "Qwen3.5 (9b)"),
            ("", ""),
        ] {
            assert_eq!(name(id), expected, "{id}");
        }
    }

    #[test]
    fn a_version_is_read_in_both_spellings() {
        assert_eq!(version("claude-sonnet-5-5"), Some((5, 5)));
        assert_eq!(version("claude-opus-5"), Some((5, 0)));
        assert_eq!(version("gemini-3.1-pro"), Some((3, 1)));
        assert_eq!(version("gemini-4-argon"), Some((4, 0)));
        assert_eq!(version("qwen3.8-max"), Some((3, 8)));
        assert_eq!(version("glm-4.6"), Some((4, 6)));
        assert_eq!(version("deepseek-chat"), None);
        assert_eq!(version("sonnet"), None);
    }

    #[test]
    fn a_line_groups_a_menu() {
        assert_eq!(line("claude-sonnet-5-5"), "claude sonnet");
        assert_eq!(line("gemini-3.1-pro"), "gemini pro");
        assert_eq!(line("gpt-5-mini"), "gpt mini");
        assert_eq!(line("deepseek-chat"), "deepseek");
    }

    /// What a scan of a CLI's own files finds besides the model ids.
    #[test]
    fn a_scan_keeps_the_ids_and_drops_the_rest() {
        for real in ["gemini-3.1-pro-preview", "gemini-4-argon", "gemini-3.8-flash", "claude-opus-5-5", "gpt-5.6-codex", "gemini-omni-1.1-flash"] {
            assert!(plausible(real), "{real} is a model id");
        }

        for junk in [
            "gemini-9001-super-duper",
            "gemini-3.md",
            "gemini-3.js",
            "gemini-3-flash-is-now-available-in-gemini-cli",
            "gemini-",
            "g",
        ] {
            assert!(!plausible(junk), "{junk} is not a model id");
        }
    }

    /// The numbers in an id: a date folds away, a version does not, and a joke is neither.
    #[test]
    fn a_date_is_told_apart_from_a_version() {
        for date in ["20250805", "202508", "2026", "0905", "1231"] {
            assert!(date_like(date), "{date} is a date");
        }

        for not in ["9001", "5", "55", "999", "4b", ""] {
            assert!(!date_like(not), "{not} is not a date");
        }
    }

    #[test]
    fn a_model_a_chat_cannot_run_is_left_out() {
        for chat in ["claude-sonnet-5-5", "gemini-3.8-flash", "gpt-5", "qwen3-coder-plus", "deepseek-chat"] {
            assert!(chatty(chat), "{chat} takes a turn");
        }

        for other in [
            "gemini-2.5-computer-use-preview-10-2025",
            "gemini-3.1-pro-preview-customtools",
            "gemini-3.8-flash-tts",
            "gemini-3.1-flash-image",
            "gemini-3.5-transcribe",
            "gemini-embedding-2",
            "gemini-3-flash-base",
            "lyria-3.5",
            "gemini-3.8-live",
            "gpt-4o-audio",
            "text-moderation-stable",
        ] {
            assert!(!chatty(other), "{other} cannot take a turn");
        }
    }

    /// Nothing is invented and nothing is lost: every id that went in comes back, as an id or as an alias.
    #[test]
    fn collapsing_never_loses_an_id() {
        let ids = [
            "claude-opus-5-5",
            "claude-opus-5-5-20260918",
            "claude-opus-5",
            "opus",
            "claude-sonnet-5-5",
            "sonnet",
            "gpt-5",
            "gpt-5-mini",
            "gemini-3.1-pro-preview",
            "gemini-3.1-pro",
        ];
        let rows = collapse(ids.iter().map(|id| json!({ "id": id })).collect());
        let mut seen: Vec<String> = Vec::new();

        for row in &rows {
            seen.push(row["id"].as_str().unwrap_or_default().to_string());
            seen.extend(row["aliases"].as_array().unwrap_or(&vec![]).iter().filter_map(Value::as_str).map(str::to_string));
        }

        seen.sort();

        let mut expected: Vec<String> = ids.iter().map(|id| id.to_string()).collect();

        expected.sort();

        assert_eq!(seen, expected);
        assert_eq!(rows.len(), 6, "Opus 5.5, Opus 5, Sonnet 5.5, GPT-5, GPT-5 Mini, Gemini 3.1 Pro");
    }
}
