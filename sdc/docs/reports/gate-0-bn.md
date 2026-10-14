# GATE 0 — প্রস্তুতি (Phase 0) রিপোর্ট

তারিখ: 2026-10-14 · branch: `sdc-v3/P0` · baseline: `sdc-baseline-v1` = `274ec6a` (= `v1.0.0`)

## 1. কী করা হয়েছে

| কাজ | অবস্থা | বিস্তারিত |
|---|---|---|
| HEAD-এ tag `sdc-baseline-v1` | ✅ | annotated tag, শুধু local, push হয়নি |
| v1 build → `bench/bin/v1/` | ✅ (macOS arm64) | `sdcd 1.0.0 (SDCP 0.1)`, sha256 `7ac0fe9c…16c4`, build info `bench/bin/v1/BUILD.txt`; tag থেকে আলাদা worktree-তে build হয়েছে। `bench/bin/` gitignore-এ যোগ হয়েছে। Windows/Linux v1 binary Phase 1-এ CI থেকে নেওয়া হবে |
| v1 test suite | ✅ সবুজ (macOS) | `cargo test --no-fail-fast`: ৭৩৮টা pass, ০টা fail, ৬টা ignored |
| A-01 যাচাই | ✅ | `docs/plan/ASSUMPTIONS.md` — ৪টা সত্য, ৩টা আংশিক, ২টা মিথ্যা |
| বিদ্যমান capability যাচাই | ✅ | নিচে §2 |
| U0 uniqueness audit | ✅ | `docs/reports/U0-uniqueness-bn.md` |
| `PROGRESS.md`, `ASSUMPTIONS.md` | ✅ | `docs/plan/` |

এই Mac-এ Rust toolchain ছিল না। build-এর জন্য `rustup` দিয়ে stable (cargo 1.99.0) `~/.cargo`-তে install করেছি,
shell profile বদলাইনি।

## 2. কোডে আসলে কী আছে (code > prompt)

| বিষয় | অবস্থা | প্রমাণ / মন্তব্য |
|---|---|---|
| Prompt caching | আংশিক | Anthropic `cache_control` system + শেষ message-এ (`agent/dialect.rs:78,165`); OpenAI-dialect-এ শুধু Alibaba-র জন্য; cached token পড়া হয় Anthropic, OpenAI, DeepSeek থেকে (`dialect.rs:471,616`)। **সমস্যা:** system prompt-এ `Today: {date}` (`agent/mod.rs:324`), তাই prefix রোজ বদলায়; `keep_small` পুরনো tool output ছোট করার সময় আগের message বদলে দেয়, ফলে cache ভাঙে |
| Checkpoints | আছে | data folder-এ shadow git repo (`git/mod.rs:18`), প্রতি turn-এ commit, rewind। user-এর repo-তে হাত দেয় না; SSH-এ host-এ থাকে |
| Secret redaction | আংশিক | secret ফাইল লুকানো (`fs/mod.rs:27`); command output ও web page redact হয় না |
| Sandbox | নেই | `pty/mod.rs:18`: "It is not a sandbox" — শুধু deny list |
| Retry/backoff | আছে | 2→30 s (`engines/native_api.rs:731`); `Retry-After` মানে না, jitter নেই |
| Crash recovery | আংশিক | চালু থাকা turn "Interrupted" হিসেবে বন্ধ হয় (`lib.rs:175`); agent loop resume হয় না |
| MCP client | আছে | stdio, local + SSH (`agent/mcp.rs`) |
| Hooks | নেই | — |
| Trace viewer | নেই | কাছাকাছি: Proof/audit tab |
| 3-OS CI | আছে | `cargo test` Ubuntu/Windows/macOS (`.github/workflows/ci.yml`) |
| API key storage | মিশ্র | Windows DPAPI / macOS Keychain (feature `keychain`); **Linux-এ plain file** mode 0600 (`auth/keychain.rs:205`); SQLite-এ কখনো নয় |
| Auto-update | আংশিক | শুধু নতুন version আছে কিনা দেখে; download/install নেই |
| Code signing | নেই | Sigstore provenance আছে; Authenticode/notarization নেই, macOS ad-hoc |
| File search | সরল | নিজের `read_dir` walk, সর্বোচ্চ 50k ফাইল, depth 24, 2 MB-এর বেশি বাদ (`agent/search.rs`); `.gitignore` মানে না, ripgrep/`ignore` crate/index/watcher নেই; SSH-এ `find`/`grep` |
| Per-provider rate limit | নেই | শুধু 429-এর পর retry |
| SQLite | আংশিক | WAL চালু; `integrity_check` বা backup নেই |
| Bench harness | আংশিক | `sdcd bench` নেই; `bench/remote/` (remote perf) ও `scripts/tools/bench-agent.mjs` (৩টা task) আছে |
| Local model backend | আংশিক | `trait Engine` আছে, কিন্তু agent-এর ভেতরে `enum Backend { Api, Ollama }`-এর branch সব জায়গায় ছড়ানো — D-1-এর "পরিষ্কার trait" এখনো নেই |
| ভাষা শনাক্ত | আছে | Banglish/Hinglish তালিকা `understand.rs` ও `intent/detect.rs`-এ দুই জায়গায় (cost.rs-এ নয়, prompt যা বলেছিল) |
| `/research` | আছে | DDG/SearXNG/Tavily/Brave/Serper, 5 search / 10 page / 10 মিনিট (`agent/research.rs`) |
| Sub-agent model | আংশিক | নিজে থেকে একই provider-এর `fast` model বেছে নেয় |
| Repo map / tree-sitter | নেই | শুধু ~150 token-এর top-level map (`agent/orient.rs`) |

## 3. Prompt থেকে পার্থক্য (code জিতবে; plan-এ যেভাবে মানিয়ে নেব)

1. **A-01(f) মিথ্যা:** `task` sub-agent আগে থেকেই fast model ব্যবহার করে। W6-এ এটাকে নতুন করে বানাব না, router-এর অংশ করব।
2. **A-01(i) মিথ্যা:** default-এ ১৯টা tool schema যায়, সবসময় ২২টা নয়। intent-ভিত্তিক filter নেই, তাই W5-এর কাজটা এখনো দরকার।
3. **Checkpoints:** prompt বলছে user repo-তে লুকানো ref (`refs/sdc/checkpoints/...`), কিন্তু বিদ্যমান shadow repo user-এর `.git`-এ কিছুই লেখে না, যা আরো নিরাপদ। **প্রস্তাব:** shadow repo রেখে দিয়ে turn/session index ও `checkpoint.list/restore` যোগ করা।
4. **W8:** অনেকখানি আগেই আছে (একাধিক search provider, limit, সূত্র)। বাকি কাজ: `require_command`, readability extraction, তারিখ, per-page sub-agent claims, pre-flight card।
5. **Token counter তিনটা** (দুটো নয়)। I8-এর একক per-script estimator তিনটারই জায়গা নেবে।
6. **"Today" date prefix-এ** থাকায় W5-এর আগেই A-08 খারাপ আসবে। W1 baseline-এ এটা যেমন আছে তেমনই মাপা হবে।
7. `docs/plan/SDC-PLAN-v3.md` repo-তে নেই। repo-তে `docs/MASTER-PLAN-v3-TRUST-KERNEL.md` নামে আলাদা একটা "v3" আছে, নামে গোলমাল হতে পারে।
8. Master prompt-এ **I12, Q1, G28–G33, G38, G40**-এর বর্ণনা নেই। PROGRESS-এ এগুলো `blocked`। U0-তে I12 = "durable resume + E2E handoff" ধরে নিয়েছি।

## 4. U0-এর সারকথা

১২টার কোনোটাই প্রতিযোগীরা হুবহু ship করে না। **I7 (Fleet) ও I8 (Interlingua) পরিষ্কারভাবে অনন্য।** বাকিগুলো আংশিক
অনন্য। সবচেয়ে কাছের প্রতিযোগী: I1 → Claude Code-এর unchanged-read stub; I11 → Copilot-এর citation-যাচাই memory;
I12 → Codex/Claude Code/Cursor/Copilot-এর handoff। বিস্তারিত ও link: `U0-uniqueness-bn.md`।

## 5. আপনার সিদ্ধান্ত দরকার

প্রতিটা প্রশ্নের সাথে আমার সুপারিশ দিলাম। শুধু "ঠিক আছে" বললেই সুপারিশটা নেওয়া হবে।

1. **Benchmark model (৫টা):** প্রস্তাব: Claude Sonnet 5.5 (`claude-sonnet-5-5`), OpenAI-এর বর্তমান মাঝারি coding model,
   Gemini Flash-শ্রেণি, DeepSeek-এর সর্বশেষ chat model, Qwen coder (Alibaba Model Studio)। OpenAI ও Gemini-র কোন model ID,
   সেটা আপনি নিশ্চিত করুন।
   - **Key:** DeepSeek, Alibaba আর Tavily-র key ফাইল চোখে পড়েছে (খুলিনি)। Anthropic, OpenAI ও Google key কোথায় থাকবে?
   - Competitor run-এর জন্য `claude` (2.1.296) আর `codex` (0.162.1) CLI install করা আছে। কোন account বা key দিয়ে চলবে?
2. **Bench budget (`bench.max_usd`):** প্রস্তাব (অনুমান, মাপা সংখ্যা নয়): smoke $2/run, medium $40/run, full $250/run। প্রথম smoke
   run-এর পর আসল খরচ দেখে ঠিক করব।
3. **Knowledge sync:** প্রস্তাব **off**, যতদিন X3 (E2E sync) তৈরি না হয়।
4. **External knowledge source allow-list:** প্রস্তাব: provider-দের pricing ও changelog পাতা (Anthropic, OpenAI, Google, DeepSeek, Alibaba,
   xAI, Moonshot, Mistral, Z.ai, Groq, OpenRouter), GitHub releases, crates.io, npm, PyPI। এর বাইরে কিছু লাগবে?
5. **VPS Runner host:** কোন VPS? sdc.skilleddesk.com যে VPS-এ, সেটাই, নাকি আলাদা?
6. **Account server:** প্রস্তাব: প্রথমে **single-user**, তবে schema multi-user-এর উপযোগী করে রাখা।
7. **Code-improvement proposal-এর ছন্দ:** প্রস্তাব **সাপ্তাহিক**, যেমন prompt-এ আছে।
8. **অনির্ধারিত item:** I12, Q1, G28–G33, G38, G40-এর সংজ্ঞা দিন, অথবা `SDC-PLAN-v3.md` repo-তে দিন।
9. **Push:** `sdc-baseline-v1` tag আর `sdc-v3/P0` branch GitHub-এ push করব? এখনো শুধু local।

"continue" পেলে Phase 1 (W1: `sdcd bench`) শুরু করব।
