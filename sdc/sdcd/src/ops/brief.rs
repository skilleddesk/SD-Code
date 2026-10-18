//! **The morning brief and the weekly client report (Phase 4.8).**
//!
//! * The *brief* is one page for the person, across every site and project: what is down, which certificates are about to expire,
//!   which disks are filling, which backups are old, what the dependency checks last said, what the agent did in the last 24 hours,
//!   and what AI cost yesterday (with the cost's kind - measured, priced or estimate - never merged). Everything is read from what SDC
//!   already recorded; it makes no new network calls and invents no field. A site SDC has not checked recently is said to be
//!   unchecked, not "up".
//! * The *client report* is for someone else: the week's work from the Proof Packs - what was asked, what changed, whether it was
//!   verified, and whether the audit ledger is intact. It leaves out cost, models and prompts' internals. It is a self-contained HTML
//!   page (a link or a file to attach; "Print → Save as PDF" makes the PDF - a PDF written here could not show Bangla or Arabic text
//!   without bundling fonts).

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

use crate::sdcp::events::StoredEvent;
use crate::store::sqlite::Store;

/// A certificate with this many days left (or fewer) is called out.
pub const SSL_SOON_DAYS: i64 = 14;
/// A disk this full (percent) is called out.
pub const DISK_HIGH_PERCENT: i64 = 85;
/// A newest backup older than this many hours is called out.
pub const BACKUP_OLD_HOURS: f64 = 48.0;
/// A site whose last check is older than this many hours is "unchecked".
pub const STALE_HOURS: i64 = 24;

/// What one site's last health report says, as flags a person can act on.
pub fn site_flags(report: &Value, checked_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Vec<&'static str> {
    let mut flags = Vec::new();

    match checked_at {
        None => flags.push("unchecked"),
        Some(at) if now - at > Duration::hours(STALE_HOURS) => flags.push("stale"),
        _ => {}
    }

    if report["http"]["ok"] == false {
        flags.push("down");
    }

    if let Some(days) = report["ssl"]["days"].as_i64() {
        if days < 0 {
            flags.push("ssl-expired");
        } else if days <= SSL_SOON_DAYS {
            flags.push("ssl-soon");
        }
    }

    if report["disk"]["percent"].as_i64().is_some_and(|percent| percent >= DISK_HIGH_PERCENT) {
        flags.push("disk-high");
    }

    if report["backup"]["ageHours"].as_f64().is_some_and(|hours| hours > BACKUP_OLD_HOURS) || (report["backup"]["configured"] == true && report["backup"]["newest"].is_null() && !report["backup"].is_null()) {
        flags.push("backup-old");
    }

    flags
}

fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text).ok().map(|time| time.with_timezone(&Utc))
}

/// The last `ship.check` results SDC kept, per project folder: `{ root: { at, verdict, checks: [{id, status, summary}] } }`.
pub const SHIP_SETTING: &str = "ship.last";

/// Remembers a Ship Safe report's headline for the brief (no details, no values).
pub fn remember_ship(store: &Store, root: &str, report: &Value) {
    let mut all: Value = store.setting(SHIP_SETTING).ok().flatten().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_else(|| json!({}));

    all[root] = json!({
        "at": Utc::now().to_rfc3339(),
        "verdict": report["verdict"],
        "checks": report["checks"].as_array().into_iter().flatten().map(|check| json!({ "id": check["id"], "status": check["status"], "summary": check["summary"] })).collect::<Vec<_>>(),
    });

    let _ = store.set_setting(SHIP_SETTING, &all.to_string());
}

/// The morning brief as data.
pub fn morning(store: &Store, now: DateTime<Utc>) -> Value {
    let mut sites = Vec::new();

    for site in store.sites().unwrap_or_default() {
        let id = site["id"].as_str().unwrap_or_default();
        let latest = store.health_history(id, 1).unwrap_or_default().into_iter().next();
        let checked_at = latest.as_ref().and_then(|report| report["ts"].as_str()).and_then(parse_time);
        let report = latest.unwrap_or(Value::Null);
        let flags = site_flags(&report, checked_at, now);

        sites.push(json!({
            "id": id,
            "name": site["name"],
            "url": site["url"],
            "checkedAt": checked_at.map(|at| at.to_rfc3339()),
            "up": report["http"]["ok"],
            "status": report["http"]["status"],
            "sslDays": report["ssl"]["days"],
            "diskPercent": report["disk"]["percent"],
            "backupAgeHours": report["backup"]["ageHours"],
            "lastDeploy": report["deploy"]["state"],
            "flags": flags,
        }));
    }

    let dependencies: Vec<Value> = store
        .setting(SHIP_SETTING)
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|all| all.as_object().cloned())
        .map(|all| {
            all.iter()
                .flat_map(|(root, entry)| {
                    entry["checks"].as_array().into_iter().flatten().filter(|check| matches!(check["id"].as_str(), Some("audit" | "existence")) && matches!(check["status"].as_str(), Some("fail" | "warn"))).map(move |check| json!({ "project": root, "check": check["id"], "status": check["status"], "summary": check["summary"], "at": entry["at"] }))
                })
                .collect()
        })
        .unwrap_or_default();

    let since = now - Duration::hours(24);
    let rows = store.audit_rows(None, None, 5000).unwrap_or_default();
    let recent: Vec<_> = rows.iter().filter(|row| parse_time(&row.ts).is_some_and(|at| at >= since)).collect();
    let count = |kind: &str| recent.iter().filter(|row| row.kind == kind).count();
    let yesterday = (now - Duration::days(1)).format("%Y-%m-%d").to_string();
    let usage = store.usage_rows(Some(&yesterday)).unwrap_or_default();
    let cost = crate::trust::cost::by_source(&usage, &yesterday);
    let yesterday_cost: Vec<Value> = cost["today"].as_array().cloned().unwrap_or_default();

    let flagged = sites.iter().filter(|site| site["flags"].as_array().is_some_and(|flags| flags.iter().any(|flag| flag != "stale" && flag != "unchecked"))).count();

    json!({
        "generatedAt": now.to_rfc3339(),
        "sites": sites,
        "dependencies": dependencies,
        "agent": {
            "sinceHours": 24,
            "turnsStarted": count("TurnStarted"),
            "turnsCompleted": count("TurnCompleted"),
            "filesChanged": count("FileRestored") + recent.iter().filter(|row| row.kind == "ToolCallStarted" && row.summary.to_lowercase().starts_with("edit")).count(),
            "errors": count("ErrorRaised"),
            "rewinds": count("RewindApplied"),
            "policyViolations": count("PolicyViolation"),
            "deploys": count("DeployUpdated"),
        },
        "yesterdayCost": { "day": yesterday, "bySource": yesterday_cost },
        "attention": flagged + dependencies.len(),
    })
}

/// The one line the notification carries.
pub fn headline(brief: &Value, lang: &str) -> String {
    let down = brief["sites"].as_array().into_iter().flatten().filter(|site| site["flags"].as_array().is_some_and(|flags| flags.iter().any(|flag| flag == "down"))).count();
    let ssl = brief["sites"].as_array().into_iter().flatten().filter(|site| site["flags"].as_array().is_some_and(|flags| flags.iter().any(|flag| flag == "ssl-soon" || flag == "ssl-expired"))).count();
    let attention = brief["attention"].as_u64().unwrap_or(0);

    match lang.split('-').next().unwrap_or("en") {
        "bn" if attention == 0 => "সকালের রিপোর্ট: সবকিছু ঠিক আছে।".to_string(),
        "bn" => format!("সকালের রিপোর্ট: {down}টা সাইট বন্ধ, {ssl}টার SSL শেষ হচ্ছে, মোট {attention}টা বিষয় দেখার মতো।"),
        _ if attention == 0 => "Morning brief: nothing needs your attention.".to_string(),
        _ => format!("Morning brief: {down} site(s) down, {ssl} certificate(s) expiring, {attention} thing(s) need a look."),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The brief as a one-page HTML document.
pub fn brief_html(brief: &Value, lang: &str) -> String {
    let bn = lang.starts_with("bn");
    let t = |en: &'static str, bn_text: &'static str| if bn { bn_text } else { en };
    let mut rows = String::new();

    for site in brief["sites"].as_array().into_iter().flatten() {
        let flags: Vec<&str> = site["flags"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();

        rows.push_str(&format!(
            "<tr class=\"{}\"><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            if flags.iter().any(|flag| matches!(*flag, "down" | "ssl-expired")) { "bad" } else if flags.is_empty() { "ok" } else { "warn" },
            escape(site["name"].as_str().unwrap_or_default()),
            match site["up"].as_bool() { Some(true) => t("up", "চালু"), Some(false) => t("DOWN", "বন্ধ"), None => t("not checked", "পরীক্ষা হয়নি") },
            site["sslDays"].as_i64().map_or("-".to_string(), |days| format!("{days} d")),
            site["diskPercent"].as_i64().map_or("-".to_string(), |percent| format!("{percent}%")),
            site["backupAgeHours"].as_f64().map_or("-".to_string(), |hours| format!("{hours} h")),
            escape(&flags.join(", ")),
        ));
    }

    let dependencies: String = brief["dependencies"].as_array().into_iter().flatten().map(|entry| format!("<li>{} - {}</li>", escape(entry["project"].as_str().unwrap_or_default()), escape(entry["summary"].as_str().unwrap_or_default()))).collect();
    let cost: String = brief["yesterdayCost"]["bySource"].as_array().into_iter().flatten().filter(|row| row["turns"].as_u64().unwrap_or(0) > 0).map(|row| format!("<li>{}: ${:.4} ({} turn(s))</li>", escape(row["kind"].as_str().unwrap_or_default()), row["usd"].as_f64().unwrap_or(0.0), row["turns"])).collect();
    let agent = &brief["agent"];

    format!(
        "<!doctype html><html lang=\"{lang}\"{dir}><head><meta charset=\"utf-8\"><title>{title}</title><style>body{{font:14px system-ui;max-width:860px;margin:24px auto;padding:0 16px;color:#222}}table{{border-collapse:collapse;width:100%}}td,th{{border-bottom:1px solid #ddd;padding:6px 8px;text-align:left}}.bad td{{color:#b00020;font-weight:600}}.warn td{{color:#8a5a00}}h2{{margin-top:28px}}</style></head><body><h1>{title}</h1><p>{headline}</p><h2>{sites}</h2><table><tr><th>{site}</th><th>{status}</th><th>SSL</th><th>{disk}</th><th>{backup}</th><th>{flags}</th></tr>{rows}</table><h2>{deps}</h2><ul>{dependencies}</ul><h2>{did}</h2><p>{turns} / {done} / {errors}</p><h2>{costh}</h2><ul>{cost}</ul><p style=\"color:#777\">{foot}</p></body></html>",
        lang = escape(lang),
        dir = if lang.starts_with("ar") || lang.starts_with("ur") { " dir=\"rtl\"" } else { "" },
        title = t("SDC morning brief", "SDC সকালের রিপোর্ট"),
        headline = escape(&headline(brief, lang)),
        sites = t("Sites", "সাইট"),
        site = t("Site", "সাইট"),
        status = t("Status", "অবস্থা"),
        disk = t("Disk", "ডিস্ক"),
        backup = t("Newest backup", "সর্বশেষ ব্যাকআপ"),
        flags = t("Needs a look", "দেখার বিষয়"),
        rows = rows,
        deps = t("Dependency warnings (last Ship Safe run)", "ডিপেন্ডেন্সি সতর্কতা (সর্বশেষ Ship Safe)"),
        dependencies = if dependencies.is_empty() { format!("<li>{}</li>", t("none recorded", "কিছু নেই")) } else { dependencies },
        did = t("What the agent did in the last 24 hours", "গত ২৪ ঘণ্টায় এজেন্ট কী করেছে"),
        turns = format!("{} {}", agent["turnsStarted"], t("turns started", "টার্ন শুরু")),
        done = format!("{} {}", agent["turnsCompleted"], t("completed", "শেষ")),
        errors = format!("{} {}", agent["errors"], t("errors", "ত্রুটি")),
        costh = t("Yesterday's AI cost", "গতকালের AI খরচ"),
        cost = if cost.is_empty() { format!("<li>{}</li>", t("nothing recorded", "কিছু নেই")) } else { cost },
        foot = t("Built from what SDC recorded; nothing here was guessed. A site not checked in 24 hours is shown as stale or unchecked, never as up.", "SDC যা রেকর্ড করেছে তা থেকে তৈরি; কিছুই আন্দাজ নয়। ২৪ ঘণ্টায় পরীক্ষা না হওয়া সাইট 'চালু' নয়, 'পুরনো/অপরীক্ষিত' দেখায়।"),
    )
}

/// The weekly client report as data: the turns that started in the last `days`, per chat, from the Proof Packs.
pub fn client_report(store: &Store, log: &[StoredEvent], now: DateTime<Utc>, days: i64) -> Value {
    let since = now - Duration::days(days);
    let mut projects: Vec<Value> = Vec::new();

    for host in store.hosts().unwrap_or_default() {
        let host_id = host["id"].as_str().unwrap_or_default().to_string();

        for session in store.sessions_for_host(&host_id).unwrap_or_default() {
            let id = session["sessionId"].as_str().unwrap_or_default();
            let pack = crate::trust::proof::build(store, log, id, None);
            let turns: Vec<Value> = pack["turns"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|turn| turn["startedAt"].as_str().and_then(parse_time).is_some_and(|at| at >= since))
                .map(|turn| {
                    json!({
                        "request": turn["prompt"],
                        "summary": turn["answer"].as_str().map(|answer| answer.lines().next().unwrap_or_default().chars().take(240).collect::<String>()),
                        "files": turn["files"].as_array().into_iter().flatten().map(|file| file["path"].clone()).collect::<Vec<_>>(),
                        "verified": turn["verify"]["pass"],
                        "verdict": turn["verify"]["verdict"],
                        "at": turn["startedAt"],
                    })
                })
                .collect();

            if !turns.is_empty() {
                projects.push(json!({ "title": session["title"], "projectRoot": pack["session"]["projectRoot"], "turns": turns, "ledger": pack["ledger"] }));
            }
        }
    }

    json!({ "format": "sdc-client-report/1", "from": since.format("%Y-%m-%d").to_string(), "to": now.format("%Y-%m-%d").to_string(), "projects": projects, "sdcd": crate::VERSION })
}

/// The client report as a self-contained page. No cost, no model names, no internal ids.
pub fn client_html(report: &Value, lang: &str) -> String {
    let bn = lang.starts_with("bn");
    let t = |en: &'static str, bn_text: &'static str| if bn { bn_text } else { en };
    let mut body = String::new();

    for project in report["projects"].as_array().into_iter().flatten() {
        body.push_str(&format!("<h2>{}</h2>", escape(project["title"].as_str().unwrap_or_default())));

        for turn in project["turns"].as_array().into_iter().flatten() {
            let verified = match turn["verified"].as_bool() {
                Some(true) => t("verified", "যাচাই হয়েছে"),
                Some(false) => t("checks did not pass", "যাচাই পাস হয়নি"),
                None => t("not verified", "যাচাই হয়নি"),
            };
            let files: Vec<String> = turn["files"].as_array().into_iter().flatten().filter_map(Value::as_str).map(escape).collect();

            body.push_str(&format!(
                "<div class=\"turn\"><b>{}</b><br>{}<br><small>{} · {}{}</small></div>",
                escape(turn["request"].as_str().unwrap_or_default().chars().take(300).collect::<String>().as_str()),
                escape(turn["summary"].as_str().unwrap_or_default()),
                verified,
                escape(turn["at"].as_str().unwrap_or_default().get(..10).unwrap_or_default()),
                if files.is_empty() { String::new() } else { format!(" · {}: {}", t("files", "ফাইল"), files.join(", ")) },
            ));
        }

        body.push_str(&format!("<p><small>{}: {}</small></p>", t("Audit ledger", "অডিট লেজার"), if project["ledger"]["intact"] == true { t("intact", "অক্ষত") } else { t("see the Proof Pack", "Proof Pack দেখুন") }));
    }

    if body.is_empty() {
        body = format!("<p>{}</p>", t("No work was recorded this week.", "এই সপ্তাহে কোনো কাজ রেকর্ড হয়নি।"));
    }

    format!(
        "<!doctype html><html lang=\"{lang}\"{dir}><head><meta charset=\"utf-8\"><title>{title}</title><style>body{{font:15px system-ui;max-width:780px;margin:28px auto;padding:0 16px;color:#222}}.turn{{border-left:3px solid #4c8dff;padding:6px 12px;margin:10px 0;background:#f7f9ff}}small{{color:#666}}@media print{{.turn{{break-inside:avoid}}}}</style></head><body><h1>{title}</h1><p>{from} - {to}</p>{body}<p><small>{foot}</small></p></body></html>",
        lang = escape(lang),
        dir = if lang.starts_with("ar") || lang.starts_with("ur") { " dir=\"rtl\"" } else { "" },
        title = t("Weekly work report", "সাপ্তাহিক কাজের রিপোর্ট"),
        from = escape(report["from"].as_str().unwrap_or_default()),
        to = escape(report["to"].as_str().unwrap_or_default()),
        foot = t("Generated by SDC from its own record of the work (Proof Packs). Use Print → Save as PDF to make a PDF.", "SDC তার নিজের কাজের রেকর্ড (Proof Pack) থেকে বানিয়েছে। PDF করতে Print → Save as PDF ব্যবহার করুন।"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        parse_time("2026-10-18T06:00:00Z").unwrap()
    }

    #[test]
    fn a_site_is_flagged_for_down_an_expiring_certificate_a_full_disk_and_an_old_backup() {
        let report = json!({ "http": { "ok": false }, "ssl": { "days": 9 }, "disk": { "percent": 91 }, "backup": { "configured": true, "ageHours": 72.0, "newest": "b.tgz" } });
        let flags = site_flags(&report, Some(now() - Duration::hours(1)), now());

        for expected in ["down", "ssl-soon", "disk-high", "backup-old"] {
            assert!(flags.contains(&expected), "{expected} in {flags:?}");
        }

        assert!(site_flags(&json!({ "http": { "ok": true }, "ssl": { "days": 80 }, "disk": { "percent": 40 }, "backup": { "ageHours": 3.0 } }), Some(now()), now()).is_empty());
    }

    #[test]
    fn a_site_nobody_checked_is_never_called_up() {
        assert!(site_flags(&Value::Null, None, now()).contains(&"unchecked"));
        assert!(site_flags(&json!({ "http": { "ok": true } }), Some(now() - Duration::hours(30)), now()).contains(&"stale"));
        assert!(site_flags(&json!({ "ssl": { "days": -2 } }), Some(now()), now()).contains(&"ssl-expired"));
    }

    #[test]
    fn the_brief_reads_the_stores_sites_cost_and_ship_results_and_has_a_one_line_headline() {
        let store = Store::in_memory().unwrap();

        store.upsert_site("s1", "Shop", "local", "/srv/shop", "https://shop.example", &json!({})).unwrap();
        store.save_health("s1", &json!({ "http": { "ok": false, "status": 502 }, "ssl": { "days": 5 } })).unwrap();
        remember_ship(&store, "/work/shop", &json!({ "verdict": "fail", "checks": [{ "id": "audit", "status": "fail", "summary": "2 critical" }, { "id": "headers", "status": "fail", "summary": "x" }] }));

        let brief = morning(&store, Utc::now());

        assert_eq!(brief["sites"][0]["name"], "Shop");
        assert!(brief["sites"][0]["flags"].as_array().unwrap().iter().any(|flag| flag == "down"));
        assert_eq!(brief["dependencies"].as_array().unwrap().len(), 1, "only the dependency checks, not the headers");
        assert!(brief["attention"].as_u64().unwrap() >= 2);
        assert!(headline(&brief, "en").starts_with("Morning brief: 1 site(s) down"));
        assert!(headline(&brief, "bn").contains("সাইট বন্ধ"));

        let page = brief_html(&brief, "en");

        assert!(page.contains("Shop") && page.contains("DOWN") && page.contains("2 critical"));
        assert!(brief_html(&brief, "bn").contains("সকালের রিপোর্ট"));
    }

    #[test]
    fn a_quiet_morning_says_so() {
        let store = Store::in_memory().unwrap();
        let brief = morning(&store, Utc::now());

        assert_eq!(brief["attention"], 0);
        assert_eq!(headline(&brief, "en"), "Morning brief: nothing needs your attention.");
    }

    #[test]
    fn the_client_report_has_the_weeks_work_and_no_cost_models_or_ids() {
        let store = Store::in_memory().unwrap();
        let report = client_report(&store, &[], Utc::now(), 7);

        assert_eq!(report["projects"], json!([]));

        let page = client_html(&json!({ "from": "2026-10-11", "to": "2026-10-18", "projects": [{ "title": "Shop", "turns": [{ "request": "fix <login>", "summary": "Fixed it", "files": ["src/a.js"], "verified": true, "at": "2026-10-15T10:00:00Z" }], "ledger": { "intact": true } }] }), "en");

        assert!(page.contains("Fixed it") && page.contains("verified") && page.contains("src/a.js") && page.contains("intact"));
        assert!(page.contains("fix &lt;login&gt;"), "text is escaped");
        assert!(!page.to_lowercase().contains("cost") && !page.contains("model"));
        assert!(client_html(&report, "bn").contains("কোনো কাজ রেকর্ড হয়নি"));
    }
}
