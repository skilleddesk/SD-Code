//! The cost governor (Trust Kernel, part 5).
//!
//! Four jobs, and one rule that runs through all of them - **a number is either measured or labelled as
//! an estimate** (principle P4):
//!
//! * **account** - every turn's tokens and cost, from the provider's own numbers when it sends them
//!   (`measured`), from the catalogue's price per million tokens when it sends only tokens (`priced`),
//!   `subscription` when a CLI runs on a plan that bills nothing per turn;
//! * **estimate** - before a turn, what it will probably cost, from the size of what is sent;
//! * **cap** - a turn, a chat, a day and a month can each have a budget; a turn that would start over one
//!   is refused with the sentence that says which, and a turn that crosses one while running is stopped;
//! * **route** - a short, simple request on an expensive model gets a cheaper connected model suggested,
//!   with the price difference from the catalogue.
//!
//! "Saved" is only ever a measured number: the same measured tokens priced on the person's baseline
//! model, minus what the turn actually cost. With no baseline chosen there is nothing to claim.

use serde_json::{json, Value};

use crate::store::sqlite::Store;

/// What an engine reported about a turn's size and price. Fields grow monotonically over a turn: the
/// governor keeps the largest value it has seen of each (providers repeat cumulative totals).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The provider's own figure for the turn, when it states one (Claude Code's `total_cost_usd`).
    pub cost_usd: Option<f64>,
}

impl Usage {
    pub fn merge(&mut self, other: Usage) {
        self.input_tokens = self.input_tokens.max(other.input_tokens);
        self.output_tokens = self.output_tokens.max(other.output_tokens);

        if let Some(cost) = other.cost_usd {
            self.cost_usd = Some(self.cost_usd.map_or(cost, |seen| seen.max(cost)));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.input_tokens == 0 && self.output_tokens == 0 && self.cost_usd.is_none()
    }
}

/// `"$4 / $20"` as `(4.0, 20.0)` per million tokens; `None` for `free`, `subscription`, or nothing.
pub fn parse_price(cost: &str) -> Option<(f64, f64)> {
    let mut numbers = cost.split('/').map(|part| part.trim().trim_start_matches('$').trim().parse::<f64>().ok());

    Some((numbers.next()??, numbers.next()??))
}

/// The catalogue's price for a model, by provider when known. A local model is free, and says so.
pub fn price_of(provider: Option<&str>, model: &str) -> Option<(f64, f64)> {
    if provider == Some("ollama") {
        return Some((0.0, 0.0));
    }

    let blocks = crate::providers::models::blocked();
    let rows: Vec<&Value> = blocks
        .iter()
        .filter(|block| provider.map_or(true, |id| block.id == id || id.is_empty()))
        .flat_map(|block| block.models.iter())
        .collect();
    /* `anthropic/claude-opus-5-5` (the registry's spelling) is the catalogue's `claude-opus-5-5`, but
       OpenRouter's own ids contain a slash and are looked up whole first. */
    let bare = model.rsplit_once('/').map(|(_, id)| id).unwrap_or(model);

    rows.iter()
        .find(|row| row["id"].as_str() == Some(model))
        .or_else(|| rows.iter().find(|row| row["id"].as_str() == Some(bare)))
        .and_then(|row| {
            let cost = row["cost"].as_str().unwrap_or_default();

            if cost == "free" {
                Some((0.0, 0.0))
            } else {
                parse_price(cost)
            }
        })
}

/// Tokens in a text, roughly: about four characters per token for Latin script, about two for the
/// scripts a tokenizer splits finer (Bengali, Devanagari, Arabic, CJK). An estimate, and labelled one.
pub fn tokens_in(text: &str) -> u64 {
    let (mut latin, mut other) = (0u64, 0u64);

    for character in text.chars() {
        if character.is_ascii() {
            latin += 1;
        } else {
            other += 1;
        }
    }

    latin / 4 + other / 2 + 1
}

/// What a turn will probably cost, before it runs.
pub fn estimate(engine: &str, provider: Option<&str>, model: &str, prompt: &str, history_chars: usize, agent: bool) -> Value {
    let input = tokens_in(prompt) + history_chars as u64 / 4 + 1500;
    /* An agent calls the model once per step and re-sends the conversation each time; eight steps is the
       median of the turns in this build's own logs. A chat answers once. */
    let steps: u64 = if agent || matches!(engine, "claude_code" | "codex" | "gemini") { 8 } else { 1 };
    let output = (input / 3).clamp(400, 4000);
    let total_in = input * steps;
    let total_out = output * steps / 2 + output;
    let price = price_of(provider, model);
    let subscription = matches!(engine, "claude_code" | "codex" | "gemini") && price.is_none();
    let usd = price.map(|(per_in, per_out)| total_in as f64 / 1e6 * per_in + total_out as f64 / 1e6 * per_out);

    json!({
        "inputTokens": total_in,
        "outputTokens": total_out,
        "usd": usd,
        "source": if subscription { "subscription" } else if usd.is_some() { "estimate" } else { "unknown" },
        "complexity": complexity(prompt),
    })
}

/// How demanding a request looks - the router's only input. Deliberately plain: length, code, and the
/// verbs of work that spans files.
pub fn complexity(prompt: &str) -> &'static str {
    let lowered = prompt.to_lowercase();
    let words = prompt.split_whitespace().count();
    let heavy = [
        "refactor", "architecture", "migrate", "migration", "implement", "build", "create a", "whole", "entire", "every file",
        "debug", "security", "performance", "deploy", "database", "schema", "test suite", "banao", "baniye", "toiri", "তৈরি",
        "সম্পূর্ণ", "পুরো",
    ];
    let has_code = prompt.contains("```") || prompt.contains("fn ") || prompt.contains("function ") || prompt.contains("=>");

    if words > 120 || has_code && words > 40 || heavy.iter().filter(|verb| lowered.contains(*verb)).count() >= 2 {
        "complex"
    } else if words <= 25 && !has_code && !heavy.iter().any(|verb| lowered.contains(*verb)) {
        "simple"
    } else {
        "normal"
    }
}

/// A cheaper model that is connected and good enough for a simple request - or `None`.
///
/// `connected` is `[(provider, model)]` of what the person can actually run. Local models win outright
/// (they cost nothing); otherwise the cheapest priced model that is at least 3x cheaper than the one
/// chosen. A complex request is never routed down.
pub fn cheaper(prompt: &str, provider: Option<&str>, model: &str, connected: &[(String, String)]) -> Option<Value> {
    if complexity(prompt) != "simple" {
        return None;
    }

    let current = price_of(provider, model)?;
    let blended = |price: (f64, f64)| price.0 + price.1 / 4.0;

    if blended(current) <= 0.0 {
        return None;
    }

    let best = connected
        .iter()
        .filter(|(p, m)| !(Some(p.as_str()) == provider && m == model))
        .filter_map(|(p, m)| price_of(Some(p), m).map(|price| (p, m, price)))
        .min_by(|a, b| blended(a.2).partial_cmp(&blended(b.2)).unwrap_or(std::cmp::Ordering::Equal))?;

    if blended(best.2) * 3.0 > blended(current) {
        return None;
    }

    Some(json!({
        "provider": best.0,
        "model": best.1,
        "reason": if blended(best.2) == 0.0 {
            "A short, simple request: a local model answers it for free.".to_string()
        } else {
            format!("A short, simple request: {} costs about {:.0}x less for it.", best.1, blended(current) / blended(best.2))
        },
        "pricePerMillion": { "in": best.2.0, "out": best.2.1 },
    }))
}

/// The turn's cost and where the number came from.
pub fn settle(engine: &str, provider: Option<&str>, model: &str, usage: &Usage) -> (f64, &'static str) {
    if let Some(cost) = usage.cost_usd {
        return (cost, "measured");
    }

    if usage.input_tokens + usage.output_tokens == 0 {
        return (0.0, "none");
    }

    match price_of(provider, model) {
        Some((per_in, per_out)) => (
            usage.input_tokens as f64 / 1e6 * per_in + usage.output_tokens as f64 / 1e6 * per_out,
            if per_in == 0.0 && per_out == 0.0 { "local" } else { "priced" },
        ),
        None if matches!(engine, "claude_code" | "codex" | "gemini") => (0.0, "subscription"),
        None => (0.0, "unpriced"),
    }
}

/// The same measured tokens on the baseline model: what "saved" is compared against. `None` when there
/// is no baseline, or the turn's tokens were not measured.
pub fn baseline(store: &Store, usage: &Usage) -> Option<f64> {
    if usage.input_tokens + usage.output_tokens == 0 {
        return None;
    }

    let chosen = store.setting("cost.baseline").ok().flatten()?;
    let (provider, model) = chosen.split_once('|')?;
    let (per_in, per_out) = price_of(Some(provider), model)?;

    Some(usage.input_tokens as f64 / 1e6 * per_in + usage.output_tokens as f64 / 1e6 * per_out)
}

/// The budgets a person set (`cost.budget` setting, JSON): `{turn, chat, day, month}` in dollars.
pub fn budgets(store: &Store) -> Value {
    store
        .setting("cost.budget")
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_else(|| json!({}))
}

fn cap(budgets: &Value, key: &str) -> Option<f64> {
    budgets[key].as_f64().filter(|usd| *usd > 0.0)
}

/// What has been spent: today, this month, in this chat, and in total, from the usage rows.
pub fn spent(store: &Store, session_id: Option<&str>) -> Value {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let month = &today[..7];
    let rows = store.usage_rows(Some(&format!("{month}-01"))).unwrap_or_default();
    let sum = |filter: &dyn Fn(&Value) -> bool| -> f64 { rows.iter().filter(|row| filter(row)).map(|row| row["costUsd"].as_f64().unwrap_or(0.0)).sum() };
    let chat = session_id.map(|session| {
        store
            .usage_rows(None)
            .unwrap_or_default()
            .iter()
            .filter(|row| row["sessionId"] == session)
            .map(|row| row["costUsd"].as_f64().unwrap_or(0.0))
            .sum::<f64>()
    });

    json!({
        "day": sum(&|row| row["day"] == today.as_str()),
        "month": sum(&|_| true),
        "chat": chat,
    })
}

/// Refuses a turn that would start over a budget, with the sentence that says which one and how to go on.
pub fn check_before(store: &Store, session_id: &str, policy_turn_cap: Option<f64>, estimate_usd: Option<f64>) -> Result<(), String> {
    let budgets = budgets(store);
    let spent = spent(store, Some(session_id));

    for (key, label, amount) in [
        ("day", "today's", spent["day"].as_f64().unwrap_or(0.0)),
        ("month", "this month's", spent["month"].as_f64().unwrap_or(0.0)),
        ("chat", "this chat's", spent["chat"].as_f64().unwrap_or(0.0)),
    ] {
        if let Some(limit) = cap(&budgets, key) {
            if amount >= limit {
                return Err(format!(
                    "Budget reached: {label} AI spend is ${amount:.2} of the ${limit:.2} you set. Raise the budget in the cost panel, or switch to a local model (free) to continue."
                ));
            }
        }
    }

    let turn_cap = policy_turn_cap.or_else(|| cap(&budgets, "turn"));

    if let (Some(limit), Some(estimate)) = (turn_cap, estimate_usd) {
        /* An estimate is uncertain: it takes three times the cap to refuse on one, and the sentence says it is an estimate. */
        if estimate > limit * 3.0 {
            return Err(format!(
                "This turn is estimated at about ${estimate:.2}, well over the ${limit:.2} per-turn budget. Use a cheaper model, narrow the request, or raise the budget."
            ));
        }
    }

    Ok(())
}

/// The per-turn cap in force: the project's policy first, then the person's own budget.
pub fn turn_cap(store: &Store, policy_turn_cap: Option<f64>) -> Option<f64> {
    policy_turn_cap.or_else(|| cap(&budgets(store), "turn"))
}

/// Watches a running turn for the shape of a runaway: the same tool on the same target again and again
/// with nothing changing between.
///
/// There is no cap on the number of calls (0.15.6): a 150-call cap stopped long page builds that were on
/// track, the same way the step limit 0.15.2 removed did. A budget set in Settings is the bound.
#[derive(Debug, Default)]
pub struct Runaway {
    last: Option<String>,
    repeats: usize,
}

/// The same action this many times in a row is a loop, not progress.
pub const REPEAT_LIMIT: usize = 5;

impl Runaway {
    /// A call finished and changed something (an edit with a diff), so the next call on the same target
    /// is new work, not a repeat (0.15.6). The report's turns were stopped as a loop after five
    /// `Edit page.tsx` calls in a row that each changed different lines.
    pub fn progress(&mut self) {
        self.last = None;
        self.repeats = 0;
    }

    /// One tool call. `Some(reason)` when the turn should stop.
    pub fn observe(&mut self, name: &str, target: &str) -> Option<String> {
        let key = format!("{name}\u{1f}{target}");

        /* A call with no target cannot be shown to be the same call again (0.14.1): Claude's Bash cards
           once arrived without their command, and five different commands were stopped as a loop. */
        if target.trim().is_empty() {
            self.last = None;
            self.repeats = 0;
        } else if self.last.as_deref() == Some(key.as_str()) {
            self.repeats += 1;
        } else {
            self.last = Some(key);
            self.repeats = 1;
        }

        if self.repeats >= REPEAT_LIMIT {
            return Some(format!(
                "Stopped a loop: `{name} {target}` ran {REPEAT_LIMIT} times in a row without anything changing between. Look at the last result, then send a clearer instruction."
            ));
        }

        None
    }
}

/// The cost panel's numbers: spend by day for 30 days, by model, by chat and by project, what was saved
/// against the baseline (measured turns only), and the budgets.
pub fn summary(store: &Store) -> Value {
    let since = (chrono::Utc::now() - chrono::Duration::days(30)).format("%Y-%m-%d").to_string();
    let rows = store.usage_rows(Some(&since)).unwrap_or_default();
    let mut by_day: std::collections::BTreeMap<String, f64> = Default::default();
    let mut by_model: std::collections::HashMap<String, (f64, u64, u64, usize)> = Default::default();
    let mut by_project: std::collections::HashMap<String, f64> = Default::default();
    let mut by_site: std::collections::HashMap<String, f64> = Default::default();
    let (mut saved, mut measured, mut estimated_turns) = (0.0f64, 0usize, 0usize);

    for row in &rows {
        let cost = row["costUsd"].as_f64().unwrap_or(0.0);

        *by_day.entry(row["day"].as_str().unwrap_or_default().to_string()).or_default() += cost;

        let entry = by_model.entry(row["model"].as_str().unwrap_or_default().to_string()).or_default();

        entry.0 += cost;
        entry.1 += row["inputTokens"].as_u64().unwrap_or(0);
        entry.2 += row["outputTokens"].as_u64().unwrap_or(0);
        entry.3 += 1;

        if let Some(project) = row["projectRoot"].as_str() {
            *by_project.entry(project.to_string()).or_default() += cost;
        }

        if let Some(site) = row["siteId"].as_str() {
            *by_site.entry(site.to_string()).or_default() += cost;
        }

        match row["costSource"].as_str() {
            Some("measured" | "priced" | "local") => {
                measured += 1;

                if let Some(base) = row["baselineUsd"].as_f64() {
                    saved += (base - cost).max(0.0);
                }
            }
            _ => estimated_turns += 1,
        }
    }

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    json!({
        "days": by_day.iter().map(|(day, usd)| json!({ "day": day, "usd": usd })).collect::<Vec<_>>(),
        "models": by_model.iter().map(|(model, (usd, input, output, turns))| json!({ "model": model, "usd": usd, "inputTokens": input, "outputTokens": output, "turns": turns })).collect::<Vec<_>>(),
        "projects": by_project.iter().map(|(root, usd)| json!({ "root": root, "usd": usd })).collect::<Vec<_>>(),
        "sites": by_site.iter().map(|(site, usd)| json!({ "siteId": site, "usd": usd })).collect::<Vec<_>>(),
        "today": by_day.get(&today).copied().unwrap_or(0.0),
        "spent": spent(store, None),
        "savedUsd": saved,
        "measuredTurns": measured,
        "unmeasuredTurns": estimated_turns,
        "baseline": store.setting("cost.baseline").ok().flatten(),
        "budgets": budgets(store),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_prices_models_and_local_ones_are_free() {
        assert_eq!(price_of(Some("anthropic-api"), "claude-opus-5-5"), Some((4.0, 20.0)));
        assert_eq!(price_of(Some("ollama"), "llama3.2:3b"), Some((0.0, 0.0)));
        assert_eq!(price_of(None, "deepseek-chat"), Some((0.14, 0.28)));
        assert_eq!(price_of(Some("claude"), "sonnet"), None, "a subscription has no per-token price");
    }

    #[test]
    fn a_measured_cost_wins_and_a_subscription_is_not_priced() {
        let usage = Usage { input_tokens: 1_000_000, output_tokens: 100_000, cost_usd: None };

        assert_eq!(settle("native_api", Some("anthropic-api"), "claude-opus-5-5", &usage), (6.0, "priced"));
        assert_eq!(settle("claude_code", Some("claude"), "sonnet", &Usage { cost_usd: Some(0.03), ..usage }).1, "measured");
        assert_eq!(settle("codex", Some("openai"), "default", &usage), (0.0, "subscription"));
        assert_eq!(settle("native_api", None, "x", &Usage::default()).1, "none");
    }

    #[test]
    fn usage_keeps_the_largest_cumulative_value() {
        let mut usage = Usage::default();

        usage.merge(Usage { input_tokens: 120, output_tokens: 1, cost_usd: None });
        usage.merge(Usage { input_tokens: 0, output_tokens: 42, cost_usd: None });
        usage.merge(Usage { input_tokens: 0, output_tokens: 0, cost_usd: Some(0.01) });

        assert_eq!(usage, Usage { input_tokens: 120, output_tokens: 42, cost_usd: Some(0.01) });
    }

    #[test]
    fn simple_requests_are_routed_to_a_cheaper_connected_model() {
        let connected = vec![
            ("anthropic-api".to_string(), "claude-opus-5-5".to_string()),
            ("deepseek".to_string(), "deepseek-chat".to_string()),
        ];
        let suggestion = cheaper("what does this error mean?", Some("anthropic-api"), "claude-opus-5-5", &connected).unwrap();

        assert_eq!(suggestion["model"], "deepseek-chat");
        assert!(cheaper("refactor the whole architecture and migrate the database schema", Some("anthropic-api"), "claude-opus-5-5", &connected).is_none());

        let local = vec![("ollama".to_string(), "llama3.2:3b".to_string())];

        assert!(cheaper("hi, what is 2+2?", Some("anthropic-api"), "claude-opus-5-5", &local).unwrap()["reason"].as_str().unwrap().contains("free"));
    }

    #[test]
    fn a_loop_is_stopped_on_the_fifth_identical_call() {
        let mut runaway = Runaway::default();

        for _ in 0..4 {
            assert!(runaway.observe("Run", "npm test").is_none());
        }

        assert!(runaway.observe("Run", "npm test").is_some());

        let mut varied = Runaway::default();

        for index in 0..20 {
            assert!(varied.observe("Edit", &format!("file{index}.ts")).is_none());
        }
    }

    /// The 0.14 report: five Claude Bash cards with no command were stopped as one command run five times.
    #[test]
    fn calls_without_a_target_are_never_a_loop() {
        let mut runaway = Runaway::default();

        for _ in 0..20 {
            assert!(runaway.observe("Bash", "").is_none());
        }
    }

    /// No call cap: a long turn of different calls is never stopped for its length.
    #[test]
    fn a_long_turn_is_not_stopped_for_its_length() {
        let mut runaway = Runaway::default();

        for index in 0..1000 {
            assert!(runaway.observe("Edit", &format!("file{index}.ts")).is_none());
        }
    }

    /// The 0.15.6 report: edits to one file that each changed it are work, not a loop.
    #[test]
    fn edits_that_change_the_file_are_never_a_loop() {
        let mut runaway = Runaway::default();

        for _ in 0..20 {
            assert!(runaway.observe("Edit", "page.tsx").is_none());
            runaway.progress();
        }
    }

    #[test]
    fn bengali_text_counts_as_more_tokens_per_character() {
        assert!(tokens_in("আমার সাইট ঠিক করো") > tokens_in("fix my site pls"));
    }

    #[test]
    fn a_budget_that_is_spent_refuses_the_next_turn() {
        let store = Store::in_memory().unwrap();

        store.set_setting("cost.budget", r#"{"day": 0.05}"#).unwrap();
        store
            .record_usage("t1", "s1", None, None, "native_api", "claude-opus-5-5", Some("anthropic-api"), 1000, 1000, 0.06, "priced", None, None)
            .unwrap();

        let refusal = check_before(&store, "s1", None, None).unwrap_err();

        assert!(refusal.contains("today's"), "{refusal}");
        assert!(check_before(&Store::in_memory().unwrap(), "s1", None, None).is_ok());
    }
}
