//! The agent's two doors to the web (0.13): `web_fetch` reads one page as text, `web_search` finds pages.
//!
//! A coding agent that cannot read documentation guesses at APIs; Claude Code has WebFetch and WebSearch
//! for that reason. Both run on this machine (never on a host - a VPS's outbound traffic is its owner's),
//! read-only, capped, and without cookies or credentials of any kind.

use std::io::Read;
use std::time::Duration;

use serde_json::{json, Value};

/// The most of one page handed to the model, as text.
pub const PAGE_CAP: usize = 60_000;
/// The most bytes read off the wire for one page.
const BYTES_CAP: u64 = 4 * 1024 * 1024;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 SDC-Agent";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .redirects(0)
        .user_agent(USER_AGENT)
        .build()
}

/// Whether a URL is one the agent may fetch: http(s) to a public name - never this machine, the local
/// network, or the cloud metadata address a server leaks its credentials through.
pub fn allowed(url: &str) -> Result<(), String> {
    let lowered = url.trim().to_ascii_lowercase();

    if !(lowered.starts_with("http://") || lowered.starts_with("https://")) {
        return Err("only http:// and https:// addresses can be fetched".to_string());
    }

    let host = lowered.split("://").nth(1).unwrap_or_default().split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or(host);
    let name = if host.starts_with('[') { host.split(']').next().unwrap_or(host).trim_start_matches('[') } else { host.split(':').next().unwrap_or(host) };

    let private = name == "localhost"
        || name.ends_with(".localhost")
        || name.ends_with(".local")
        || name.ends_with(".internal")
        || name.starts_with("127.")
        || name.starts_with("10.")
        || name.starts_with("192.168.")
        || name.starts_with("169.254.")
        || name == "0.0.0.0"
        || name == "::1"
        || (name.starts_with("172.") && name.split('.').nth(1).and_then(|part| part.parse::<u8>().ok()).is_some_and(|part| (16..=31).contains(&part)));

    if private {
        return Err(format!("`{name}` is this machine or a private network; web_fetch reads public pages only (use run_command with curl for a local server)"));
    }

    Ok(())
}

/// A page as text: its address after redirects, and its readable text. JSON and plain text come back as they are.
pub fn fetch(url: &str) -> Result<(String, String), String> {
    fetch_page(url, PAGE_CAP).map(|page| (page.url, page.text))
}

/// One page, read for a person who will check it (0.16.1): where it ended up, its title, the date it
/// says it was published, and its main text - the article, not the menus around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub date: Option<String>,
    pub text: String,
}

/// `fetch`, keeping the title and the date, with at most `cap` characters of text - a local model's
/// window is smaller than a page.
pub fn fetch_page(url: &str, cap: usize) -> Result<Page, String> {
    /* Redirects are followed by hand, each hop checked: a public page that redirects to 169.254.169.254
       must not become a read of this machine's cloud credentials. */
    let mut current = url.trim().to_string();
    let mut hops = 0;
    let response = loop {
        allowed(&current)?;

        let response = match agent().get(&current).set("accept", "text/html,application/xhtml+xml,application/json,text/plain;q=0.9,*/*;q=0.5").call() {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) => {
                return Err(format!("{current} answered HTTP {code} {}", response.status_text()));
            }
            Err(ureq::Error::Transport(transport)) => return Err(format!("{current} could not be reached: {transport}")),
        };

        if !(300..400).contains(&response.status()) {
            break response;
        }

        let Some(location) = response.header("location").map(str::to_string) else {
            break response;
        };

        hops += 1;

        if hops > 6 {
            return Err(format!("{url} redirects too many times"));
        }

        current = join_url(response.get_url(), &location);
    };
    let content_type = response.header("content-type").unwrap_or("").to_ascii_lowercase();
    let final_url = response.get_url().to_string();
    let mut bytes = Vec::new();

    response.into_reader().take(BYTES_CAP).read_to_end(&mut bytes).map_err(|error| format!("reading {url}: {error}"))?;

    if content_type.starts_with("image/") || content_type.contains("octet-stream") || content_type.contains("pdf") || content_type.contains("zip") {
        return Err(format!("{url} is {content_type}, not a page of text"));
    }

    let body = String::from_utf8_lossy(&bytes).to_string();
    let html = content_type.contains("html") || body.trim_start().starts_with('<');
    let (title, date) = if html { (page_title(&body), page_date(&body)) } else { (String::new(), None) };
    let text = if html { html_to_text(main_content(&body)) } else { body };
    let mut text = text.trim().to_string();

    if text.len() > cap {
        let mut cut = cap;

        while !text.is_char_boundary(cut) {
            cut -= 1;
        }

        text.truncate(cut);
        text.push_str("\n[…the page is longer; only its beginning is shown]");
    }

    Ok(Page { url: final_url, title, date, text })
}

/// The page's `<title>`, decoded.
fn page_title(html: &str) -> String {
    let lowered = html.to_ascii_lowercase();

    lowered
        .find("<title")
        .and_then(|start| lowered[start..].find('>').map(|end| start + end + 1))
        .and_then(|from| lowered[from..].find("</title>").map(|to| &html[from..from + to]))
        .map(|title| decode_entities(title).split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// The date a page says it was published - the article's meta tags, its structured data, or a
/// `<time datetime>` - shortened to the day (`2026-03-14` from `2026-03-14T09:00:00Z`).
pub fn page_date(html: &str) -> Option<String> {
    let lowered = html.to_ascii_lowercase();
    let short = |raw: &str| {
        let raw = raw.trim();

        (raw.len() >= 4 && raw.chars().take(4).all(|c| c.is_ascii_digit())).then(|| raw.chars().take(10).collect::<String>())
    };

    for marker in ["article:published_time", "\"datepublished\" content", "name=\"date\"", "name=\"pubdate\"", "og:updated_time", "article:modified_time"] {
        let Some(at) = lowered.find(marker) else {
            continue;
        };
        let tag_start = lowered[..at].rfind('<').unwrap_or(at);
        let tag_end = lowered[at..].find('>').map(|end| at + end).unwrap_or(lowered.len());

        if let Some(found) = between(&html[tag_start..tag_end], "content=\"", "\"").and_then(short) {
            return Some(found);
        }
    }

    /* JSON-LD: "datePublished": "2026-03-14T…" */
    if let Some(at) = lowered.find("\"datepublished\"") {
        if let Some(found) = html[at + 15..].split('"').nth(1).and_then(short) {
            return Some(found);
        }
    }

    lowered.find("<time").and_then(|at| {
        let tag_end = lowered[at..].find('>').map(|end| at + end).unwrap_or(lowered.len());

        between(&html[at..tag_end], "datetime=\"", "\"").and_then(short)
    })
}

/// The part of a page that is its content: the largest `<article>` or `<main>` when one holds enough
/// text to be the article, else the whole page. Menus, cookie banners and related links stay outside.
pub fn main_content(html: &str) -> &str {
    let lowered = html.to_ascii_lowercase();
    let mut best: Option<(usize, usize)> = None;

    for tag in ["article", "main"] {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");
        let mut from = 0;

        while let Some(at) = lowered[from..].find(&open).map(|offset| from + offset) {
            let after = lowered[at + open.len()..].chars().next();

            from = at + open.len();

            if !matches!(after, Some(' ' | '>' | '\n' | '\t')) {
                continue;
            }

            let Some(end) = lowered[at..].find(&close).map(|offset| at + offset + close.len()) else {
                break;
            };

            if best.is_none_or(|(start, stop)| end - at > stop - start) {
                best = Some((at, end));
            }
        }
    }

    match best {
        Some((start, end)) if html_to_text(&html[start..end]).trim().len() >= 400 => &html[start..end],
        _ => html,
    }
}

/// Search results: title, URL and snippet, from DuckDuckGo's HTML endpoint (no key, no account).
pub fn search(query: &str, limit: usize) -> Result<Vec<Value>, String> {
    let url = format!("https://html.duckduckgo.com/html/?q={}", encode(query));
    let response = agent().get(&url).call().map_err(|error| format!("the search could not be reached: {error}"))?;
    let mut bytes = Vec::new();

    response.into_reader().take(BYTES_CAP).read_to_end(&mut bytes).map_err(|error| error.to_string())?;

    Ok(parse_results(&String::from_utf8_lossy(&bytes), limit))
}

fn parse_results(html: &str, limit: usize) -> Vec<Value> {
    let mut results = Vec::new();

    for block in html.split("class=\"result__a\"").skip(1) {
        if results.len() >= limit {
            break;
        }

        let href = between(block, "href=\"", "\"").unwrap_or_default();
        let title = between(block, ">", "</a>").map(html_to_text).unwrap_or_default();
        let snippet = block
            .split("class=\"result__snippet\"")
            .nth(1)
            .and_then(|rest| between(rest, ">", "</a>"))
            .map(html_to_text)
            .unwrap_or_default();
        let target = href
            .split("uddg=")
            .nth(1)
            .map(|rest| decode(rest.split('&').next().unwrap_or(rest)))
            .unwrap_or_else(|| decode_entities(href));

        if target.starts_with("http") && !target.contains("duckduckgo.com/y.js") {
            results.push(json!({ "title": title.trim(), "url": target, "snippet": snippet.trim() }));
        }
    }

    results
}

/// A redirect's `Location` against the address that sent it.
fn join_url(base: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_string();
    }

    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    let host = rest.split('/').next().unwrap_or(rest);

    if let Some(stripped) = location.strip_prefix("//") {
        format!("{scheme}://{stripped}")
    } else if location.starts_with('/') {
        format!("{scheme}://{host}{location}")
    } else {
        let path = rest.split(['?', '#']).next().unwrap_or(rest);
        let dir = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or(host);

        format!("{scheme}://{dir}/{location}")
    }
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let to = text[from..].find(end)? + from;

    Some(&text[from..to])
}

/// Readable text from HTML: scripts, styles and navigation chrome out, block elements as line breaks,
/// links kept as `text (url)` so the model can follow them.
pub fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len() / 2);
    let lowered = html.to_ascii_lowercase();
    let mut index = 0;
    let bytes = html.as_bytes();

    while index < html.len() {
        if bytes[index] == b'<' {
            let rest = &lowered[index..];

            /* Whole elements whose content is never the page's text. */
            let mut skipped = false;

            for tag in ["script", "style", "noscript", "svg", "head", "nav", "footer", "iframe", "template", "aside", "form"] {
                if rest.starts_with(&format!("<{tag}")) && rest[tag.len() + 1..].starts_with([' ', '>', '\n', '\t', '/']) {
                    let close = format!("</{tag}>");

                    index = match rest.find(&close) {
                        Some(at) => index + at + close.len(),
                        None => html.len(),
                    };
                    skipped = true;
                    break;
                }
            }

            if skipped {
                continue;
            }

            let end = match html[index..].find('>') {
                Some(at) => index + at + 1,
                None => html.len(),
            };
            let tag = &lowered[index..end];
            let name = tag.trim_start_matches(['<', '/']).split([' ', '>', '\n', '\t', '/']).next().unwrap_or_default();

            if matches!(name, "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article" | "pre" | "table" | "ul" | "ol" | "blockquote" | "hr") {
                text.push('\n');

                if name.starts_with('h') && !tag.starts_with("</") && name.len() == 2 {
                    text.push_str(&"#".repeat(name[1..].parse::<usize>().unwrap_or(1)));
                    text.push(' ');
                }

                if name == "li" && !tag.starts_with("</") {
                    text.push_str("- ");
                }
            }

            if name == "a" && tag.starts_with("</") {
                if let Some(href) = pending_href(&mut text) {
                    text.push_str(&format!(" ({href})"));
                }
            } else if name == "a" {
                if let Some(href) = between(&html[index..end], "href=\"", "\"").filter(|href| href.starts_with("http")) {
                    text.push_str(&format!("\u{1}{}\u{2}", decode_entities(href)));
                }
            }

            index = end;
            continue;
        }

        let next = html[index..].find('<').map(|at| index + at).unwrap_or(html.len());

        text.push_str(&decode_entities(&html[index..next]));
        index = next;
    }

    /* Unmatched link markers out, runs of blank space folded. */
    let text = text.replace(['\u{1}', '\u{2}'], "");
    let mut out = String::new();
    let mut blank = 0;

    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");

        if line.is_empty() {
            blank += 1;

            if blank == 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(&line);
            out.push('\n');
        }
    }

    out
}

/// The href stored by the opening `<a>` - taken out of the text so it is written after the link's words.
fn pending_href(text: &mut String) -> Option<String> {
    let start = text.rfind('\u{1}')?;
    let end = text[start..].find('\u{2}')? + start;
    let href = text[start + 1..end].to_string();

    text.replace_range(start..=end, "");

    Some(href)
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
}

pub(crate) fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            b' ' => "+".to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }

        out.push(if bytes[index] == b'+' { b' ' } else { bytes[index] });
        index += 1;
    }

    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_reads_as_text_with_its_links() {
        let html = "<html><head><title>x</title><script>var a=1;</script></head><body><nav>menu</nav>\
                    <h1>Install</h1><p>Run <code>npm i</code> &amp; see <a href=\"https://docs.example.com/a\">the docs</a>.</p>\
                    <ul><li>one</li><li>two</li></ul></body></html>";
        let text = html_to_text(html);

        assert!(text.contains("# Install"), "{text}");
        assert!(text.contains("Run npm i & see the docs (https://docs.example.com/a)."), "{text}");
        assert!(text.contains("- one"), "{text}");
        assert!(!text.contains("var a") && !text.contains("menu"), "{text}");
    }

    #[test]
    fn search_results_are_parsed_from_the_measured_shape() {
        let html = r#"<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fv2.tauri.app%2Fdevelop%2Fsidecar%2F&amp;rut=d9">Embedding External Binaries - Tauri</a>
            <a class="result__snippet" href="//duckduckgo.com/l/?uddg=x">The <b>sidecar&#x27;s</b> child process</a>"#;
        let results = parse_results(html, 5);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["url"], "https://v2.tauri.app/develop/sidecar/");
        assert_eq!(results[0]["title"], "Embedding External Binaries - Tauri");
        assert_eq!(results[0]["snippet"], "The sidecar's child process");
    }

    /// 0.16.1: the article, its title and its date - not the menus and the related links around it.
    #[test]
    fn a_page_gives_its_article_title_and_date() {
        let article = "The release adds streaming tool calls and a context option. ".repeat(10);
        let html = format!(
            "<html><head><title>Ollama 0.9 &amp; tools</title><meta property=\"article:published_time\" content=\"2026-03-14T09:00:00Z\"></head>\
             <body><header>Site menu</header><aside>Related: ten other posts</aside>\
             <article><h1>What is new</h1><p>{article}</p></article><footer>(c) site</footer></body></html>"
        );

        assert_eq!(page_title(&html), "Ollama 0.9 & tools");
        assert_eq!(page_date(&html).as_deref(), Some("2026-03-14"));

        let text = html_to_text(main_content(&html));

        assert!(text.contains("# What is new") && text.contains("streaming tool calls"), "{text}");
        assert!(!text.contains("Related") && !text.contains("Site menu"), "{text}");
        assert_eq!(page_date("<time datetime=\"2025-11-02\">Nov 2</time>").as_deref(), Some("2025-11-02"));
        assert_eq!(page_date("<script type=\"application/ld+json\">{\"datePublished\": \"2024-05-06T10:00\"}</script>").as_deref(), Some("2024-05-06"));
    }

    #[test]
    fn private_addresses_are_refused() {
        for url in ["http://localhost:3000", "http://127.0.0.1/", "http://169.254.169.254/latest/meta-data", "http://192.168.1.4", "http://172.20.0.1", "file:///etc/passwd", "http://[::1]:80/"] {
            assert!(allowed(url).is_err(), "{url}");
        }

        for url in ["https://docs.rs/regex", "http://172.100.1.1/x"] {
            assert!(allowed(url).is_ok(), "{url}");
        }
    }
}
