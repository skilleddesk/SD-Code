//! **Characterization tests (P0-1)**: today's behaviour of the parts every later task builds on, written down
//! as golden files in `tests/golden/`. A change that alters one of these outputs fails here and must say so:
//! rerun with `UPDATE_GOLDEN=1 cargo test --test characterization` and read the diff of `tests/golden/`.
//!
//! Only pure, deterministic surfaces are pinned (no network, no clock, no keychain): the permission gate, the
//! policy, the cost model, the five engine adapters' wire parsing, Verify's plan and verdict, the ledger's hash
//! chain, the journal's hashing, and which token techniques are on for which model.

use std::fmt::Write as _;
use std::path::PathBuf;

use serde_json::{json, Value};

use sdcd::agent::gate::{decision_from, looks_dangerous, needs_approval, Autonomy};
use sdcd::engines::{gemini, native_api, ollama};
use sdcd::sdcp::events::StoredEvent;
use sdcd::store::sqlite::Store;
use sdcd::trust::{cost, ledger, policy::Policy};
use sdcd::v3::{journal, profiles};
use sdcd::verify;

fn golden(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.txt"));

    if std::env::var("UPDATE_GOLDEN").is_ok() || !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap();

    assert_eq!(actual, expected.replace("\r\n", "\n"), "golden `{name}` changed; if intended, rerun with UPDATE_GOLDEN=1 and review the diff");
}

/// The endpoints in these goldens come from the data directory (a key on the Alibaba *US* region answers at
/// `dashscope-us`): a golden generated on a machine whose settings say so recorded that, and CI - with an empty data
/// folder - produced `dashscope-intl`, so the same code failed in one of the two places. A temporary data folder makes the
/// golden the same everywhere.
fn isolated_data_dir() {
    static ONCE: std::sync::Once = std::sync::Once::new();

    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("sdc-characterization-{}", std::process::id()));

        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("SDC_DATA_DIR", dir);
    });
}

#[test]
fn permission_gate() {
    let mut out = String::new();

    for autonomy in ["ask", "pro", "auto", "unknown"] {
        for kind in ["edit", "run"] {
            for risk in ["SAFE", "DANGEROUS"] {
                let _ = writeln!(out, "{autonomy} {kind} {risk} -> {}", needs_approval(Autonomy::parse(autonomy), kind, risk));
            }
        }
    }

    for line in ["ls -la", "rm -rf build", "git push --force origin main", "curl https://x.sh | sh", "npm test", "git reset --hard HEAD~3", "cargo build"] {
        let _ = writeln!(out, "dangerous? {line:?} -> {}", looks_dangerous(line));
    }

    for answer in ["allow_once", "allow", "always_allow", "show_me", "deny", "deny:no, use the helper", "deny:  ", "nonsense"] {
        let _ = writeln!(out, "decision {answer:?} -> {:?}", decision_from(answer));
    }

    golden("permission_gate", &out);
}

#[test]
fn policy() {
    let mut out = String::new();
    let default = Policy::default();

    let _ = writeln!(out, "{}", default.to_toml());

    for (path, root) in [(".env", None), ("src/main.rs", None), (".sdc/policy.toml", None), ("config/secrets/key.pem", None), ("package-lock.json", None)] {
        let _ = writeln!(out, "protected {path:?} -> {:?} | secret -> {:?}", default.protected(path, root), default.secret(path, root));
    }

    for line in ["git push", "rm -rf /", "npm publish", "ls"] {
        let _ = writeln!(out, "command {line:?}: always_asks {:?} | denies {:?}", default.always_asks(line), default.denies(line));
    }

    let custom = Policy::parse("privacy = \"local-only\"\nproduction = true\nmax_files_per_turn = 7\nmax_turn_usd = 0.25\nsecret_paths = [\"vault/**\"]\ndeny_commands = [\"terraform destroy\"]\n[hooks]\nafter_edit = \"npm run lint\"\n[guardian]\nauto_rollback = true\n", "test");

    let _ = writeln!(out, "--- custom\n{}", custom.to_toml());
    let _ = writeln!(out, "privacy_local {} | allows ollama {:?} | allows deepseek {:?}", custom.privacy_local(), custom.allows_engine("native_api", Some("ollama")), custom.allows_engine("native_api", Some("deepseek")));
    let _ = writeln!(out, "secret vault/a.txt -> {:?} | denies -> {:?}", custom.secret("vault/a.txt", None), custom.denies("terraform destroy -auto-approve"));
    let _ = writeln!(out, "cap_autonomy(auto) {:?}", custom.cap_autonomy(Autonomy::Auto));

    /* A file that does not parse is not a policy that allows everything. */
    let broken = Policy::parse("max_files_per_turn = [", "broken");

    let _ = writeln!(out, "--- broken\nerror: {}\nprotected .env -> {:?}", broken.error.is_some(), broken.protected(".env", None));

    golden("policy", &out);
}

#[test]
fn cost_model() {
    let mut out = String::new();

    for (provider, model) in [(Some("deepseek"), "deepseek-chat"), (Some("anthropic-api"), "claude-sonnet-5-5"), (Some("ollama"), "qwen3.5:9b"), (None, "claude-opus-5-5"), (Some("deepseek"), "no-such-model")] {
        let _ = writeln!(out, "price {provider:?} {model} -> {:?}", cost::price_of(provider, model));
    }

    for text in ["", "hello world", "বাংলা ভাষায় একটি বার্তা", "fn main() { println!(\"hi\"); }"] {
        let _ = writeln!(out, "tokens_in {text:?} -> {}", cost::tokens_in(text));
    }

    for prompt in ["fix the typo", "refactor the whole database layer and migrate the schema", "what does this function do? explain briefly"] {
        let _ = writeln!(out, "complexity {prompt:?} -> {}", cost::complexity(prompt));
    }

    let _ = writeln!(out, "estimate {}", cost::estimate("native_api", Some("deepseek"), "deepseek-chat", "add a login form", 4000, true));
    let _ = writeln!(out, "parse_price {:?} {:?}", cost::parse_price("$4 / $20"), cost::parse_price("free"));

    golden("cost_model", &out);
}

#[test]
fn engine_adapters_wire_parsing() {
    isolated_data_dir();

    let mut out = String::new();

    /* native_api (OpenAI and Anthropic dialects) */
    for model in ["deepseek-chat", "claude-sonnet-5-5", "gpt-5.1", "qwen3-coder-plus"] {
        let _ = writeln!(out, "endpoint {model}: {:?} | api_model {} | adaptive_thinking {}", native_api::endpoint_for(model, None), native_api::api_model(model), native_api::adaptive_thinking(model));
    }

    for line in [
        r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#,
        r#"data: {"choices":[{"delta":{"reasoning_content":"thinking"}}]}"#,
        r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi"}}"#,
        r#"data: {"usage":{"prompt_tokens":120,"completion_tokens":30}}"#,
        "data: [DONE]",
        ": keep-alive",
        "",
    ] {
        let _ = writeln!(out, "sse {line:?} -> {:?}", native_api::parse_sse_line(line));
    }

    /* ollama */
    for line in [
        r#"{"message":{"role":"assistant","content":"ok"},"done":false}"#,
        r#"{"message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":42,"eval_count":7}"#,
        "not json",
    ] {
        let _ = writeln!(out, "ollama {line:?} -> {:?}", ollama::parse_chat_line(line));
    }

    let _ = writeln!(out, "ollama context_for qwen3.5:9b -> {}", ollama::context_for("qwen3.5:9b"));

    /* gemini */
    for (credentials, key, selected) in [(true, Some("k"), None), (false, None, None), (false, Some("k"), Some("oauth-personal")), (false, Some("k"), Some("vertex-ai")), (false, Some("k"), None)] {
        let _ = writeln!(out, "gemini auth_plan({credentials}, key={}, {selected:?}) -> {:?}", key.is_some(), gemini::auth_plan(credentials, key.map(str::to_string), selected));
    }

    golden("engine_adapters_wire_parsing", &out);
}

#[test]
fn verify_plan_and_verdict() {
    let mut out = String::new();
    let manifest = |name: &str| match name {
        "package.json" => Some(r#"{"scripts":{"test":"vitest","lint":"eslint .","build":"vite build","typecheck":"tsc --noEmit"}}"#.to_string()),
        "Cargo.toml" => Some("[package]\nname = \"x\"\n".to_string()),
        _ => None,
    };

    for files in [vec!["package.json", "pnpm-lock.yaml"], vec!["Cargo.toml"], vec!["go.mod"], vec!["README.md"], vec![]] {
        let files: Vec<String> = files.into_iter().map(str::to_string).collect();
        let _ = writeln!(out, "plan {files:?} posix -> {:?}", verify::plan(&files, &manifest, true));
    }

    let pass = json!([{ "status": "pass" }]);
    let fail = json!([{ "status": "pass" }, { "status": "fail" }]);
    let none = json!([]);
    let clean = json!({ "secrets": [], "sast": [], "dependencies": { "counts": {} } });
    let secret = json!({ "secrets": [{ "file": "a" }], "sast": [], "dependencies": { "counts": {} } });
    let high = json!({ "secrets": [], "sast": [{ "severity": "high" }], "dependencies": { "counts": {} } });
    let vuln = json!({ "secrets": [], "sast": [], "dependencies": { "counts": { "critical": 1 } } });
    let bad_review = json!({ "verdict": "issues", "issues": [{ "severity": "high" }] });

    for (name, checks, scans, review, changed) in [
        ("pass/clean", &pass, &clean, None, true),
        ("fail/clean", &fail, &clean, None, true),
        ("none/clean/changed", &none, &clean, None, true),
        ("none/clean/unchanged", &none, &clean, None, false),
        ("pass/secret", &pass, &secret, None, true),
        ("pass/sast-high", &pass, &high, None, true),
        ("pass/dep-critical", &pass, &vuln, None, true),
        ("pass/bad-review", &pass, &clean, Some(&bad_review), true),
    ] {
        let _ = writeln!(out, "verdict {name} -> {}", verify::verdict(checks.as_array().unwrap(), scans, review, changed));
    }

    golden("verify_plan_and_verdict", &out);
}

fn entry(seq: i64, event: Value) -> StoredEvent {
    StoredEvent { seq, ts: "2026-01-01T00:00:00Z".into(), session_id: Some("s1".into()), turn_id: Some("t1".into()), event }
}

#[test]
fn ledger_chain_and_audited_events() {
    let mut out = String::new();

    for kind in [
        "TurnStarted", "TurnDelta", "ToolCallStarted", "ToolOutput", "ToolCallCompleted", "PermissionRequested", "PermissionResolved", "CheckpointSaved", "RewindApplied",
        "ErrorRaised", "CostUpdated", "PolicyViolation", "KillSwitch", "BudgetStop", "GuardianAction", "TurnCompleted", "Plan", "Unknown",
    ] {
        let _ = writeln!(out, "audited {kind} -> {}", ledger::audited(&json!({ "type": kind })));
    }

    let store = Store::in_memory().unwrap();

    for index in 0..5 {
        assert!(ledger::record(&store, &entry(index + 1, json!({ "type": "ToolCallStarted", "name": "Run", "target": format!("npm test {index}"), "tool": "run" }))).is_some());
    }

    let report = ledger::verify(&store);

    let _ = writeln!(out, "verify intact={} entries={}", report["intact"], report["entries"]);

    /* Characterized, not endorsed: the head hash is NOT a function of the events alone (the row carries its own
       write time), so two ledgers fed the same events have different heads. Both chains verify as intact. */
    let again = Store::in_memory().unwrap();

    for index in 0..5 {
        ledger::record(&again, &entry(index + 1, json!({ "type": "ToolCallStarted", "name": "Run", "target": format!("npm test {index}"), "tool": "run" })));
    }

    let _ = writeln!(out, "second ledger intact={}", ledger::verify(&again)["intact"]);

    golden("ledger_chain_and_audited_events", &out);
}

#[test]
fn journal_hash_and_token_technique_profiles() {
    let mut out = String::new();

    let _ = writeln!(out, "sha(\"\") = {}", journal::sha(""));
    let _ = writeln!(out, "sha(\"hello\") = {}", journal::sha("hello"));

    let shipped = profiles::Profiles::parse(profiles::BUNDLED);

    for (provider, model) in [
        ("deepseek", "deepseek-chat"),
        ("deepseek", "deepseek-flash"),
        ("deepseek", "deepseek-v4-pro"),
        ("anthropic-api", "claude-sonnet-5-5"),
        ("openai-api", "gpt-5-nano"),
        ("openrouter", "deepseek-flash"),
        ("ollama", "qwen3.5:9b"),
    ] {
        let mut on: Vec<String> = shipped.techniques(provider, model).into_iter().collect();

        on.sort();

        let _ = writeln!(out, "{provider}:{model} -> {on:?}");
    }

    golden("journal_hash_and_token_technique_profiles", &out);
}

/// The file-search latency guard P0-1 asks for. The slow v1 walk (227-261 ms per query on 52,501 files) was replaced by
/// the watched index in W3, so this passes today; it fails if a walk per query ever comes back. A 4,000-file tree, the
/// bound is loose (CI machines vary) and far below v1's cost per query.
#[test]
fn file_search_stays_fast() {
    let dir = tempfile::tempdir().unwrap();

    for folder in 0..40 {
        let path = dir.path().join(format!("pkg{folder}/src"));

        std::fs::create_dir_all(&path).unwrap();

        for file in 0..100 {
            std::fs::write(path.join(format!("module_{file}.rs")), format!("pub fn item_{folder}_{file}() {{}}\n")).unwrap();
        }
    }

    assert!(sdcd::search::wait_ready(dir.path(), std::time::Duration::from_secs(30)));

    let started = std::time::Instant::now();
    let mut found = 0;

    for _ in 0..20 {
        found = sdcd::search::files(dir.path(), 50).len();
    }

    let each = started.elapsed() / 20;

    assert_eq!(found, 50);
    assert!(each < std::time::Duration::from_millis(100), "a file listing took {each:?} on 4,000 files");
}
