//! **Ship Safe (Phase 4.5)**: one press, five checks before something goes live - secrets, access, security headers, vulnerable dependencies,
//! and dependencies that do not exist (a package an AI invented is a package an attacker can register: "slopsquatting").
//!
//! Every check answers in plain words and says what to do: `pass`, `warn` or `fail`, or `skipped` with the reason (a tool that is not
//! installed, no site address given). A check that could not run is **never** reported as passed. Each problem carries a fix: either a
//! ready prompt for the agent ("Fix with a prompt", the same pattern as Verify) or, for secrets, the sweep's move-to-keychain.
//!
//! Network use is stated: the headers and exposure checks ask the site address the person gave; the existence check asks the public
//! package registries (npm, PyPI, crates.io) for each dependency's *name* - never a file, never a key.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use super::sweep;

/// Where the package registries are asked (injectable for tests).
#[derive(Clone)]
pub struct Registries {
    pub npm: String,
    pub pypi: String,
    pub crates: String,
}

impl Default for Registries {
    fn default() -> Self {
        Self { npm: "https://registry.npmjs.org".into(), pypi: "https://pypi.org/pypi".into(), crates: "https://crates.io/api/v1/crates".into() }
    }
}

fn check(id: &str, title: &str, status: &str, summary: impl Into<String>, details: Vec<String>, fix: Option<Value>) -> Value {
    json!({ "id": id, "title": title, "status": status, "summary": summary.into(), "details": details, "fix": fix })
}

fn prompt_fix(label: &str, prompt: impl Into<String>) -> Value {
    json!({ "kind": "prompt", "label": label, "prompt": prompt.into() })
}

/// The five checks. `site` is the address of the live (or staging) site, when there is one.
pub fn run(root: &Path, site: Option<&str>, registries: &Registries) -> Value {
    let checks = vec![secrets(root), access(root, site), headers(site), audit(root), existence(root, registries)];
    let worst = if checks.iter().any(|c| c["status"] == "fail") {
        "fail"
    } else if checks.iter().any(|c| c["status"] == "warn" || c["status"] == "skipped") {
        "warn"
    } else {
        "pass"
    };

    json!({ "verdict": worst, "checks": checks, "site": site })
}

/// 1. Secrets already in the folder (the first-open sweep's own scan).
pub fn secrets(root: &Path) -> Value {
    let sweep = sweep::run(root);
    let found: Vec<&sweep::Finding> = sweep.findings.iter().filter(|finding| !finding.by_name).collect();
    let named = sweep.findings.len() - found.len();

    if found.is_empty() {
        return check(
            "secrets",
            "Secrets in your files",
            "pass",
            format!("No password, key or token was found in {} files{}.", sweep.files, if named > 0 { format!(" ({named} file names look sensitive - SDC asks before an AI reads them)") } else { String::new() }),
            Vec::new(),
            None,
        );
    }

    check(
        "secrets",
        "Secrets in your files",
        "fail",
        format!("{} place(s) hold something that looks like a secret. Anything committed or deployed with it should be treated as leaked.", found.len()),
        found.iter().take(12).map(|finding| format!("{}:{} - {}", finding.file, finding.line, finding.what)).collect(),
        Some(json!({ "kind": "secrets", "label": "Move them to the keychain" })),
    )
}

/// Paths a web server must never serve, and the text that proves it did.
const EXPOSED: &[(&str, &str)] = &[("/.env", "="), ("/.git/config", "[core]"), ("/wp-config.php.bak", "DB_"), ("/.htpasswd", ":"), ("/phpinfo.php", "phpinfo"), ("/backup.sql", "INSERT INTO")];

type Fetched = (u16, Vec<(String, String)>, String);

fn http_get(url: &str, limit: u64) -> Result<Fetched, String> {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(6)).timeout_read(Duration::from_secs(8)).redirects(0).build();

    match agent.get(url).set("user-agent", "SDC-ShipSafe/3.1").call() {
        Ok(response) | Err(ureq::Error::Status(_, response)) => {
            let status = response.status();
            let headers = response.headers_names().into_iter().filter_map(|name| response.header(&name).map(|value| (name.to_ascii_lowercase(), value.to_string()))).collect();
            let mut body = String::new();

            std::io::Read::read_to_string(&mut std::io::Read::take(response.into_reader(), limit), &mut body).ok();

            Ok((status, headers, body))
        }
        Err(error) => Err(error.to_string()),
    }
}

/// 2. Access: secret files readable by every user on this computer, a web folder that holds secrets, and - with a site address - files
///    the site serves that it must not.
pub fn access(root: &Path, site: Option<&str>) -> Value {
    let mut problems: Vec<String> = Vec::new();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for name in [".env", ".env.local", ".env.production", "id_rsa", "id_ed25519"] {
            if let Ok(meta) = std::fs::metadata(root.join(name)) {
                if meta.permissions().mode() & 0o004 != 0 {
                    problems.push(format!("{name} can be read by every user of this computer (mode {:o}) - run: chmod 600 {name}", meta.permissions().mode() & 0o777));
                }
            }
        }
    }

    for served in ["public", "www", "htdocs", "public_html", "dist", "build"] {
        for secret in [".env", ".env.production", ".git", "wp-config.php.bak", "backup.sql"] {
            if root.join(served).join(secret).exists() {
                problems.push(format!("{served}/{secret} sits in a folder that a web server serves - move it out of {served}/"));
            }
        }
    }

    let mut online_checked = false;

    if let Some(site) = site.map(|site| site.trim_end_matches('/')).filter(|site| site.starts_with("http")) {
        online_checked = true;

        for (path, proof) in EXPOSED {
            if let Ok((200, _, body)) = http_get(&format!("{site}{path}"), 4_096) {
                if body.contains(proof) {
                    problems.push(format!("{site}{path} is served to anyone on the internet and looks real - remove it from the web root now"));
                }
            }
        }
    }

    if problems.is_empty() {
        return check(
            "access",
            "Who can reach your files",
            "pass",
            if online_checked { "Secret files are not readable by other users here, and the site does not serve the usual leaked files.".to_string() } else { "Secret files are not readable by other users here. (No site address was given, so what the site serves was not checked.)".to_string() },
            Vec::new(),
            None,
        );
    }

    check(
        "access",
        "Who can reach your files",
        "fail",
        format!("{} access problem(s) found.", problems.len()),
        problems.clone(),
        Some(prompt_fix("Fix with a prompt", format!("Fix these access problems in this project, one by one, and tell me what you changed:\n- {}", problems.join("\n- ")))),
    )
}

/// The headers a site should send, with the line that sets each one.
const HEADERS: &[(&str, &str, &str)] = &[
    ("content-security-policy", "Content-Security-Policy", "default-src 'self'"),
    ("strict-transport-security", "Strict-Transport-Security", "max-age=31536000; includeSubDomains"),
    ("x-content-type-options", "X-Content-Type-Options", "nosniff"),
    ("referrer-policy", "Referrer-Policy", "strict-origin-when-cross-origin"),
    ("permissions-policy", "Permissions-Policy", "camera=(), microphone=(), geolocation=()"),
];

/// 3. Security headers of the site.
pub fn headers(site: Option<&str>) -> Value {
    let Some(site) = site.map(str::trim).filter(|site| site.starts_with("http")) else {
        return check("headers", "Security headers", "skipped", "No site address was given - add the address of your staging or live site to check its headers.", Vec::new(), None);
    };

    let (status, got, _) = match http_get(site, 1) {
        Ok(answer) => answer,
        Err(error) => return check("headers", "Security headers", "skipped", format!("The site could not be reached ({error}), so its headers were not checked."), Vec::new(), None),
    };
    let has = |name: &str| got.iter().any(|(header, _)| header == name);
    let mut missing: Vec<String> = Vec::new();

    for (key, label, value) in HEADERS {
        if *key == "strict-transport-security" && !site.starts_with("https://") {
            missing.push("The site is not served over https:// at all - get a certificate first (Strict-Transport-Security only works over https)".to_string());
        } else if !has(key) {
            missing.push(format!("{label} is missing - add `{label}: {value}`"));
        }
    }

    if !has("x-frame-options") && !got.iter().any(|(header, value)| header == "content-security-policy" && value.contains("frame-ancestors")) {
        missing.push("Nothing stops other sites framing yours (clickjacking) - add `X-Frame-Options: SAMEORIGIN` or a CSP frame-ancestors rule".to_string());
    }

    let mut notes: Vec<String> = Vec::new();

    for (header, value) in &got {
        if (header == "x-powered-by" || header == "server") && value.chars().any(|c| c.is_ascii_digit()) {
            notes.push(format!("{header}: {value} tells attackers your software version - hide it"));
        }
    }

    if missing.is_empty() && notes.is_empty() {
        return check("headers", "Security headers", "pass", format!("{site} (HTTP {status}) sends all the headers SDC checks for."), Vec::new(), None);
    }

    let all: Vec<String> = missing.iter().chain(notes.iter()).cloned().collect();

    check(
        "headers",
        "Security headers",
        if missing.len() >= 3 { "fail" } else { "warn" },
        format!("{} header problem(s) on {site} (HTTP {status}).", all.len()),
        all.clone(),
        Some(prompt_fix("Fix with a prompt", format!("Add these security headers to {site}'s web server configuration in this project (nginx, Apache, or the framework's middleware), without breaking the site:\n- {}", all.join("\n- ")))),
    )
}

fn run_json(program: &str, args: &[&str], root: &Path) -> Option<(bool, String)> {
    let mut command = crate::host::program::command(program)?;

    command.args(args).current_dir(root);

    let output = command.output().ok()?;

    Some((output.status.success(), String::from_utf8_lossy(&output.stdout).to_string()))
}

/// Counts by severity from `npm audit --json` (v7+ shape, `metadata.vulnerabilities`).
pub fn parse_npm_audit(text: &str) -> Option<(u64, u64, u64, u64)> {
    let value: Value = serde_json::from_str(text.trim()).ok()?;
    let v = value.pointer("/metadata/vulnerabilities")?;
    let n = |key: &str| v[key].as_u64().unwrap_or(0);

    Some((n("critical"), n("high"), n("moderate"), n("low")))
}

/// 4. Known vulnerabilities in the dependencies, from the package manager's own audit.
pub fn audit(root: &Path) -> Value {
    let mut ran: Vec<String> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    if root.join("package-lock.json").exists() {
        match run_json("npm", &["audit", "--json"], root) {
            Some((_, out)) => match parse_npm_audit(&out) {
                Some((critical, high, moderate, low)) => {
                    ran.push("npm audit".into());

                    if critical + high + moderate + low > 0 {
                        problems.push(format!("npm: {critical} critical, {high} high, {moderate} moderate, {low} low"));
                    }
                }
                None => skipped.push("npm audit answered something unreadable (offline?)".into()),
            },
            None => skipped.push("npm is not installed".into()),
        }
    } else if root.join("pnpm-lock.yaml").exists() {
        match run_json("pnpm", &["audit", "--json"], root) {
            Some((_, out)) => match parse_npm_audit(&out) {
                Some((critical, high, moderate, low)) => {
                    ran.push("pnpm audit".into());

                    if critical + high + moderate + low > 0 {
                        problems.push(format!("pnpm: {critical} critical, {high} high, {moderate} moderate, {low} low"));
                    }
                }
                None => skipped.push("pnpm audit answered something unreadable (offline?)".into()),
            },
            None => skipped.push("pnpm is not installed".into()),
        }
    }

    if root.join("Cargo.lock").exists() {
        match run_json("cargo", &["audit", "--json"], root) {
            Some((_, out)) if out.trim_start().starts_with('{') => {
                ran.push("cargo audit".into());

                let count = serde_json::from_str::<Value>(&out).ok().and_then(|value| value.pointer("/vulnerabilities/count").and_then(Value::as_u64)).unwrap_or(0);

                if count > 0 {
                    problems.push(format!("cargo: {count} known vulnerabilit{}", if count == 1 { "y" } else { "ies" }));
                }
            }
            _ => skipped.push("cargo-audit is not installed (cargo install cargo-audit)".into()),
        }
    }

    if ran.is_empty() && problems.is_empty() {
        let reason = if skipped.is_empty() { "No lock file was found (package-lock.json, pnpm-lock.yaml, Cargo.lock), so there is nothing to audit.".to_string() } else { format!("The audit could not run: {}.", skipped.join("; ")) };

        return check("audit", "Known vulnerabilities in dependencies", "skipped", reason, Vec::new(), None);
    }

    if problems.is_empty() {
        return check("audit", "Known vulnerabilities in dependencies", "pass", format!("{} found no known vulnerabilities.", ran.join(" and ")), skipped, None);
    }

    check(
        "audit",
        "Known vulnerabilities in dependencies",
        "fail",
        "Your dependencies have known security problems.".to_string(),
        problems.iter().cloned().chain(skipped).collect(),
        Some(prompt_fix("Fix with a prompt", format!("Update the dependencies of this project to fix the security audit findings ({}). Use the smallest version bumps that fix them, run the tests after each, and tell me anything that needs a major upgrade.", problems.join("; ")))),
    )
}

/// One dependency to look up: which registry and what name.
#[derive(Debug, Clone, PartialEq)]
pub struct Dependency {
    pub registry: &'static str,
    pub name: String,
}

/// The dependency names a folder declares (`package.json`, `requirements.txt`, `Cargo.toml`). Names only.
pub fn dependencies(root: &Path) -> Vec<Dependency> {
    let mut found: Vec<Dependency> = Vec::new();

    if let Some(manifest) = std::fs::read_to_string(root.join("package.json")).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()) {
        for section in ["dependencies", "devDependencies", "optionalDependencies"] {
            for (name, version) in manifest[section].as_object().into_iter().flatten() {
                /* a git, file or workspace dependency is not a registry name */
                if !version.as_str().is_some_and(|version| version.starts_with("file:") || version.starts_with("workspace:") || version.starts_with("git") || version.contains("://") || version.starts_with("link:")) {
                    found.push(Dependency { registry: "npm", name: name.clone() });
                }
            }
        }
    }

    if let Ok(text) = std::fs::read_to_string(root.join("requirements.txt")) {
        for line in text.lines() {
            let name: String = line.trim().chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect();

            if !name.is_empty() && !line.trim_start().starts_with('#') && !line.trim_start().starts_with('-') {
                found.push(Dependency { registry: "pypi", name });
            }
        }
    }

    if let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) {
        if let Ok(manifest) = text.parse::<toml::Table>() {
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                for (name, spec) in manifest.get(section).and_then(toml::Value::as_table).into_iter().flatten() {
                    let local = spec.as_table().is_some_and(|table| table.contains_key("path") || table.contains_key("git"));
                    let renamed = spec.as_table().and_then(|table| table.get("package")).and_then(toml::Value::as_str);

                    if !local {
                        found.push(Dependency { registry: "crates", name: renamed.unwrap_or(name).to_string() });
                    }
                }
            }
        }
    }

    found.sort_by(|a, b| (a.registry, &a.name).cmp(&(b.registry, &b.name)));
    found.dedup();
    found
}

/// How a registry answered for one name: it exists (with the age in days of its first release when the registry says), it does not, or
/// the lookup failed (which proves nothing either way).
#[derive(Debug, Clone, PartialEq)]
pub enum Existence {
    Exists { age_days: Option<i64> },
    Missing,
    Unknown(String),
}

fn lookup(dependency: &Dependency, registries: &Registries) -> Existence {
    let url = match dependency.registry {
        "npm" => format!("{}/{}", registries.npm.trim_end_matches('/'), dependency.name.replace('/', "%2F")),
        "pypi" => format!("{}/{}/json", registries.pypi.trim_end_matches('/'), dependency.name),
        _ => format!("{}/{}", registries.crates.trim_end_matches('/'), dependency.name),
    };
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(6)).timeout_read(Duration::from_secs(10)).build();

    match agent.get(&url).set("user-agent", "SDC-ShipSafe/3.1 (dependency existence check)").set("accept", "application/json").call() {
        Ok(response) => {
            let body: Value = response.into_string().ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null);
            let created = match dependency.registry {
                "npm" => body.pointer("/time/created").and_then(Value::as_str).map(str::to_string),
                "crates" => body.pointer("/crate/created_at").and_then(Value::as_str).map(str::to_string),
                _ => None,
            };
            let age_days = created.and_then(|text| chrono::DateTime::parse_from_rfc3339(&text).ok()).map(|created| (chrono::Utc::now() - created.with_timezone(&chrono::Utc)).num_days());

            Existence::Exists { age_days }
        }
        Err(ureq::Error::Status(404, _)) => Existence::Missing,
        Err(error) => Existence::Unknown(error.to_string()),
    }
}

/// 5. Hallucinated packages: a name the registry has never heard of. An AI that writes `import fast-json-sanitize` may be making the name
///    up - and anyone can register it tomorrow with malware inside. A package created in the last two weeks is flagged to look at, not failed.
pub fn existence(root: &Path, registries: &Registries) -> Value {
    let all = dependencies(root);

    if all.is_empty() {
        return check("existence", "Packages that may not exist", "skipped", "No package.json, requirements.txt or Cargo.toml dependencies were found.", Vec::new(), None);
    }

    let looked: Vec<(Dependency, Existence)> = std::thread::scope(|scope| {
        let mut out = Vec::new();

        for chunk in all.chunks(all.len().div_ceil(8).max(1)) {
            let handles: Vec<_> = chunk.iter().map(|dependency| scope.spawn(move || (dependency.clone(), lookup(dependency, registries)))).collect();

            out.extend(handles.into_iter().filter_map(|handle| handle.join().ok()));
        }

        out
    });
    let missing: Vec<String> = looked.iter().filter(|(_, answer)| *answer == Existence::Missing).map(|(dependency, _)| format!("{} ({}) - not in the {} registry", dependency.name, dependency.registry, dependency.registry)).collect();
    let fresh: Vec<String> = looked.iter().filter(|(_, answer)| matches!(answer, Existence::Exists { age_days: Some(age) } if *age < 14)).map(|(dependency, _)| format!("{} ({}) - first published less than two weeks ago; check who made it", dependency.name, dependency.registry)).collect();
    let unknown = looked.iter().filter(|(_, answer)| matches!(answer, Existence::Unknown(_))).count();

    if !missing.is_empty() {
        return check(
            "existence",
            "Packages that may not exist",
            "fail",
            format!("{} dependenc{} not found in the public registry. If an AI wrote them, they may be invented - and an attacker can register an invented name.", missing.len(), if missing.len() == 1 { "y was" } else { "ies were" }),
            missing.iter().cloned().chain(fresh.iter().cloned()).collect(),
            Some(prompt_fix("Fix with a prompt", format!("These dependencies do not exist in their public registry: {}. Find out what each was meant to be, replace it with the real, maintained package (or remove it), and check nothing imports the invented name.", missing.join("; ")))),
        );
    }

    if !fresh.is_empty() || unknown > 0 {
        return check(
            "existence",
            "Packages that may not exist",
            "warn",
            if unknown > 0 { format!("All the packages that could be looked up exist; {unknown} could not be checked (offline or rate limited).") } else { "All packages exist, but some are brand new.".to_string() },
            fresh.clone(),
            (!fresh.is_empty()).then(|| prompt_fix("Fix with a prompt", format!("Review these very new packages and confirm they are legitimate and needed: {}", fresh.join("; ")))),
        );
    }

    check("existence", "Packages that may not exist", "pass", format!("All {} dependencies exist in their public registries.", looked.len()), Vec::new(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn folder(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("sdc-ship-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A tiny HTTP server: `routes` maps a path fragment to (status, extra headers, body); anything else is a 404.
    fn serve(routes: Vec<(&'static str, u16, &'static str, &'static str)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());

        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut line = String::new();

                if BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }

                let route = routes.iter().find(|(fragment, ..)| line.split_whitespace().nth(1).is_some_and(|path| path == *fragment || (fragment.ends_with('*') && path.starts_with(fragment.trim_end_matches('*')))));
                let (status, headers, body) = route.map_or((404, "", ""), |(_, status, headers, body)| (*status, *headers, *body));

                let _ = write!(stream, "HTTP/1.1 {status} X\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });

        base
    }

    #[test]
    fn dependency_names_are_read_from_the_three_manifests_and_local_ones_are_left_out() {
        let root = folder("deps");

        std::fs::write(root.join("package.json"), r#"{"dependencies":{"react":"^19","left-pad":"1.3.0","mine":"workspace:*","gitdep":"git+https://x/y.git"},"devDependencies":{"vitest":"^2"}}"#).unwrap();
        std::fs::write(root.join("requirements.txt"), "requests==2.31\n# comment\n-r other.txt\nflask>=2\n").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"x\"\nversion=\"0.1.0\"\n[dependencies]\nserde=\"1\"\nlocal={path=\"../local\"}\n[dev-dependencies]\ntempfile=\"3\"\n").unwrap();

        let names: Vec<String> = dependencies(&root).iter().map(|d| format!("{}:{}", d.registry, d.name)).collect();

        for expected in ["npm:react", "npm:left-pad", "npm:vitest", "pypi:requests", "pypi:flask", "crates:serde", "crates:tempfile"] {
            assert!(names.contains(&expected.to_string()), "{expected} in {names:?}");
        }

        for left_out in ["npm:mine", "npm:gitdep", "crates:local"] {
            assert!(!names.contains(&left_out.to_string()), "{left_out}");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_package_the_registry_has_never_heard_of_is_a_failure_with_a_fix_and_an_old_one_passes() {
        let root = folder("slop");
        let old = (chrono::Utc::now() - chrono::Duration::days(900)).to_rfc3339();
        let fresh = (chrono::Utc::now() - chrono::Duration::days(3)).to_rfc3339();

        std::fs::write(root.join("package.json"), r#"{"dependencies":{"real-old":"1","made-up-by-an-ai":"1","brand-new":"1"}}"#).unwrap();

        let body_old: &'static str = Box::leak(format!(r#"{{"time":{{"created":"{old}"}}}}"#).into_boxed_str());
        let body_new: &'static str = Box::leak(format!(r#"{{"time":{{"created":"{fresh}"}}}}"#).into_boxed_str());
        let base = serve(vec![("/real-old", 200, "", body_old), ("/brand-new", 200, "", body_new)]);
        let registries = Registries { npm: base, ..Registries::default() };
        let answer = existence(&root, &registries);

        assert_eq!(answer["status"], "fail", "{answer}");
        assert!(answer["details"].as_array().unwrap().iter().any(|line| line.as_str().unwrap().contains("made-up-by-an-ai")));
        assert!(answer["details"].as_array().unwrap().iter().any(|line| line.as_str().unwrap().contains("brand-new") && line.as_str().unwrap().contains("two weeks")));
        assert!(!answer["details"].to_string().contains("real-old"));
        assert_eq!(answer["fix"]["kind"], "prompt");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_security_headers_are_named_with_the_line_to_add_and_a_complete_set_passes() {
        let bare = serve(vec![("/", 200, "Server: nginx/1.18.0\r\n", "ok")]);
        let answer = headers(Some(&bare));

        assert_eq!(answer["status"], "fail", "{answer}");
        assert!(answer["details"].to_string().contains("Content-Security-Policy: default-src 'self'"));
        assert!(answer["details"].to_string().contains("not served over https"), "http is itself a finding");
        assert!(answer["details"].to_string().contains("nginx/1.18.0"), "a version number is a finding");

        assert_eq!(headers(None)["status"], "skipped", "no address: said, not passed");
    }

    #[test]
    fn a_site_that_serves_its_env_file_fails_and_a_clean_one_passes() {
        let root = folder("access");
        let leaky = serve(vec![("/.env", 200, "", "DB_PASSWORD=hunter2hunter2\n")]);
        let bad = access(&root, Some(&leaky));

        assert_eq!(bad["status"], "fail", "{bad}");
        assert!(bad["details"].to_string().contains("/.env"));
        assert!(!bad["details"].to_string().contains("hunter2"), "the value is never repeated");

        let clean = serve(vec![]);

        assert_eq!(access(&root, Some(&clean))["status"], "pass");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_secret_in_a_web_folder_and_a_world_readable_env_are_found() {
        let root = folder("webroot");

        std::fs::create_dir_all(root.join("public")).unwrap();
        std::fs::write(root.join("public/.env"), "A=1").unwrap();

        let answer = access(&root, None);

        assert_eq!(answer["status"], "fail");
        assert!(answer["details"].to_string().contains("public/.env"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_secret_in_a_file_fails_the_secrets_check_without_showing_it() {
        let root = folder("secret");

        std::fs::write(root.join("config.txt"), "api_key = \"FAKEvalue9Zq4Xw7Lm2Pt8\"\n").unwrap();

        let answer = secrets(&root);

        assert_eq!(answer["status"], "fail");
        assert_eq!(answer["fix"]["kind"], "secrets");
        assert!(!answer.to_string().contains("FAKEvalue"));

        let clean = folder("secret-clean");

        std::fs::write(clean.join("a.txt"), "hello").unwrap();
        assert_eq!(secrets(&clean)["status"], "pass");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&clean);
    }

    #[test]
    fn the_audit_reads_npms_counts_and_says_so_when_there_is_nothing_to_audit() {
        assert_eq!(parse_npm_audit(r#"{"metadata":{"vulnerabilities":{"critical":1,"high":2,"moderate":0,"low":4,"total":7}}}"#), Some((1, 2, 0, 4)));
        assert_eq!(parse_npm_audit("not json"), None);

        let empty = folder("audit");
        let answer = audit(&empty);

        assert_eq!(answer["status"], "skipped", "no lock file is not a pass");

        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn the_verdict_is_the_worst_check_and_a_skipped_check_never_counts_as_a_pass() {
        let root = folder("verdict");
        let answer = run(&root, None, &Registries { npm: "http://127.0.0.1:9".into(), ..Registries::default() });

        assert_eq!(answer["checks"].as_array().unwrap().len(), 5);
        assert_ne!(answer["verdict"], "pass", "headers, audit and existence were skipped: {answer}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
