//! Secret and SAST scanning of a change (Trust Kernel, part of the Verify engine).
//!
//! Both scanners read **the lines a change added** - a unified diff's `+` lines, with the file each one
//! belongs to - because the question a turn has to answer is "did this change introduce it", not "is
//! there anything anywhere in a ten-year-old repository". Both are rule tables a person can read: a
//! finding names its rule, its file and line, what is wrong and how to fix it, and a secret's value is
//! never repeated in full (it would end up in the event log, which is the one place it must not be).
//!
//! Nothing here is a model's opinion: a rule matches or it does not (principle P3). The review by a
//! second AI is a separate stage of Verify and is labelled as such.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Value};

/// One added line of a diff: the file it is in, its line number in the new file, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedLine {
    pub file: String,
    pub line: usize,
    pub text: String,
}

/// The `+` lines of a unified diff, each with its file and new-file line number.
pub fn added_lines(diff: &str) -> Vec<AddedLine> {
    let mut lines = Vec::new();
    let mut file = String::new();
    let mut number = 0usize;

    for raw in diff.lines() {
        if let Some(rest) = raw.strip_prefix("+++ ") {
            file = rest.trim().trim_start_matches("b/").to_string();

            if file == "/dev/null" {
                file.clear();
            }

            continue;
        }

        if raw.starts_with("--- ") || raw.starts_with("diff --git") || raw.starts_with("index ") {
            continue;
        }

        if let Some(header) = raw.strip_prefix("@@") {
            /* `@@ -12,5 +14,7 @@`: the new file's hunk starts at 14. */
            number = header
                .split_whitespace()
                .find_map(|part| part.strip_prefix('+'))
                .and_then(|part| part.split(',').next())
                .and_then(|start| start.parse::<usize>().ok())
                .unwrap_or(1);

            continue;
        }

        if let Some(text) = raw.strip_prefix('+') {
            lines.push(AddedLine { file: file.clone(), line: number, text: text.to_string() });
            number += 1;
        } else if !raw.starts_with('-') && !raw.starts_with('\\') {
            number += 1;
        }
    }

    lines
}

/// A secret rule: its id, what it finds, how bad it is.
struct SecretRule {
    id: &'static str,
    what: &'static str,
    severity: &'static str,
    pattern: &'static str,
}

const SECRET_RULES: &[SecretRule] = &[
    SecretRule { id: "private-key", what: "a private key", severity: "high", pattern: r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY-----" },
    SecretRule { id: "aws-access-key", what: "an AWS access key id", severity: "high", pattern: r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b" },
    SecretRule { id: "github-token", what: "a GitHub token", severity: "high", pattern: r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})\b" },
    SecretRule { id: "anthropic-key", what: "an Anthropic API key", severity: "high", pattern: r"\bsk-ant-[A-Za-z0-9_\-]{20,}" },
    SecretRule { id: "openai-key", what: "an OpenAI-style API key", severity: "high", pattern: r"\bsk-(?:proj-|live-)?[A-Za-z0-9]{20,}[A-Za-z0-9_\-]*" },
    SecretRule { id: "google-api-key", what: "a Google API key", severity: "high", pattern: r"\bAIza[0-9A-Za-z_\-]{35}\b" },
    SecretRule { id: "slack-token", what: "a Slack token", severity: "high", pattern: r"\bxox[baprs]-[0-9A-Za-z\-]{10,}" },
    SecretRule { id: "stripe-live-key", what: "a live Stripe key", severity: "high", pattern: r"\b(?:sk|rk)_live_[0-9A-Za-z]{20,}" },
    SecretRule { id: "db-url-password", what: "a database URL with its password", severity: "high", pattern: r"(?i)\b(?:mysql|postgres(?:ql)?|mongodb(?:\+srv)?|redis|amqp)://[^:\s/@]+:[^@\s]{3,}@" },
    SecretRule { id: "jwt", what: "a signed token (JWT)", severity: "medium", pattern: r"\beyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}" },
    SecretRule { id: "wp-db-password", what: "a WordPress database password", severity: "high", pattern: r#"define\(\s*['"]DB_PASSWORD['"]\s*,\s*['"][^'"]{4,}['"]"# },
    SecretRule {
        id: "assigned-secret",
        what: "a password or secret written into the code",
        severity: "medium",
        pattern: r#"(?i)\b(?:password|passwd|pwd|secret|api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret)\b\s*[:=]\s*['"]([^'"\s]{8,})['"]"#,
    },
];

fn secret_regexes() -> &'static Vec<(&'static SecretRule, Regex)> {
    static COMPILED: OnceLock<Vec<(&'static SecretRule, Regex)>> = OnceLock::new();

    COMPILED.get_or_init(|| SECRET_RULES.iter().map(|rule| (rule, Regex::new(rule.pattern).expect("secret rule"))).collect())
}

/// A value that is obviously a placeholder, not a secret: the docs' own examples and templates.
fn placeholder(value: &str) -> bool {
    let lowered = value.to_lowercase();

    ["xxx", "your", "example", "changeme", "placeholder", "dummy", "<", "${", "{{", "process.env", "getenv", "env(", "****", "redacted", "test", "sample"]
        .iter()
        .any(|marker| lowered.contains(marker))
}

/// What a finding shows of a secret: its first four characters and its length.
fn redact(value: &str) -> String {
    let head: String = value.chars().take(4).collect();

    format!("{head}… ({} chars)", value.chars().count())
}

/// Files a secret scan reads past: lock files carry integrity hashes that look like keys.
fn skipped_file(file: &str) -> bool {
    let lowered = file.to_lowercase();

    ["package-lock.json", "pnpm-lock.yaml", "yarn.lock", "cargo.lock", "composer.lock", "poetry.lock", ".min.js", ".map"]
        .iter()
        .any(|suffix| lowered.ends_with(suffix))
}

/// The secrets a change added.
pub fn secrets(diff: &str) -> Vec<Value> {
    let mut findings = Vec::new();

    for added in added_lines(diff) {
        if skipped_file(&added.file) {
            continue;
        }

        for (rule, regex) in secret_regexes() {
            let Some(found) = regex.captures(&added.text) else {
                continue;
            };
            let value = found.get(1).or_else(|| found.get(0)).map(|matched| matched.as_str()).unwrap_or_default();

            if rule.id == "assigned-secret" && placeholder(value) {
                continue;
            }

            /* The generic OpenAI shape also matches an Anthropic key; the specific rule already said it. */
            if rule.id == "openai-key" && added.text.contains("sk-ant-") {
                continue;
            }

            findings.push(json!({
                "rule": rule.id,
                "severity": rule.severity,
                "file": added.file,
                "line": added.line,
                "message": format!("This change adds {} to the code ({}).", rule.what, redact(value)),
                "fix": "Move it to an environment variable or a secret store, and rotate it: once written, assume it has leaked.",
            }));

            break;
        }
    }

    findings
}

/// A SAST rule: a pattern on one added line, for files with these extensions.
struct SastRule {
    id: &'static str,
    severity: &'static str,
    extensions: &'static [&'static str],
    pattern: &'static str,
    message: &'static str,
    fix: &'static str,
}

const JS: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs", "vue", "svelte"];
const PY: &[&str] = &["py"];
const PHP: &[&str] = &["php", "phtml", "inc"];
const GO: &[&str] = &["go"];
const ANY: &[&str] = &[];

const SAST_RULES: &[SastRule] = &[
    SastRule { id: "js-eval", severity: "high", extensions: JS, pattern: r"\beval\s*\(|new\s+Function\s*\(", message: "Code built from a string is run with eval / new Function.", fix: "Parse the data instead (JSON.parse), or call the function directly." },
    SastRule { id: "js-inner-html", severity: "medium", extensions: JS, pattern: r"\.innerHTML\s*=|\bdangerouslySetInnerHTML\b|document\.write\s*\(", message: "HTML is written from a value, which is how cross-site scripting gets in.", fix: "Use textContent, or sanitise the HTML (DOMPurify) before inserting it." },
    SastRule { id: "js-exec-interpolated", severity: "high", extensions: JS, pattern: r"\b(?:exec|execSync|spawnSync)\s*\(\s*(?:`[^`]*\$\{|[^)]*\+\s*\w)", message: "A shell command is built from a variable (command injection).", fix: "Use execFile/spawn with an argument array, never a string built from input." },
    SastRule { id: "js-sql-concat", severity: "high", extensions: JS, pattern: r#"(?i)\b(?:query|execute|raw)\s*\(\s*(?:`\s*(?:select|insert|update|delete)[^`]*\$\{|['"]\s*(?:select|insert|update|delete)[^'"]*['"]\s*\+)"#, message: "An SQL statement is built by joining strings (SQL injection).", fix: "Use the driver's placeholders (? or $1) and pass the values separately." },
    SastRule { id: "js-tls-off", severity: "high", extensions: JS, pattern: r#"rejectUnauthorized\s*:\s*false|NODE_TLS_REJECT_UNAUTHORIZED\s*=\s*['"]?0"#, message: "TLS certificate checking is switched off.", fix: "Keep verification on; trust a private CA with the `ca` option instead." },
    SastRule { id: "py-eval", severity: "high", extensions: PY, pattern: r"\b(?:eval|exec)\s*\(", message: "Code built from a string is run with eval / exec.", fix: "Use ast.literal_eval for data, or call the function directly." },
    SastRule { id: "py-shell-true", severity: "high", extensions: PY, pattern: r"subprocess\.\w+\([^)]*shell\s*=\s*True|\bos\.system\s*\(", message: "A command runs through the shell (command injection if any part is input).", fix: "Pass an argument list with shell=False." },
    SastRule { id: "py-pickle", severity: "medium", extensions: PY, pattern: r"\bpickle\.loads?\s*\(|\byaml\.load\s*\(", message: "Untrusted data may be deserialised into objects (code execution).", fix: "Use json, or yaml.safe_load." },
    SastRule { id: "py-sql-format", severity: "high", extensions: PY, pattern: r#"\.execute\s*\(\s*(?:f['"]|['"][^'"]*['"]\s*%|['"][^'"]*['"]\s*\.format\()"#, message: "An SQL statement is built with string formatting (SQL injection).", fix: "Pass the values as the second argument: cursor.execute(sql, params)." },
    SastRule { id: "py-tls-off", severity: "high", extensions: PY, pattern: r"verify\s*=\s*False", message: "TLS certificate checking is switched off.", fix: "Keep verify=True, or point it at a CA bundle." },
    SastRule { id: "php-eval", severity: "high", extensions: PHP, pattern: r"\b(?:eval|assert|create_function)\s*\(", message: "Code built from a string is run.", fix: "Remove the eval; call the code directly." },
    SastRule { id: "php-input-sql", severity: "high", extensions: PHP, pattern: r"(?:mysqli?_query|->query|->get_results|->get_var|->get_row)\s*\([^;]*\$_(?:GET|POST|REQUEST|COOKIE)", message: "Request input goes straight into an SQL query (SQL injection).", fix: "Use prepared statements ($wpdb->prepare, PDO placeholders)." },
    SastRule { id: "php-input-echo", severity: "medium", extensions: PHP, pattern: r"\b(?:echo|print)\s+[^;]*\$_(?:GET|POST|REQUEST|COOKIE)", message: "Request input is printed into the page unescaped (cross-site scripting).", fix: "Escape it: esc_html(), htmlspecialchars()." },
    SastRule { id: "php-input-exec", severity: "high", extensions: PHP, pattern: r"\b(?:shell_exec|system|passthru|exec|popen|proc_open)\s*\([^;]*\$_(?:GET|POST|REQUEST|COOKIE)", message: "Request input goes into a shell command (command injection).", fix: "Never pass input to a shell; if unavoidable, escapeshellarg() every part." },
    SastRule { id: "php-unserialize", severity: "high", extensions: PHP, pattern: r"\bunserialize\s*\(\s*\$_(?:GET|POST|REQUEST|COOKIE)", message: "Request input is unserialised (object injection).", fix: "Use json_decode for input." },
    SastRule { id: "php-include-input", severity: "high", extensions: PHP, pattern: r"\b(?:include|require)(?:_once)?\s*\(?\s*\$_(?:GET|POST|REQUEST|COOKIE)", message: "A file path from the request is included (file inclusion).", fix: "Map the input to a fixed list of allowed files." },
    SastRule { id: "go-tls-off", severity: "high", extensions: GO, pattern: r"InsecureSkipVerify\s*:\s*true", message: "TLS certificate checking is switched off.", fix: "Remove InsecureSkipVerify; add the CA to RootCAs instead." },
    SastRule { id: "go-sql-sprintf", severity: "high", extensions: GO, pattern: r#"(?i)fmt\.Sprintf\(\s*"\s*(?:select|insert|update|delete)"#, message: "An SQL statement is built with Sprintf (SQL injection).", fix: "Use db.Query(sql, args...) with placeholders." },
    SastRule { id: "chmod-777", severity: "medium", extensions: ANY, pattern: r"\bchmod\s+(?:-R\s+)?0?777\b", message: "Everything is made writable by everyone.", fix: "Grant the least access needed: 755 for folders, 644 for files." },
    SastRule { id: "weak-hash-password", severity: "medium", extensions: ANY, pattern: r"(?i)\b(?:md5|sha1)\s*\([^)]*pass", message: "A password is hashed with MD5/SHA-1, which are fast to crack.", fix: "Use bcrypt, scrypt or Argon2 (password_hash in PHP)." },
];

fn sast_regexes() -> &'static Vec<(&'static SastRule, Regex)> {
    static COMPILED: OnceLock<Vec<(&'static SastRule, Regex)>> = OnceLock::new();

    COMPILED.get_or_init(|| SAST_RULES.iter().map(|rule| (rule, Regex::new(rule.pattern).expect("sast rule"))).collect())
}

fn extension(file: &str) -> String {
    file.rsplit('.').next().unwrap_or_default().to_lowercase()
}

/// The risky patterns a change added.
pub fn sast(diff: &str) -> Vec<Value> {
    let mut findings = Vec::new();

    for added in added_lines(diff) {
        let ext = extension(&added.file);
        let trimmed = added.text.trim_start();

        /* A comment that mentions eval is not a call to it. */
        if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }

        for (rule, regex) in sast_regexes() {
            if !rule.extensions.is_empty() && !rule.extensions.contains(&ext.as_str()) {
                continue;
            }

            /* `yaml.load(…, Loader=yaml.SafeLoader)` is the safe spelling (the regex crate has no look-ahead). */
            if rule.id == "py-pickle" && added.text.contains("SafeLoader") {
                continue;
            }

            if regex.is_match(&added.text) {
                findings.push(json!({
                    "rule": rule.id,
                    "severity": rule.severity,
                    "file": added.file,
                    "line": added.line,
                    "message": rule.message,
                    "fix": rule.fix,
                }));
            }
        }
    }

    findings
}

/// The number of rules each scanner has - what the Verify tab says it checked.
pub fn rule_counts() -> (usize, usize) {
    (SECRET_RULES.len(), SAST_RULES.len())
}

/// A text a tool is about to write, scanned as if it were one whole new file - so the agent is warned
/// before a key lands on disk, not after.
pub fn secrets_in_text(file: &str, text: &str) -> Vec<Value> {
    let diff: String = std::iter::once(format!("+++ b/{file}\n@@ -0,0 +1 @@"))
        .chain(text.lines().map(|line| format!("+{line}")))
        .collect::<Vec<_>>()
        .join("\n");

    secrets(&diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/pay.ts b/src/pay.ts\n\
--- a/src/pay.ts\n\
+++ b/src/pay.ts\n\
@@ -10,3 +10,6 @@ export function pay() {\n\
 const a = 1;\n\
+const key = \"sk-proj-abcdefghijklmnopqrstuvwx1234\";\n\
+db.query(\"SELECT * FROM users WHERE id = \" + id);\n\
-const old = 2;\n\
+el.innerHTML = input;\n";

    #[test]
    fn added_lines_carry_their_file_and_new_line_number() {
        let lines = added_lines(DIFF);

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].file, "src/pay.ts");
        assert_eq!(lines[0].line, 11);
        assert_eq!(lines[2].line, 13);
    }

    #[test]
    fn a_key_in_a_change_is_found_and_never_repeated_whole() {
        let found = secrets(DIFF);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["rule"], "openai-key");
        assert_eq!(found[0]["line"], 11);
        assert!(!found[0]["message"].as_str().unwrap().contains("abcdefghijklmnop"), "{}", found[0]["message"]);
    }

    #[test]
    fn placeholders_and_lock_files_are_not_secrets() {
        let diff = format!(
            "+++ b/config.js\n@@ -0,0 +1,2 @@\n+const password = \"your-password-here\";\n+const api_key = process.env.KEY;\n+++ b/package-lock.json\n@@ -0,0 +1 @@\n+\"integrity\": \"sha512-{}\"",
            fake_aws_key()
        );

        assert!(secrets(&diff).is_empty(), "{:?}", secrets(&diff));
    }

    #[test]
    fn sast_finds_injection_and_xss_by_language() {
        let found = sast(DIFF);
        let rules: Vec<&str> = found.iter().map(|finding| finding["rule"].as_str().unwrap()).collect();

        assert!(rules.contains(&"js-sql-concat"), "{rules:?}");
        assert!(rules.contains(&"js-inner-html"), "{rules:?}");

        let php = "+++ b/search.php\n@@ -0,0 +1,2 @@\n+$rows = $wpdb->get_results(\"SELECT * FROM t WHERE q = '\" . $_GET['q'] . \"'\");\n+echo $_GET['q'];";
        let rules: Vec<String> = sast(php).iter().map(|finding| finding["rule"].as_str().unwrap().to_string()).collect();

        assert_eq!(rules, ["php-input-sql", "php-input-echo"]);
    }

    #[test]
    fn a_python_rule_does_not_fire_on_javascript_and_comments_are_skipped() {
        let diff = "+++ b/a.js\n@@ -0,0 +1,2 @@\n+requests.get(url, verify=False)\n+// eval(x) is dangerous";

        assert!(sast(diff).is_empty());
    }

    #[test]
    fn text_about_to_be_written_is_scanned_too() {
        assert_eq!(secrets_in_text(".env.example", &format!("AWS_KEY={}", fake_aws_key()))[0]["rule"], "aws-access-key");
    }

    /// A made-up AWS key id, put together at run time so the repository's own secret scan (gitleaks) does
    /// not read the scanner's test as a leak.
    fn fake_aws_key() -> String {
        ["AK", "IA", "ABCDEFGHIJKLMNOP"].concat()
    }
}
