//! **Health Watch**: every site, on its own interval - is it up and how fast, how many days its SSL
//! certificate has left, how full its disk is, how old its newest backup is, how many error lines its
//! log has grown, and how its last deploy ended. Each report becomes an Agency Ops Score with reasons
//! (`trust::score::ops`), and a change a person should hear about becomes an alert in their language.
//!
//! Structured signals only (P3): an HTTP status, an exit code, a date, a byte count. An error *count* in
//! a log is a signal to look at, never a verdict of success.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{Runner, Site};
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;
use crate::DaemonState;

/// One HTTP check: the status, the time, and whether it is what the site should answer.
pub fn http(url: &str, expect_status: u16, expect_text: Option<&str>) -> Value {
    let started = Instant::now();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(20))
        .redirects(5)
        .user_agent(&format!("SDC-HealthWatch/{}", crate::VERSION))
        .build();
    let response = agent.get(url).call();
    let ms = started.elapsed().as_millis() as u64;

    let (status, body) = match response {
        Ok(response) => {
            let status = response.status();

            (status, response.into_string().unwrap_or_default())
        }
        Err(ureq::Error::Status(status, response)) => (status, response.into_string().unwrap_or_default()),
        Err(error) => {
            return json!({ "ok": false, "status": Value::Null, "ms": ms, "detail": format!("not reachable: {error}") });
        }
    };

    let status_ok = status == expect_status;
    let text_ok = expect_text.is_none_or(|text| text.is_empty() || body.contains(text));
    let detail = match (status_ok, text_ok) {
        (true, true) => format!("HTTP {status} in {ms} ms"),
        (false, _) => format!("HTTP {status} (expected {expect_status})"),
        (true, false) => format!("HTTP {status}, but the page does not contain \"{}\"", expect_text.unwrap_or_default()),
    };

    json!({ "ok": status_ok && text_ok, "status": status, "ms": ms, "detail": detail })
}

/// The host part of a URL, for the certificate check.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split(['/', '?', '#']).next()?.split('@').next_back()?;
    let host = host.split(':').next()?;

    (!host.is_empty()).then(|| host.to_string())
}

/// `notAfter=Sep 30 12:00:00 2026 GMT` → days from now (negative once expired).
pub fn days_until(not_after: &str) -> Option<i64> {
    let text = not_after.trim().trim_start_matches("notAfter=").trim().trim_end_matches(" GMT").trim();
    let normal = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let parsed = chrono::NaiveDateTime::parse_from_str(&normal, "%b %d %H:%M:%S %Y").ok()?;

    Some((parsed.and_utc() - chrono::Utc::now()).num_days())
}

/// The whole report for one site. Everything that cannot be measured says so rather than guessing.
pub fn check(state: &Arc<DaemonState>, site: &Site, remote: Option<crate::ssh::Ssh>) -> Value {
    let url = site.health_url();
    let expect_status = site.config["health"]["expectStatus"].as_u64().unwrap_or(200) as u16;
    let expect_text = site.config["health"]["expectText"].as_str();
    let http = if url.trim().is_empty() { json!({ "ok": Value::Null, "detail": "no URL set" }) } else { http(&url, expect_status, expect_text) };
    let runner = Runner::new(remote, &site.root);
    let backup_dir = super::deploy::dir_expr(&site.backup_dir());
    let log = site.config["health"]["logFile"].as_str().filter(|path| !path.trim().is_empty()).map(super::quote);
    let https_host = if url.starts_with("https://") { host_of(&url) } else { None };
    let mut report = json!({
        "http": http,
        "ssl": Value::Null,
        "disk": Value::Null,
        "backup": { "configured": true },
        "errors": Value::Null,
    });

    if runner.posix() {
        let ssl = https_host
            .as_ref()
            .map(|host| format!("echo \"ssl:$(echo | openssl s_client -servername {h} -connect {h}:443 2>/dev/null | openssl x509 -noout -enddate 2>/dev/null)\";", h = super::quote(host)))
            .unwrap_or_default();
        let errors = log
            .as_ref()
            .map(|log| format!("[ -r {log} ] && echo \"errors:$(tail -n 2000 {log} | grep -ciE 'error|fatal|critical')\";"))
            .unwrap_or_default();
        let script = format!(
            "{ssl} df -P . 2>/dev/null | tail -1 | awk '{{print \"disk:\"$5\":\"$4}}'; \
             newest=\"$(ls -t {backup_dir} 2>/dev/null | head -1)\"; [ -n \"$newest\" ] && echo \"backup:$newest:$(( $(date +%s) - $(stat -c %Y {backup_dir}/\"$newest\" 2>/dev/null || stat -f %m {backup_dir}/\"$newest\") ))\"; \
             {errors} true"
        );

        if let Ok(ran) = runner.script(&script, Duration::from_secs(40)) {
            for line in ran.stdout.lines() {
                if let Some(rest) = line.strip_prefix("ssl:") {
                    report["ssl"] = match days_until(rest) {
                        Some(days) => json!({ "days": days, "expires": rest.trim().trim_start_matches("notAfter=") }),
                        None => json!({ "days": Value::Null, "detail": "could not read the certificate" }),
                    };
                } else if let Some(rest) = line.strip_prefix("disk:") {
                    let mut parts = rest.split(':');
                    let percent = parts.next().and_then(|value| value.trim_end_matches('%').parse::<i64>().ok());
                    let free_kb = parts.next().and_then(|value| value.trim().parse::<i64>().ok());

                    report["disk"] = json!({ "percent": percent, "freeMb": free_kb.map(|kb| kb / 1024) });
                } else if let Some(rest) = line.strip_prefix("backup:") {
                    let mut parts = rest.rsplitn(2, ':');
                    let seconds = parts.next().and_then(|value| value.trim().parse::<f64>().ok());
                    let name = parts.next().unwrap_or_default();

                    report["backup"] = json!({ "configured": true, "newest": name, "ageHours": seconds.map(|seconds| (seconds / 3600.0 * 10.0).round() / 10.0) });
                } else if let Some(rest) = line.strip_prefix("errors:") {
                    report["errors"] = json!({ "count": rest.trim().parse::<u64>().unwrap_or(0), "file": site.config["health"]["logFile"] });
                }
            }
        }

        if report["backup"].get("ageHours").is_none() {
            report["backup"] = json!({ "configured": true, "newest": Value::Null, "ageHours": Value::Null, "detail": "no backup found yet" });
        }
    }

    let deploys = state.store.deploys(Some(&site.id), 1).unwrap_or_default();
    let last_deploy = deploys.first().cloned();

    report["deploy"] = last_deploy
        .as_ref()
        .map(|deploy| json!({ "id": deploy["id"], "state": deploy["state"], "at": deploy["startedAt"] }))
        .unwrap_or(Value::Null);

    report
}

/// Alerts, in the person's language (Settings → Language → reply language, `ui.language`).
pub fn sentence(lang: &str, key: &str, name: &str, value: &str) -> String {
    let bn = matches!(lang.split('-').next(), Some("bn"));
    let hi = matches!(lang.split('-').next(), Some("hi"));
    let ar = matches!(lang.split('-').next(), Some("ar"));
    let es = matches!(lang.split('-').next(), Some("es"));

    match key {
        "down" if bn => format!("{name} সাইটটা এখন কাজ করছে না ({value})।"),
        "down" if hi => format!("{name} साइट अभी काम नहीं कर रही ({value})।"),
        "down" if ar => format!("الموقع {name} لا يعمل الآن ({value})."),
        "down" if es => format!("El sitio {name} no responde ahora ({value})."),
        "down" => format!("{name} is down ({value})."),
        "up" if bn => format!("{name} আবার ঠিকমতো চলছে।"),
        "up" if hi => format!("{name} फिर से ठीक चल रही है।"),
        "up" if ar => format!("عاد الموقع {name} للعمل."),
        "up" if es => format!("{name} vuelve a funcionar."),
        "up" => format!("{name} is back up."),
        "ssl" if bn => format!("{name}-এর SSL সার্টিফিকেট {value} দিনের মধ্যে শেষ হবে।"),
        "ssl" if hi => format!("{name} का SSL प्रमाणपत्र {value} दिनों में खत्म होगा।"),
        "ssl" if ar => format!("شهادة SSL للموقع {name} تنتهي خلال {value} يومًا."),
        "ssl" if es => format!("El certificado SSL de {name} vence en {value} días."),
        "ssl" => format!("{name}'s SSL certificate expires in {value} days."),
        "disk" if bn => format!("{name}-এর ডিস্ক {value}% ভরে গেছে।"),
        "disk" if hi => format!("{name} की डिस्क {value}% भर चुकी है।"),
        "disk" if ar => format!("قرص {name} ممتلئ بنسبة {value}%."),
        "disk" if es => format!("El disco de {name} está al {value}%."),
        "disk" => format!("{name}'s disk is {value}% full."),
        "backup" if bn => format!("{name}-এর সবচেয়ে নতুন ব্যাকআপ {value} ঘণ্টা পুরনো।"),
        "backup" if hi => format!("{name} का सबसे नया बैकअप {value} घंटे पुराना है।"),
        "backup" if ar => format!("أحدث نسخة احتياطية للموقع {name} عمرها {value} ساعة."),
        "backup" if es => format!("La copia de seguridad más reciente de {name} tiene {value} horas."),
        "backup" => format!("{name}'s newest backup is {value} hours old."),
        _ => format!("{name}: {value}"),
    }
}

/// What changed between two reports that a person should hear about: `(level, key, value)`.
pub fn alerts(previous: Option<&Value>, report: &Value) -> Vec<(&'static str, &'static str, String)> {
    let mut found = Vec::new();
    let was_up = previous.and_then(|previous| previous["http"]["ok"].as_bool());
    let is_up = report["http"]["ok"].as_bool();

    match (was_up, is_up) {
        (Some(true) | None, Some(false)) => found.push(("critical", "down", report["http"]["detail"].as_str().unwrap_or("down").to_string())),
        (Some(false), Some(true)) => found.push(("info", "up", String::new())),
        _ => {}
    }

    let crossed = |pointer: &str, limit: f64, above: bool| -> Option<f64> {
        let now = report.pointer(pointer).and_then(Value::as_f64)?;
        let before = previous.and_then(|previous| previous.pointer(pointer)).and_then(Value::as_f64);
        let over = |value: f64| if above { value >= limit } else { value <= limit };

        (over(now) && !before.is_some_and(over)).then_some(now)
    };

    if let Some(days) = crossed("/ssl/days", 14.0, false) {
        found.push(("warning", "ssl", format!("{days:.0}")));
    }

    if let Some(percent) = crossed("/disk/percent", 90.0, true) {
        found.push(("warning", "disk", format!("{percent:.0}")));
    }

    if let Some(hours) = crossed("/backup/ageHours", 48.0, true) {
        found.push(("warning", "backup", format!("{hours:.0}")));
    }

    found
}

/// Checks one site, scores it, stores the report, pushes `HealthUpdated` and any alerts. Returns the report.
pub fn run_once(state: &Arc<DaemonState>, out: &dyn Notifier, site: &Site, remote: Option<crate::ssh::Ssh>) -> Value {
    let previous = state.store.health_history(&site.id, 1).ok().and_then(|rows| rows.into_iter().next());
    let mut report = check(state, site, remote);
    let violations = state
        .store
        .audit_rows(None, None, 2000)
        .unwrap_or_default()
        .iter()
        .filter(|row| row.kind == "PolicyViolation" && row.ts.as_str() >= (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339().as_str())
        .count();
    let (score, level, reasons) = crate::trust::score::ops(&report, report["deploy"]["state"].as_str(), violations);

    report["score"] = json!({ "score": score, "level": level, "reasons": reasons });

    let _ = state.store.save_health(&site.id, &report);

    out.push(
        event::health_updated(json!({ "siteId": site.id, "name": site.name, "report": report, "score": score, "level": level, "reasons": reasons })),
        None,
        None,
    );

    let lang = state.store.setting("ui.language").ok().flatten().unwrap_or_else(|| "en".into());

    for (alert_level, key, value) in alerts(previous.as_ref(), &report) {
        out.push(event::health_alert(&site.id, &site.name, alert_level, &sentence(&lang, key, &site.name, &value)), None, None);
    }

    report
}

/// The watcher: every minute, each site whose interval has passed is checked on the blocking pool. The
/// Night Guardian (`guardian`) reads each new report.
pub fn spawn(state: Arc<DaemonState>, out: Arc<dyn Notifier>) {
    tokio::spawn(async move {
        let mut last: std::collections::HashMap<String, Instant> = Default::default();

        /* After the window's first burst of requests. */
        tokio::time::sleep(Duration::from_secs(45)).await;

        loop {
            for row in state.store.sites().unwrap_or_default() {
                let Some(site) = Site::from_row(&row) else {
                    continue;
                };

                if site.config["health"]["enabled"].as_bool() == Some(false) {
                    continue;
                }

                let interval = Duration::from_secs(site.config["health"]["interval"].as_u64().unwrap_or(300).max(60));

                if last.get(&site.id).is_some_and(|at| at.elapsed() < interval) {
                    continue;
                }

                last.insert(site.id.clone(), Instant::now());

                let remote = super::remote_for_host(&state, &site.host_id);
                let (state, out) = (state.clone(), out.clone());

                let _ = tokio::task::spawn_blocking(move || {
                    let report = run_once(&state, &*out, &site, remote.clone());

                    super::guardian::observe(&state, &out, &site, remote, &report);
                })
                .await;
            }

            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_certificate_date_becomes_days() {
        let future = (chrono::Utc::now() + chrono::Duration::days(30)).format("%b %e %H:%M:%S %Y GMT").to_string();

        assert!((29..=30).contains(&days_until(&format!("notAfter={future}")).unwrap()));
        assert!(days_until("garbage").is_none());
    }

    #[test]
    fn hosts_are_read_from_urls() {
        assert_eq!(host_of("https://shop.test:8443/path?q=1").as_deref(), Some("shop.test"));
        assert_eq!(host_of("http://user@example.com/").as_deref(), Some("example.com"));
        assert!(host_of("not a url").is_none());
    }

    #[test]
    fn an_alert_fires_once_when_a_line_is_crossed() {
        let up = json!({ "http": { "ok": true }, "ssl": { "days": 40 }, "disk": { "percent": 50 } });
        let down = json!({ "http": { "ok": false, "detail": "HTTP 502" }, "ssl": { "days": 10 }, "disk": { "percent": 93 } });

        let first = alerts(Some(&up), &down);

        assert_eq!(first.iter().map(|(_, key, _)| *key).collect::<Vec<_>>(), ["down", "ssl", "disk"]);
        assert!(alerts(Some(&down), &down).is_empty(), "the same state again is not a new alert");
        assert_eq!(alerts(Some(&down), &up)[0].1, "up");
    }

    #[test]
    fn alerts_speak_the_persons_language() {
        assert!(sentence("bn", "down", "Shop", "HTTP 502").contains("কাজ করছে না"));
        assert!(sentence("ar", "ssl", "Shop", "5").contains("SSL"));
        assert_eq!(sentence("en", "disk", "Shop", "92"), "Shop's disk is 92% full.");
    }

    #[test]
    fn a_local_http_check_reads_the_status_and_the_text() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                use std::io::{Read, Write};

                let (mut socket, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 1024];
                let _ = socket.read(&mut buffer);
                let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nWelcome, shop");
            }
        });

        assert_eq!(http(&url, 200, Some("Welcome"))["ok"], true);
        assert_eq!(http(&url, 200, Some("Checkout"))["ok"], false);
        server.join().unwrap();
    }
}
