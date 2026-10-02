//! `/research` (0.16.1): a question answered from the web, with its sources - and a way to keep a local
//! model off the web the rest of the time.
//!
//! A research turn is an agent turn with a different brief and a smaller toolbox, not a second loop:
//!
//! ```text
//!   the question ─► a plan (update_plan) ─► 3-5 searches ─► the best pages, read one by one
//!        (a sub-agent per page when the window is small) ─► the answer, every claim with its [n]
//! ```
//!
//! What this module adds around the agent's loop:
//!
//! * **limits** - searches, pages and minutes. The agent's usual guards are Stop, the cost governor and
//!   the runaway detector, and on a local model the cost governor sees $0 and never stops anything;
//! * **sources** - every page read and every result shown is numbered once, and the turn ends with the
//!   list (`ResearchSources`): title, address and date, so a reader can check a claim;
//! * **the search service** - DuckDuckGo needs nothing and stays the default; SearXNG (your own server),
//!   Tavily, Brave and Serper can be chosen in Settings → Research, with their key in the OS keychain;
//! * **the web gate** - on a local model the web is used only through `/research` (on by default), and a
//!   folder whose policy is `privacy = "local-only"` opens it for a research turn alone.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// How far one research turn may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_searches: usize,
    pub max_pages: usize,
    pub max_minutes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_searches: 5, max_pages: 10, max_minutes: 10 }
    }
}

impl Limits {
    /// Model calls a research turn may make: one per search and per page, and room to plan and answer.
    pub fn steps(&self) -> usize {
        self.max_searches + self.max_pages + 8
    }
}

/// The search service a `web_search` asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchProvider {
    /// DuckDuckGo's HTML page: no key, no account - and no dates, and it can be rate-limited.
    DuckDuckGo,
    /// A SearXNG server - usually your own. Its `settings.yml` must allow the `json` format.
    Searxng(String),
    Tavily,
    Brave,
    Serper,
}

impl SearchProvider {
    pub fn parse(id: &str, searxng_url: &str) -> Self {
        match id.trim() {
            "searxng" if !searxng_url.trim().is_empty() => Self::Searxng(searxng_url.trim().trim_end_matches('/').to_string()),
            "tavily" => Self::Tavily,
            "brave" => Self::Brave,
            "serper" => Self::Serper,
            _ => Self::DuckDuckGo,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::DuckDuckGo => "duckduckgo",
            Self::Searxng(_) => "searxng",
            Self::Tavily => "tavily",
            Self::Brave => "brave",
            Self::Serper => "serper",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Searxng(_) => "SearXNG",
            Self::Tavily => "Tavily",
            Self::Brave => "Brave Search",
            Self::Serper => "Serper (Google)",
        }
    }

    /// Whether the service needs an API key.
    pub fn keyed(&self) -> bool {
        matches!(self, Self::Tavily | Self::Brave | Self::Serper)
    }
}

/// The keychain entry a search service's key is kept under.
pub fn key_ref(provider_id: &str) -> String {
    format!("sdc.search.{provider_id}")
}

/// The services that take a key - what `research.key.set` accepts.
pub const KEYED: &[&str] = &["tavily", "brave", "serper"];

/// Research settings, as Settings → Research left them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub provider: SearchProvider,
    pub limits: Limits,
    /// A local model reads the web only inside `/research` - off by default (live check, 0.16.1: the owner
    /// wants every model to search when asked, the way an API model does).
    pub local_web_only_research: bool,
    /// An API model that writes the final answer from what a local model gathered: `(provider, model)`.
    pub synthesis: Option<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self { provider: SearchProvider::DuckDuckGo, limits: Limits::default(), local_web_only_research: false, synthesis: None }
    }
}

fn slot() -> &'static Mutex<Config> {
    static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();

    CONFIG.get_or_init(|| Mutex::new(Config::default()))
}

/// The settings in force.
pub fn config() -> Config {
    slot().lock().map(|config| config.clone()).unwrap_or_default()
}

/// Reads the settings from the store - at start, and after every change to one of them.
pub fn configure(store: &crate::store::sqlite::Store) {
    let read = |key: &str| store.setting(key).ok().flatten().unwrap_or_default();
    let number = |key: &str, fallback: usize, most: usize| read(key).trim().parse::<usize>().ok().filter(|value| *value > 0).unwrap_or(fallback).min(most);
    let defaults = Limits::default();
    let synthesis_model = read("research.synthesisModel");
    let synthesis_provider = read("research.synthesisProvider");
    let config = Config {
        provider: SearchProvider::parse(&read("research.searchProvider"), &read("research.searxngUrl")),
        limits: Limits {
            max_searches: number("research.maxSearches", defaults.max_searches, 20),
            max_pages: number("research.maxPages", defaults.max_pages, 30),
            max_minutes: number("research.maxMinutes", defaults.max_minutes as usize, 60) as u64,
        },
        local_web_only_research: matches!(read("research.localWebOnly").as_str(), "true" | "on"),
        synthesis: (!synthesis_model.trim().is_empty() && !synthesis_provider.trim().is_empty())
            .then(|| (synthesis_provider.trim().to_string(), synthesis_model.trim().to_string())),
    };

    if let Ok(mut slot) = slot().lock() {
        *slot = config;
    }

    if let Some(tokens) = store.setting("ollama.contextTokens").ok().flatten().and_then(|value| value.trim().parse::<u64>().ok()) {
        crate::engines::ollama::set_context_cap(tokens);
    }
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub date: Option<String>,
}

fn http() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .redirects(3)
        .build()
}

fn read_json(result: Result<ureq::Response, ureq::Error>, service: &str) -> Result<Value, String> {
    match result {
        Ok(response) => response
            .into_string()
            .map_err(|error| format!("{service} could not be read: {error}"))
            .and_then(|body| serde_json::from_str::<Value>(&body).map_err(|error| format!("{service} answered something that is not JSON: {error}"))),
        Err(ureq::Error::Status(401 | 403, _)) if service == "SearXNG" => Err(
            "SearXNG refused the JSON format (HTTP 403). Add `json` under `search: formats:` in the server's settings.yml and restart it.".to_string(),
        ),
        Err(ureq::Error::Status(401 | 403, _)) => Err(format!("{service} refused the API key (HTTP 401/403). Check it in Settings → Research.")),
        Err(ureq::Error::Status(429, _)) => Err(format!("{service} says too many requests (HTTP 429) - the plan's limit is used up for now.")),
        Err(ureq::Error::Status(code, response)) => Err(format!("{service} answered HTTP {code} {}", response.status_text())),
        Err(ureq::Error::Transport(transport)) => Err(format!("{service} could not be reached: {transport}")),
    }
}

fn text(value: &Value, keys: &[&str]) -> String {
    keys.iter().find_map(|key| value[*key].as_str().filter(|text| !text.is_empty())).unwrap_or_default().trim().to_string()
}

fn date(value: &Value, keys: &[&str]) -> Option<String> {
    let found = text(value, keys);

    (!found.is_empty()).then_some(found)
}

/// Results from one service, in the shape every service shares.
pub fn search_with(provider: &SearchProvider, query: &str, limit: usize) -> Result<Vec<Hit>, String> {
    let key = || {
        crate::auth::keychain::get(&key_ref(provider.id()))
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| format!("{} needs an API key. Add it in Settings → Research, or pick DuckDuckGo (no key).", provider.label()))
    };

    match provider {
        SearchProvider::DuckDuckGo => Ok(super::web::search(query, limit)?
            .iter()
            .map(|result| Hit { title: text(result, &["title"]), url: text(result, &["url"]), snippet: text(result, &["snippet"]), date: None })
            .collect()),
        SearchProvider::Searxng(base) => {
            let url = format!("{base}/search?q={}&format=json", super::web::encode(query));
            let value = read_json(http().get(&url).set("accept", "application/json").call(), "SearXNG")?;

            Ok(value["results"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .take(limit)
                        .map(|row| Hit { title: text(row, &["title"]), url: text(row, &["url"]), snippet: text(row, &["content"]), date: date(row, &["publishedDate"]) })
                        .collect()
                })
                .unwrap_or_default())
        }
        SearchProvider::Tavily => {
            let body = json!({ "query": query, "max_results": limit, "search_depth": "basic" }).to_string();
            let value = read_json(
                http()
                    .post("https://api.tavily.com/search")
                    .set("content-type", "application/json")
                    .set("authorization", &format!("Bearer {}", key()?))
                    .send_string(&body),
                "Tavily",
            )?;

            Ok(value["results"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|row| Hit { title: text(row, &["title"]), url: text(row, &["url"]), snippet: text(row, &["content"]), date: date(row, &["published_date"]) })
                        .collect()
                })
                .unwrap_or_default())
        }
        SearchProvider::Brave => {
            let url = format!("https://api.search.brave.com/res/v1/web/search?q={}&count={}", super::web::encode(query), limit.min(20));
            let value = read_json(
                http().get(&url).set("accept", "application/json").set("x-subscription-token", &key()?).call(),
                "Brave Search",
            )?;

            Ok(value
                .pointer("/web/results")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .map(|row| Hit {
                            title: text(row, &["title"]),
                            url: text(row, &["url"]),
                            snippet: super::web::html_to_text(&text(row, &["description"])).trim().to_string(),
                            date: date(row, &["page_age", "age"]),
                        })
                        .collect()
                })
                .unwrap_or_default())
        }
        SearchProvider::Serper => {
            let body = json!({ "q": query, "num": limit }).to_string();
            let value = read_json(
                http()
                    .post("https://google.serper.dev/search")
                    .set("content-type", "application/json")
                    .set("x-api-key", &key()?)
                    .send_string(&body),
                "Serper",
            )?;

            Ok(value["organic"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|row| Hit { title: text(row, &["title"]), url: text(row, &["link"]), snippet: text(row, &["snippet"]), date: date(row, &["date"]) })
                        .collect()
                })
                .unwrap_or_default())
        }
    }
}

/// A search on the configured service. A keyed service that fails (no key, the plan used up, down) is
/// said in a note and DuckDuckGo answers instead, so a research turn is not lost to a billing page.
pub fn search(query: &str, limit: usize) -> Result<(Vec<Hit>, String), String> {
    let provider = config().provider;

    match search_with(&provider, query, limit) {
        Ok(hits) => Ok((hits.into_iter().take(limit).collect(), provider.label().to_string())),
        Err(error) if provider != SearchProvider::DuckDuckGo => {
            let hits = search_with(&SearchProvider::DuckDuckGo, query, limit)
                .map_err(|fallback| format!("{error} DuckDuckGo did not answer either: {fallback}"))?;

            Ok((hits, format!("DuckDuckGo, because {error}")))
        }
        Err(error) => Err(error),
    }
}

/// A page or a result the answer may cite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub n: usize,
    pub title: String,
    pub url: String,
    pub date: Option<String>,
    /// Read in full (`web_fetch`), not only seen in a result list.
    pub read: bool,
}

/// One research turn's counters and sources, shared with its sub-agents.
pub struct Session {
    pub limits: Limits,
    started: Instant,
    searches: AtomicUsize,
    pages: AtomicUsize,
    sources: Mutex<Vec<Source>>,
}

impl Session {
    pub fn new(limits: Limits) -> Self {
        Self { limits, started: Instant::now(), searches: AtomicUsize::new(0), pages: AtomicUsize::new(0), sources: Mutex::new(Vec::new()) }
    }

    /// Whether the time is up.
    pub fn expired(&self) -> bool {
        self.started.elapsed() >= Duration::from_secs(self.limits.max_minutes * 60)
    }

    /// Takes one search, or says why there are none left.
    pub fn take_search(&self) -> Result<usize, String> {
        if self.expired() {
            return Err(self.out_of("time"));
        }

        let used = self.searches.fetch_add(1, Ordering::SeqCst);

        if used >= self.limits.max_searches {
            return Err(self.out_of("searches"));
        }

        Ok(used + 1)
    }

    /// Takes one page, or says why there are none left.
    pub fn take_page(&self) -> Result<usize, String> {
        if self.expired() {
            return Err(self.out_of("time"));
        }

        let used = self.pages.fetch_add(1, Ordering::SeqCst);

        if used >= self.limits.max_pages {
            return Err(self.out_of("pages"));
        }

        Ok(used + 1)
    }

    fn out_of(&self, what: &str) -> String {
        format!(
            "The research limit is reached ({what}: {} searches, {} pages, {} minutes). Do not search or fetch again - write the answer now from what you have, citing [n], and say plainly what you could not find.",
            self.limits.max_searches, self.limits.max_pages, self.limits.max_minutes
        )
    }

    /// The number a source is cited by: the same address keeps its number.
    pub fn cite(&self, url: &str, title: &str, date: Option<String>, read: bool) -> usize {
        let Ok(mut sources) = self.sources.lock() else {
            return 0;
        };

        if let Some(known) = sources.iter_mut().find(|source| same_page(&source.url, url)) {
            known.read |= read;

            if known.title.is_empty() && !title.is_empty() {
                known.title = title.to_string();
            }

            if known.date.is_none() {
                known.date = date;
            }

            return known.n;
        }

        let n = sources.len() + 1;

        sources.push(Source { n, title: title.to_string(), url: url.to_string(), date, read });

        n
    }

    pub fn sources(&self) -> Vec<Source> {
        self.sources.lock().map(|sources| sources.clone()).unwrap_or_default()
    }

    /// The sources as the `ResearchSources` event carries them: the ones read, then the ones only seen.
    pub fn sources_json(&self) -> Value {
        let mut sources = self.sources();

        sources.sort_by_key(|source| (!source.read, source.n));

        json!(sources
            .iter()
            .map(|source| json!({ "n": source.n, "title": source.title, "url": source.url, "date": source.date, "read": source.read }))
            .collect::<Vec<_>>())
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.searches.load(Ordering::SeqCst).min(self.limits.max_searches), self.pages.load(Ordering::SeqCst).min(self.limits.max_pages))
    }
}

fn same_page(left: &str, right: &str) -> bool {
    let bare = |url: &str| url.trim().trim_end_matches('/').split('#').next().unwrap_or(url).to_lowercase();

    bare(left) == bare(right)
}

/// The tools a research turn gets - the web, the plan, sub-agents, and reading the folder's files.
pub const TOOLS: &[&str] = &["web_search", "web_fetch", "task", "update_plan", "read_file"];

/// The brief of a research turn.
pub fn system_prompt(language: &crate::understand::Reading, limits: &Limits, provider: &str, small_window: bool) -> String {
    let per_page = if small_window {
        "\n- This model's window is small: for each page worth reading, hand it to a task sub-agent (\"read <url> and report what it says about <question>, with the page's date\") instead of fetching it yourself, so your own context holds only the summaries."
    } else {
        ""
    };

    format!(
        "You are SDC Research. The person asked a question that needs the web; answer it from sources, not from memory.\n\
         \n\
         Today is {today}. Search service: {provider}. Limits for this question: {searches} searches, {pages} pages, {minutes} minutes - SDC stops you when one is used up.\n\
         \n\
         How to work:\n\
         - First call update_plan with 3 to 5 search queries that together cover the question (split a broad question into parts). For anything that changes, put the current year ({year}) in the query - never an older year from your training. Write the queries in English (the web's largest index), unless the topic belongs to another language's sources.\n\
         - Run the searches with web_search. Every result has a number [n]; the same page keeps its number all through.\n\
         - Read the most useful pages with web_fetch - official documentation, the primary source, recent dates - at least two different sources before you answer, unless the first one fully answers the question.{per_page}\n\
         - Then answer. Every claim carries the [n] of the source it comes from. Prefer recent sources and say the date when it matters. When sources disagree, say so.\n\
         - If the sources do not answer the question, say exactly what could not be found. Never fill a gap with a guess, and never cite a number you were not given.\n\
         - Keep the answer focused: a short direct answer first, then the details. Do not list the sources yourself - SDC adds the numbered list under your answer.\n\
         - Language: the person writes in {label}. Write the answer in {reply}; keep names, numbers, code and quotes as they are.",
        today = chrono::Local::now().format("%Y-%m-%d"),
        year = chrono::Local::now().format("%Y"),
        searches = limits.max_searches,
        pages = limits.max_pages,
        minutes = limits.max_minutes,
        label = language.label,
        reply = if language.code == "en" { "English" } else { language.reply_in },
    )
}

/// The brief of a research sub-agent: one page or one sub-question, summarised for the main agent.
pub fn sub_agent_prompt() -> String {
    "You are a research sub-agent of SDC Research. Do the job you are given - usually: read one page (web_fetch) or run one search, \
     and report what it says about the question. Report only facts the page states, each with its [n] number exactly as the tool \
     result gave it, and the page's date when it shows one. If the page does not answer the question, say so in one line. \
     Your report is read by another model: no pleasantries, at most 250 words."
        .to_string()
}

/// The brief of the model that writes the final answer from the research of another (Settings → Research → final answer model).
pub fn synthesis_prompt(language: &crate::understand::Reading) -> String {
    format!(
        "You write the final answer to a research question. Another model has searched the web and read the pages; its notes, \
         with numbered sources [n], are in the conversation. Answer the person's question from those notes only: a short direct answer \
         first, then the details, every claim with its [n]. Say plainly what the notes do not answer - never guess, never invent a \
         source number. Do not list the sources yourself; SDC adds them under your answer. Write in {reply}.",
        reply = if language.code == "en" { "English" } else { language.reply_in },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_is_parsed_from_its_settings() {
        assert_eq!(SearchProvider::parse("", ""), SearchProvider::DuckDuckGo);
        assert_eq!(SearchProvider::parse("searxng", "https://search.example.org/"), SearchProvider::Searxng("https://search.example.org".into()));
        /* SearXNG without an address is no service at all - DuckDuckGo answers. */
        assert_eq!(SearchProvider::parse("searxng", ""), SearchProvider::DuckDuckGo);
        assert_eq!(SearchProvider::parse("tavily", ""), SearchProvider::Tavily);
        assert!(SearchProvider::Serper.keyed() && !SearchProvider::DuckDuckGo.keyed());
    }

    #[test]
    fn a_session_stops_at_its_limits_and_says_how_to_finish() {
        let session = Session::new(Limits { max_searches: 2, max_pages: 1, max_minutes: 5 });

        assert_eq!(session.take_search(), Ok(1));
        assert_eq!(session.take_search(), Ok(2));
        assert!(session.take_search().unwrap_err().contains("write the answer now"));
        assert_eq!(session.take_page(), Ok(1));
        assert!(session.take_page().is_err());
        assert_eq!(session.counts(), (2, 1));
    }

    #[test]
    fn a_session_stops_when_its_time_is_up() {
        let session = Session::new(Limits { max_searches: 5, max_pages: 5, max_minutes: 0 });

        assert!(session.expired());
        assert!(session.take_search().unwrap_err().contains("write the answer now"));
        assert!(session.take_page().unwrap_err().contains("write the answer now"));
    }

    #[test]
    fn a_page_keeps_its_number_and_read_pages_come_first() {
        let session = Session::new(Limits::default());

        assert_eq!(session.cite("https://a.example/x", "A", None, false), 1);
        assert_eq!(session.cite("https://b.example/", "B", Some("2026-01-02".into()), true), 2);
        assert_eq!(session.cite("https://a.example/x/", "", None, true), 1, "the same page, written differently");

        let listed = session.sources_json();

        assert_eq!(listed[0]["n"], 1);
        assert_eq!(listed[0]["read"], true);
        assert_eq!(listed[1]["date"], "2026-01-02");
    }

    #[test]
    fn the_brief_names_the_limits_and_the_language() {
        let bengali = crate::understand::Reading::of("আমাকে নতুন ফ্রেমওয়ার্ক সম্পর্কে বলো");
        let brief = system_prompt(&bengali, &Limits::default(), "DuckDuckGo", true);

        assert!(brief.contains("5 searches, 10 pages, 10 minutes"));
        assert!(brief.contains("task sub-agent"));
        assert!(brief.contains(bengali.reply_in));
    }
}
