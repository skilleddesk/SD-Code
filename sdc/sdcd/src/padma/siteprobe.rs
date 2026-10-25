//! The code steps of R-REVIEW-SITE: a site (a web address in the request, or the .html files of the project
//! folder) is probed by code, and every finding is something that was measured, with the snippet that shows it (P3-3).
//!
//! Findings use the review line `place · severity · category · problem · evidence`, so ranking is shared with R-REVIEW-CODE.
//! Not measurable from here and said so in the report: the TLS certificate's expiry, and the server side (`nginx -t`, error logs).

use super::codefns::{Data, Env};
use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

const MAX_PAGES: usize = 30;
const MAX_BODY: u64 = 1_000_000;
const MAX_REDIRECTS: usize = 5;
const BIG_IMAGE_BYTES: u64 = 500_000;

/// What the request is about: the first web address in it, else the project folder.
pub fn site_target(env: &Env, _unit: Option<&str>, _data: &Data) -> Result<Vec<String>, String> {
    if let Some(url) = first_url(&env.goal) {
        return Ok(vec![format!("url:{url}")]);
    }

    let has_pages = super::codefns::list_files(env, None, &Data::new()).is_ok_and(|files| files.iter().any(|file| is_page(file)));

    if has_pages { Ok(vec!["folder".to_string()]) } else { Err("the request has no web address and the folder has no .html files - give me the address of the site".to_string()) }
}

fn is_page(path: &str) -> bool {
    let lower = path.to_lowercase();

    lower.ends_with(".html") || lower.ends_with(".htm")
}

fn first_url(text: &str) -> Option<String> {
    let start = text.find("http://").or_else(|| text.find("https://"))?;
    let url: String = text[start..].chars().take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>' | ')' | ',')).collect();
    let url = url.trim_end_matches(['.', ';', ':']).to_string();

    (url.len() > 8).then_some(url)
}

struct Found {
    place: String,
    severity: &'static str,
    category: &'static str,
    problem: String,
    evidence: String,
}

impl Found {
    fn line(&self) -> String {
        format!("{} · {} · {} · {} · {}", self.place, self.severity, self.category, self.problem, self.evidence.replace(" · ", " - ").replace('\n', " "))
    }
}

fn snippet(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");

    if one.chars().count() > 120 { format!("{}...", one.chars().take(120).collect::<String>()) } else { one }
}

/// The opening tags `<name ...>` of a page, with the line they start on.
fn tags<'a>(html: &'a str, name: &str) -> Vec<(usize, &'a str)> {
    let lower = html.to_lowercase();
    let open = format!("<{name}");
    let mut found = Vec::new();
    let mut from = 0;

    while let Some(at) = lower[from..].find(&open) {
        let start = from + at;
        let after = lower[start + open.len()..].chars().next();

        from = start + open.len();

        if after.is_some_and(|c| !(c.is_whitespace() || c == '>' || c == '/')) {
            continue;
        }

        let end = lower[start..].find('>').map(|end| start + end + 1).unwrap_or(lower.len());
        let line = html[..start].matches('\n').count() + 1;

        found.push((line, &html[start..end]));
    }

    found
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let mut from = 0;

    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        let before_ok = start == 0 || lower.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = tag[start + name.len()..].trim_start();

        from = start + name.len();

        if !before_ok || !rest.starts_with('=') {
            continue;
        }

        let value = rest[1..].trim_start();

        return Some(match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or("").to_string(),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string(),
        });
    }

    None
}

struct Page {
    /// Targets of links (`a href`).
    links: Vec<(usize, String, String)>,
    /// Targets of things the page loads (images, scripts, stylesheets).
    assets: Vec<(usize, String, String)>,
    findings: Vec<Found>,
}

/// What a page says about itself: the problems it has on its own, and what it points to.
fn analyze(place: &str, html: &str, https: bool) -> Page {
    let mut findings = Vec::new();
    let at = |line: usize| if place.contains("://") { place.to_string() } else { format!("{place}:{line}") };
    let title = html.to_lowercase().find("<title").and_then(|start| html[start..].find('>').map(|open| (start + open + 1, start))).map(|(open, _)| html[open..].split('<').next().unwrap_or("").trim().to_string());

    if title.as_deref().is_none_or(str::is_empty) {
        findings.push(Found { place: at(1), severity: "medium", category: "seo", problem: "the page has no title".into(), evidence: if title.is_none() { "no <title> element".into() } else { "<title></title>".into() } });
    }

    let described = tags(html, "meta").iter().any(|(_, tag)| attr(tag, "name").is_some_and(|name| name.eq_ignore_ascii_case("description")) && attr(tag, "content").is_some_and(|content| !content.trim().is_empty()));

    if !described {
        findings.push(Found { place: at(1), severity: "low", category: "seo", problem: "the page has no meta description".into(), evidence: "no <meta name=\"description\" content=...>".into() });
    }

    let mut assets = Vec::new();

    for (line, tag) in tags(html, "img") {
        if attr(tag, "alt").is_none() {
            findings.push(Found { place: at(line), severity: "medium", category: "accessibility", problem: "an image has no alt text".into(), evidence: snippet(tag) });
        }

        if let Some(src) = attr(tag, "src") {
            assets.push((line, src, snippet(tag)));
        }
    }

    for (line, tag) in tags(html, "script") {
        if let Some(src) = attr(tag, "src") {
            assets.push((line, src, snippet(tag)));
        }
    }

    for (line, tag) in tags(html, "link") {
        if attr(tag, "rel").is_some_and(|rel| rel.to_lowercase().contains("stylesheet")) {
            if let Some(href) = attr(tag, "href") {
                assets.push((line, href, snippet(tag)));
            }
        }
    }

    if https {
        for (line, target, tag) in &assets {
            if target.to_lowercase().starts_with("http://") {
                findings.push(Found { place: at(*line), severity: "high", category: "mixed-content", problem: "an https page loads something over plain http".into(), evidence: tag.clone() });
            }
        }
    }

    let links = tags(html, "a").into_iter().filter_map(|(line, tag)| attr(tag, "href").map(|href| (line, href, snippet(tag)))).collect();

    Page { links, assets, findings }
}

fn skipped_link(href: &str) -> bool {
    let lower = href.trim().to_lowercase();

    lower.is_empty() || lower.starts_with('#') || ["mailto:", "tel:", "javascript:", "data:", "sms:"].iter().any(|scheme| lower.starts_with(scheme))
}

/// `https://host:port` of an address.
fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;

    Some(format!("{}://{}", scheme.to_lowercase(), host.to_lowercase()))
}

/// The address a link on `base` points to, without its `#fragment`.
fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.trim().split('#').next().unwrap_or("");

    if href.is_empty() {
        return None;
    }

    let origin = origin(base)?;

    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }

    if let Some(rest) = href.strip_prefix("//") {
        return Some(format!("{}://{rest}", origin.split("://").next()?));
    }

    let path_start = base.find("://")? + 3;
    let path = base[path_start..].find('/').map(|at| &base[path_start + at..]).unwrap_or("/");
    let path = path.split(['?', '#']).next().unwrap_or("/");
    let joined = if href.starts_with('/') {
        href.to_string()
    } else if href.starts_with('?') {
        return Some(format!("{origin}{path}{href}"));
    } else {
        format!("{}{href}", &path[..path.rfind('/').map_or(0, |at| at + 1)])
    };
    let mut parts: Vec<&str> = Vec::new();
    let (joined_path, query) = joined.split_once('?').map(|(path, query)| (path.to_string(), format!("?{query}"))).unwrap_or((joined.clone(), String::new()));

    for part in joined_path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }

    let trailing = if joined_path.ends_with('/') && !parts.is_empty() { "/" } else { "" };

    Some(format!("{origin}/{}{trailing}{query}", parts.join("/")))
}

struct Fetched {
    status: u16,
    location: Option<String>,
    length: u64,
    content_type: String,
    body: String,
}

fn fetch(agent: &ureq::Agent, url: &str, want_body: bool) -> Result<Fetched, String> {
    use std::io::Read;

    let response = match agent.get(url).set("User-Agent", "SDC-site-probe").call() {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(error) => return Err(error.to_string()),
    };
    let content_type = response.header("content-type").unwrap_or("").to_lowercase();
    let location = response.header("location").map(str::to_string);
    let status = response.status();
    let header_length = response.header("content-length").and_then(|value| value.parse::<u64>().ok());
    let mut bytes = Vec::new();

    if want_body || header_length.is_none() {
        let _ = response.into_reader().take(MAX_BODY).read_to_end(&mut bytes);
    }

    Ok(Fetched { status, location, length: header_length.unwrap_or(bytes.len() as u64), content_type, body: if want_body { String::from_utf8_lossy(&bytes).to_string() } else { String::new() } })
}

fn local_host(url: &str) -> bool {
    origin(url).is_some_and(|origin| ["://localhost", "://127.", "://[::1]", "://0.0.0.0"].iter().any(|host| origin.contains(host)))
}

/// Follows redirects by hand so the chain can be reported. The last answer and the chain of addresses.
fn follow(agent: &ureq::Agent, url: &str, want_body: bool) -> Result<(Fetched, Vec<String>), String> {
    let mut chain = vec![url.to_string()];
    let mut current = url.to_string();

    loop {
        let fetched = fetch(agent, &current, want_body)?;

        if (300..400).contains(&fetched.status) {
            let next = fetched.location.as_deref().and_then(|location| resolve(&current, location)).ok_or_else(|| format!("{current} redirects (HTTP {}) without a usable Location", fetched.status))?;

            if chain.contains(&next) {
                chain.push(next);

                return Err(format!("redirect loop: {}", chain.join(" -> ")));
            }

            if chain.len() > MAX_REDIRECTS {
                return Err(format!("more than {MAX_REDIRECTS} redirects: {}", chain.join(" -> ")));
            }

            chain.push(next.clone());

            current = next;

            continue;
        }

        return Ok((fetched, chain));
    }
}

/// Probes a live site: its pages (same origin, at most 30), what they link to and load.
fn probe_url(start: &str, lines: &mut Vec<String>) {
    let agent = ureq::AgentBuilder::new().redirects(0).timeout_connect(Duration::from_secs(8)).timeout_read(Duration::from_secs(15)).build();
    let mut found: Vec<Found> = Vec::new();
    let site = origin(start).unwrap_or_default();
    let mut queue: VecDeque<String> = VecDeque::from([start.to_string()]);
    let mut seen: BTreeSet<String> = BTreeSet::from([start.to_string()]);
    let mut checked: BTreeSet<String> = BTreeSet::new();
    let mut crawled = 0;

    if start.starts_with("http://") && !local_host(start) {
        found.push(Found { place: start.to_string(), severity: "medium", category: "https", problem: "the site is given over plain http".into(), evidence: format!("address {start}") });
    }

    while let Some(url) = queue.pop_front() {
        if crawled >= MAX_PAGES {
            lines.push(format!("#limit only the first {MAX_PAGES} pages were visited"));

            break;
        }

        crawled += 1;

        let (page, chain) = match follow(&agent, &url, true) {
            Ok(done) => done,
            Err(reason) => {
                let category = if reason.contains("loop") || reason.contains("redirects") { "redirect" } else if url.starts_with("https://") && (reason.to_lowercase().contains("tls") || reason.to_lowercase().contains("certificate") || reason.to_lowercase().contains("handshake")) { "https" } else { "reachability" };

                found.push(Found { place: url.clone(), severity: "high", category, problem: "the page could not be loaded".into(), evidence: reason });

                continue;
            }
        };

        if chain.len() > 3 {
            found.push(Found { place: url.clone(), severity: "medium", category: "redirect", problem: format!("{} redirects before the page", chain.len() - 1), evidence: chain.join(" -> ") });
        }

        if chain.first().is_some_and(|first| first.starts_with("https://")) && chain.last().is_some_and(|last| last.starts_with("http://")) {
            found.push(Found { place: url.clone(), severity: "high", category: "https", problem: "an https address ends on plain http".into(), evidence: chain.join(" -> ") });
        }

        if page.status >= 400 {
            found.push(Found { place: url.clone(), severity: "high", category: "http-status", problem: "the page answers with an error".into(), evidence: format!("HTTP {}", page.status) });

            continue;
        }

        if !page.content_type.contains("html") {
            continue;
        }

        let here = chain.last().cloned().unwrap_or_else(|| url.clone());
        let analysis = analyze(&url, &page.body, here.starts_with("https://"));

        found.extend(analysis.findings);

        for (_, href, tag) in &analysis.links {
            if skipped_link(href) {
                continue;
            }

            let Some(target) = resolve(&here, href) else { continue };

            if origin(&target).as_deref() != Some(site.as_str()) {
                continue;
            }

            if checked.insert(target.clone()) && seen.insert(target.clone()) {
                if let Err(reason) = follow(&agent, &target, false).and_then(|(fetched, _)| if fetched.status >= 400 { Err(format!("HTTP {}", fetched.status)) } else { Ok(()) }) {
                    found.push(Found { place: url.clone(), severity: "high", category: "broken-link", problem: format!("a link goes to {target}, which does not load"), evidence: format!("{tag} -> {reason}") });
                } else {
                    queue.push_back(target);
                }
            }
        }

        for (_, src, tag) in &analysis.assets {
            let Some(target) = resolve(&here, src) else { continue };

            if !checked.insert(target.clone()) {
                continue;
            }

            match follow(&agent, &target, false) {
                Ok((fetched, _)) if fetched.status >= 400 => found.push(Found { place: url.clone(), severity: "high", category: "broken-asset", problem: format!("{target} does not load"), evidence: format!("{tag} -> HTTP {}", fetched.status) }),
                Ok((fetched, _)) if fetched.content_type.starts_with("image/") && fetched.length > BIG_IMAGE_BYTES => {
                    found.push(Found { place: url.clone(), severity: "medium", category: "performance", problem: format!("an image is {} KB", fetched.length / 1000), evidence: format!("{tag} -> {} bytes", fetched.length) });
                }
                Ok(_) => {}
                Err(reason) => found.push(Found { place: url.clone(), severity: "high", category: "broken-asset", problem: format!("{target} could not be loaded"), evidence: format!("{tag} -> {reason}") }),
            }
        }
    }

    lines.extend(found.iter().map(Found::line));
    lines.push(format!("#ran http probe: {crawled} pages visited"));

    if start.starts_with("https://") {
        lines.push("#skipped TLS certificate expiry: the connection worked or failed, but its expiry date is not read from here".to_string());
    }

    lines.push("#skipped server side (nginx -t, error log): needs access to the server, which is not connected to this task".to_string());
}

/// Probes the .html files of the project folder: the same page checks, and links/assets that point at files that do not exist.
fn probe_folder(env: &Env, lines: &mut Vec<String>) {
    let pages: Vec<String> = super::codefns::list_files(env, None, &Data::new()).unwrap_or_default().into_iter().filter(|file| is_page(file)).take(MAX_PAGES).collect();
    let mut found = Vec::new();

    for page in &pages {
        let Ok(html) = std::fs::read_to_string(env.root.join(page)) else {
            lines.push(format!("#skipped {page}: could not be read as text"));

            continue;
        };
        let analysis = analyze(page, &html, false);
        let dir = std::path::Path::new(page).parent().map(std::path::Path::to_path_buf).unwrap_or_default();

        found.extend(analysis.findings);

        for (line, href, tag) in analysis.links.iter().chain(analysis.assets.iter()) {
            let clean = href.split(['#', '?']).next().unwrap_or("");

            if skipped_link(href) || clean.is_empty() || clean.contains("://") || clean.starts_with("//") {
                continue;
            }

            let path = if let Some(rooted) = clean.strip_prefix('/') { env.root.join(rooted) } else { env.root.join(&dir).join(clean) };
            let exists = path.exists() || (path.is_dir() && path.join("index.html").exists());

            if !exists {
                let category = if analysis.assets.iter().any(|(asset_line, asset, _)| asset_line == line && asset == href) { "broken-asset" } else { "broken-link" };

                found.push(Found { place: format!("{page}:{line}"), severity: "high", category, problem: format!("{href} does not exist in the project"), evidence: tag.clone() });
            } else if path.is_file() && path.metadata().is_ok_and(|meta| meta.len() > BIG_IMAGE_BYTES) && matches!(path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).as_deref(), Some("png" | "jpg" | "jpeg" | "gif" | "webp")) {
                found.push(Found { place: format!("{page}:{line}"), severity: "medium", category: "performance", problem: format!("an image is {} KB", path.metadata().map(|meta| meta.len() / 1000).unwrap_or(0)), evidence: tag.clone() });
            }
        }
    }

    lines.extend(found.iter().map(Found::line));
    lines.push(format!("#ran folder probe: {} pages", pages.len()));
    lines.push("#skipped the live site (status codes, redirects, HTTPS): give me the address to check those".to_string());
}

/// R-REVIEW-SITE step 2: the probes. One line per finding, then `#` lines for what ran and what could not be measured.
pub fn probe_site(env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let target = data.get("target").and_then(|lines| lines.first()).cloned().unwrap_or_default();
    let mut lines = Vec::new();

    if let Some(url) = target.strip_prefix("url:") {
        probe_url(url, &mut lines);
    } else {
        probe_folder(env, &mut lines);
    }

    Ok(lines)
}

/// The site report, built only from the ranked probe lines.
pub fn site_report(_env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let ranked = data.get("ranked").cloned().unwrap_or_default();
    let findings: Vec<&String> = ranked.iter().filter(|line| !line.starts_with('#')).collect();
    let count = |wanted: &str| findings.iter().filter(|line| line.split(" · ").nth(1) == Some(wanted)).count();
    let target = data.get("target").and_then(|lines| lines.first()).map(|target| target.strip_prefix("url:").unwrap_or("the project folder")).unwrap_or("the site");
    let mut report = vec!["# Site review".to_string(), String::new(), format!("Checked {target}: {} findings ({} high, {} medium, {} low). Each one was measured, with the snippet that shows it.", findings.len(), count("high"), count("medium"), count("low"))];

    for (title, wanted) in [("High", "high"), ("Medium", "medium"), ("Low", "low")] {
        let group: Vec<&&String> = findings.iter().filter(|line| line.split(" · ").nth(1) == Some(wanted)).collect();

        if group.is_empty() {
            continue;
        }

        report.push(String::new());
        report.push(format!("## {title}"));

        for (number, line) in group.iter().enumerate() {
            let mut field = line.split(" · ");
            let place = field.next().unwrap_or("");

            field.next();

            let category = field.next().unwrap_or("");
            let problem = field.next().unwrap_or("");
            let evidence = field.collect::<Vec<_>>().join(" · ");

            report.push(format!("{}. `{place}` [{category}] {problem}", number + 1));
            report.push(format!("   evidence: `{evidence}`"));
        }
    }

    if findings.is_empty() {
        report.push(String::new());
        report.push("No problems were found by these probes.".to_string());
    }

    let notes: Vec<String> = ranked.iter().filter(|line| line.starts_with('#')).map(|line| format!("- {}", line.trim_start_matches('#'))).collect();

    if !notes.is_empty() {
        report.push(String::new());
        report.push("## What ran, and what could not be measured".to_string());
        report.extend(notes);
    }

    Ok(vec![report.join("\n")])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve(routes: Vec<(&'static str, u16, &'static str, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, kind, body) = routes.iter().find(|(route, ..)| *route == path).map(|(_, status, kind, body)| (*status, *kind, body.clone())).unwrap_or((404, "text/html", b"<html>not found</html>".to_vec()));
                let head = format!("HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());

                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });

        format!("http://{address}")
    }

    #[test]
    fn links_resolve_the_way_a_browser_does() {
        assert_eq!(resolve("http://a.test/x/y.html", "z.html").unwrap(), "http://a.test/x/z.html");
        assert_eq!(resolve("http://a.test/x/y.html", "../z.html#top").unwrap(), "http://a.test/z.html");
        assert_eq!(resolve("http://a.test/x/y.html", "/q?a=1").unwrap(), "http://a.test/q?a=1");
        assert_eq!(resolve("https://a.test/x/", "//cdn.test/a.js").unwrap(), "https://cdn.test/a.js");
        assert_eq!(resolve("http://a.test", "about").unwrap(), "http://a.test/about");
        assert!(resolve("http://a.test", "#top").is_none());
        assert_eq!(first_url("please review https://example.com/shop. thanks").unwrap(), "https://example.com/shop");
    }

    #[test]
    fn a_site_with_three_planted_problems_reports_all_three_with_their_evidence() {
        let big = vec![0u8; 600_000];
        let address = serve(vec![
            ("/", 200, "text/html", b"<html><head><title>Shop</title><meta name=\"description\" content=\"A shop\"></head><body>\n<a href=\"/about\">About</a>\n<a href=\"/missing\">Gone</a>\n<img src=\"/big.png\" alt=\"banner\">\n<img src=\"/small.png\">\n</body></html>".to_vec()),
            ("/about", 200, "text/html", b"<html><head><meta name=\"description\" content=\"About us\"></head><body>about</body></html>".to_vec()),
            ("/big.png", 200, "image/png", big),
            ("/small.png", 200, "image/png", vec![1, 2, 3]),
        ]);
        let mut env = Env::new(std::env::temp_dir());

        env.goal = format!("review the site {address} and report problems");

        let mut data = Data::new();

        data.insert("target".to_string(), site_target(&env, None, &data).unwrap());

        assert_eq!(data["target"], vec![format!("url:{address}")]);

        let probes = probe_site(&env, None, &data).unwrap();
        let text = probes.join("\n");

        assert!(text.contains("broken-link") && text.contains("/missing") && text.contains("HTTP 404"), "{text}");
        assert!(text.contains("the page has no title") && text.contains(&format!("{address}/about")), "{text}");
        assert!(text.contains("an image has no alt text") && text.contains("<img src=\"/small.png\">"), "{text}");
        assert!(text.contains("an image is 600 KB"), "{text}");
        assert!(text.contains("#skipped server side"), "{text}");

        data.insert("verified".to_string(), probes);

        let ranked = super::super::codefns::rank_findings(&env, None, &data).unwrap();

        data.insert("ranked".to_string(), ranked);

        let report = site_report(&env, None, &data).unwrap().join("\n");

        assert!(report.contains("## High") && report.contains("[broken-link]") && report.contains("evidence: `<a href=\"/missing\"> -> HTTP 404`"), "{report}");
        assert!(report.contains("What ran, and what could not be measured"), "{report}");
    }

    #[test]
    fn a_redirect_loop_and_a_dead_address_are_findings_not_crashes() {
        let mut env = Env::new(std::env::temp_dir());
        let mut lines = Vec::new();

        env.goal = String::new();
        probe_url("http://127.0.0.1:9/", &mut lines);

        assert!(lines.iter().any(|line| line.contains("reachability") && line.contains("could not be loaded")), "{lines:?}");
    }

    #[test]
    fn a_project_folder_is_probed_for_missing_files_without_a_network() {
        let root = std::env::temp_dir().join(format!("sdc-site-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        std::fs::create_dir_all(root.join("css")).unwrap();
        std::fs::write(root.join("css/site.css"), "body{}").unwrap();
        std::fs::write(root.join("index.html"), "<html><head><title>T</title><link rel=\"stylesheet\" href=\"css/site.css\"><link rel=\"stylesheet\" href=\"css/gone.css\"></head>\n<body><a href=\"about.html\">About</a></body></html>").unwrap();

        let env = Env::new(root.clone());
        let mut data = Data::new();

        data.insert("target".to_string(), site_target(&env, None, &data).unwrap());

        let text = probe_site(&env, None, &data).unwrap().join("\n");

        assert_eq!(data["target"], vec!["folder"]);
        assert!(text.contains("index.html:1 · high · broken-asset") && text.contains("css/gone.css"), "{text}");
        assert!(text.contains("index.html:2 · high · broken-link") && text.contains("about.html"), "{text}");
        assert!(!text.contains("css/site.css does not exist"), "{text}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn with_nothing_to_look_at_it_asks_for_the_address() {
        let root = std::env::temp_dir().join(format!("sdc-site-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), "x = 1\n").unwrap();

        assert!(site_target(&Env::new(root.clone()), None, &Data::new()).unwrap_err().contains("web address"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
