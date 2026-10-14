//! **A site's own terms** (v3 G37, switch `v3.terms`, on): SDC reads the web for the person, and a site is
//! entitled to say what may be read. Before a page is fetched, the site's `robots.txt` is read (once per host, kept
//! for the turn) and a path it disallows is refused - with the rule quoted, so the person can see whose word it
//! was. A site that says nothing, or cannot be reached, is not treated as forbidden: the absence of a rule is not a
//! rule.
//!
//! W8's research turns fetch many pages from many hosts; they go through the same door, and a site asking for a
//! crawl delay is given one.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// One `User-agent` section of the file: who it names, its rules (`true` = Disallow), and its crawl delay.
type Section = (Vec<String>, Vec<(bool, String)>, Option<Duration>);

/// What a site's file said about SDC.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Robots {
    /// The rules that match SDC's user agent, in the order the file lists them.
    rules: Vec<(bool, String)>,
    /// What the site asks of whoever follows it (`Crawl-delay`).
    pub delay: Option<Duration>,
}

/// The user agent SDC reads as, and the names a file may name SDC by.
pub const AGENT: &str = "SDC";

impl Robots {
    /// Reads the file. A section naming SDC is the one that counts (the standard's "most specific group wins");
    /// with no such section, the `*` sections are SDC's. Another agent's section is not SDC's business.
    pub fn parse(text: &str) -> Self {
        /* Sections in the order the file lists them: who they name, their rules, their delay. */
        let mut sections: Vec<Section> = Vec::new();
        let mut agents: Vec<String> = Vec::new();

        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();

            if line.is_empty() {
                continue;
            }

            let Some((field, value)) = line.split_once(':') else {
                continue;
            };
            let (field, value) = (field.trim().to_ascii_lowercase(), value.trim());

            match field.as_str() {
                /* A `User-agent` line starts a section, or joins the one just before it. */
                "user-agent" => {
                    if sections.last().is_some_and(|(named, rules, delay)| !named.is_empty() || !rules.is_empty() || delay.is_some()) {
                        agents = Vec::new();
                    }

                    agents.push(value.to_string());
                    sections.push((agents.clone(), Vec::new(), None));
                }
                /* An empty value asserts nothing: `Disallow:` on its own means "read what you like". */
                "disallow" if !value.is_empty() => {
                    if let Some(section) = sections.last_mut() {
                        section.1.push((true, value.to_string()));
                    }
                }
                "allow" if !value.is_empty() => {
                    if let Some(section) = sections.last_mut() {
                        section.1.push((false, value.to_string()));
                    }
                }
                "crawl-delay" => {
                    if let Some(section) = sections.last_mut() {
                        section.2 = value.parse::<f32>().ok().filter(|seconds| *seconds > 0.0).map(Duration::from_secs_f32);
                    }
                }
                _ => {}
            }
        }

        let named = |section: &Section| section.0.iter().any(|agent| agent.eq_ignore_ascii_case(AGENT));
        let any_for_sdc = sections.iter().any(named);
        /* The wildcard's rules apply to SDC as well: a site that disallows a path for everyone means it. SDC's own
           section comes after them, so it can add a rule or take one back with `Allow`. */
        let wildcard: Vec<&Section> = sections.iter().filter(|section| section.0.iter().any(|agent| agent == "*")).collect();
        let ours: Vec<&Section> = sections.iter().filter(|section| any_for_sdc && named(section)).collect();
        let all: Vec<&Section> = wildcard.into_iter().chain(ours.iter().copied()).collect();

        Robots {
            rules: all.iter().flat_map(|section| section.1.clone()).collect(),
            delay: ours.iter().find_map(|section| section.2).or_else(|| all.iter().find_map(|section| section.2)),
        }
    }

    /// The rule that decides this path - the longest matching one, and a tie goes to `Allow`, which is what the
    /// standard says. Answers the rule itself, so the refusal can quote it.
    pub fn rule_for(&self, path: &str) -> Option<(bool, String)> {
        let path = if path.is_empty() { "/" } else { path };

        self.rules
            .iter()
            .filter(|(_, pattern)| path.starts_with(pattern.trim_end_matches('*')))
            .max_by_key(|(disallow, pattern)| (pattern.trim_end_matches('*').len(), *disallow))
            .filter(|(disallow, _)| *disallow)
            .cloned()
    }

    /// May SDC read this path?
    pub fn allows(&self, path: &str) -> bool {
        self.rule_for(path).is_none()
    }
}

/// The path (and query) of a URL, which is what a robots rule decides on.
pub fn path_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let after_host = rest.find('/').map(|slash| &rest[slash..]).unwrap_or("/");
    let without_fragment = after_host.split('#').next().unwrap_or(after_host);

    if without_fragment.is_empty() { "/".to_string() } else { without_fragment.to_string() }
}

/// The origin (`scheme://host`) of a URL.
pub fn origin_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);

    match rest.find('/') {
        Some(slash) => format!("{}{}", &url[..url.len() - rest.len()], &rest[..slash]),
        None => url.trim_end_matches('/').to_string(),
    }
}

fn cache() -> &'static Mutex<HashMap<String, Robots>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Robots>>> = OnceLock::new();

    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The site's file, read once per host. `reader` fetches it (so a test need not use the network).
pub fn robots_for(origin: &str, reader: &dyn Fn(&str) -> Result<String, String>) -> Robots {
    if let Ok(cache) = cache().lock() {
        if let Some(known) = cache.get(origin) {
            return known.clone();
        }
    }

    /* A site with no file, or one that cannot be reached, is not silenced - silence is not a rule. */
    let robots = reader(&format!("{origin}/robots.txt")).map(|text| Robots::parse(&text)).unwrap_or_default();

    if let Ok(mut cache) = cache().lock() {
        cache.insert(origin.to_string(), robots.clone());
    }

    robots
}

/// Forgets what was read about a host (the daemon starts a new session, or a test wants a clean sheet).
pub fn forget(origin: Option<&str>) {
    if let Ok(mut cache) = cache().lock() {
        match origin {
            Some(origin) => {
                cache.remove(origin);
            }
            None => cache.clear(),
        }
    }
}

/// May SDC read this URL? `Err` carries the refusal, in the site's own words.
pub fn allows(url: &str, reader: &dyn Fn(&str) -> Result<String, String>) -> Result<(), String> {
    let origin = origin_of(url);
    let path = path_of(url);

    match robots_for(&origin, reader).rule_for(&path) {
        Some((_, pattern)) => Err(format!(
            "{origin}/robots.txt does not allow {AGENT} to read {path} (`Disallow: {pattern}`). SDC left the page alone - ask the person whether they want it read from a browser themselves, or look for another source."
        )),
        None => Ok(()),
    }
}

/// How long to wait between pages of one host, when the site asked for it.
pub fn delay_for(origin: &str, reader: &dyn Fn(&str) -> Result<String, String>) -> Option<Duration> {
    robots_for(origin, reader).delay
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# a site's own file\nUser-agent: *\nDisallow: /private/\nDisallow: /admin\nAllow: /private/readme\n\nUser-agent: SomeBot\nDisallow: /somebot-only\n\nUser-agent: SDC\nCrawl-delay: 2\nDisallow: /api/\n";

    #[test]
    fn the_rules_that_are_sdcs_business_are_the_wildcard_and_sdcs_own() {
        let robots = Robots::parse(FILE);

        assert!(!robots.allows("/private/secret"), "the wildcard's rule applies to SDC");
        assert!(!robots.allows("/private/secret"), "and SDC's own section does not take it back");
        assert!(robots.allows("/private/readme"), "the longer Allow wins, which is what the standard says");
        assert!(robots.allows("/blog/post"), "no rule, no refusal");
        assert!(!robots.allows("/api/orders"), "SDC's own section is the more specific one");
        assert!(!robots.allows("/admin"), "the wildcard's rule is SDC's too - it means everyone");
        assert!(robots.allows("/somebot-only"), "another agent's rule is not SDC's");
        assert_eq!(robots.delay, Some(Duration::from_secs(2)), "the delay SDC's own section asks for");

        assert!(Robots::default().allows("/anything"), "a site with no file is not silenced");
        assert_eq!(Robots::parse("User-agent: *\nDisallow:").rule_for("/x"), None, "an empty Disallow means nothing");
        assert!(!Robots::parse("User-agent: *\nDisallow: /").allows("/"), "a bare slash means everything");
    }

    #[test]
    fn a_url_is_split_into_its_origin_and_its_path() {
        assert_eq!(origin_of("https://docs.example/a/b?c=1"), "https://docs.example");
        assert_eq!(origin_of("https://docs.example"), "https://docs.example");
        assert_eq!(path_of("https://docs.example/a/b?c=1"), "/a/b?c=1");
        assert_eq!(path_of("https://docs.example"), "/");
        assert_eq!(path_of("https://docs.example/#top"), "/");
    }

    #[test]
    fn a_disallowed_page_is_refused_with_the_sites_own_words_and_read_once_per_host() {
        let reads = std::sync::atomic::AtomicUsize::new(0);

        forget(None);

        let reader = |url: &str| {
            reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (url == "https://docs.example/robots.txt").then(|| FILE.to_string()).ok_or_else(|| "not found".to_string())
        };

        let refusal = allows("https://docs.example/private/secret", &reader).expect_err("disallowed");

        assert!(refusal.contains("Disallow: /private/") && refusal.contains("robots.txt"), "{refusal}");
        assert!(allows("https://docs.example/guide", &reader).is_ok());
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1, "the file is read once per host");
        assert_eq!(delay_for("https://docs.example", &reader), Some(Duration::from_secs(2)));

        forget(Some("https://docs.example"));
        assert_eq!(allows("https://docs.example/private/secret", &reader), Err(refusal));
    }
}
