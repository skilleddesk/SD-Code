> **ARCHIVED (2026-10-18).** Written for version 1.0.0; SDC is now 3.x and several parts below changed or were added
> (ModelGate, Step Trace, token estimate, secret guard, Reviewer model). Use it as a map of how the v1 intelligence was laid out,
> not as a statement of what is true today. Current status: [`PLAN-STATUS.md`](PLAN-STATUS.md).

# SDC — Intelligence Map (প্রজেক্টে যত "বুদ্ধিমত্তা", এজেন্ট ও প্ল্যান আছে)

উদ্দেশ্য: এই ফাইলটা Claude-কে দিয়ে আরও বিশ্লেষণ করানোর জন্য। এখানে লেখা আছে SDC-তে কোন কোন "বুদ্ধিমান" অংশ আছে, প্রতিটা কোন ফাইলে থাকে, কী করে, আর কোন প্ল্যান ডকুমেন্টে তার নকশা আছে।
সব তথ্য কোড ও ডক থেকে নেওয়া (মডিউলের `//!` ডক-কমেন্ট, `docs/*.md`)। যেটা পড়ে যাচাই হয়নি, সেটা "অনুমান" লেখা আছে।

প্রজেক্ট: **SDC — Skilleddesk Code**, local-first coding-agent workbench। Tauri 2 app (React/TS) + Rust daemon `sdcd` + append-only event log + SDCP protocol।
বর্তমান সংস্করণ: 1.0.0 (git log অনুযায়ী)।

---

## ১. এক নজরে — বুদ্ধিমত্তার ১২টি স্তর

| # | স্তর | কাজ | মূল কোড (`sdc/sdcd/src/…`) |
|---|------|-----|----------------------------|
| 1 | **SDC Agent** | daemon-এর নিজের agent loop: মডেলকে প্রশ্ন → tool call → gate → ফলাফল ফেরত, কাজ শেষ না হওয়া পর্যন্ত | `agent/mod.rs`, `agent/tools.rs`, `agent/dialect.rs` |
| 2 | **Engine adapters** | বাইরের এজেন্ট/মডেল চালানো: `claude_code`, `codex`, `gemini`, `native_api`, `ollama` | `engines/*` |
| 3 | **Intent Engine** | যেকোনো ভাষা/উপভাষা (Banglish, Sylheti, Hinglish…) থেকে Task Spec, তারপর প্রতি engine-এর জন্য আলাদা prompt | `intent/mod.rs`, `intent/detect.rs`, `understand.rs` |
| 4 | **Voice** | কথা → text (whisper.cpp লোকাল আগে; না থাকলে Groq / Alibaba Qwen3-ASR / OpenAI) → একই Intent Engine | `intent/voice.rs` |
| 5 | **Pace + Orientation** | কাজের আকার নিজে বুঝে কম step-এ উত্তর; প্রথম step-এই ফোল্ডার, test কমান্ড, uncommitted অবস্থা জানানো (~১৫০ token) | `agent/orient.rs` |
| 6 | **Context management** | resume / compact / fit — লম্বা চ্যাট মডেলের window-এ ধরানো, পুরনো turn-এর digest | `context.rs` |
| 7 | **Continuity** | এক প্রজেক্টে একাধিক মডেল বদলালেও একই code style; মডেল বদলালে hand-over note | `continuity.rs`, `session_bridge/mod.rs` |
| 8 | **Verify** | আগে মেশিনের check (typecheck/lint/test/build), তারপর *অন্য* engine diff পড়ে review; verdict: PASS / FAIL / NO_CHECKS / UNPROVEN | `verify/mod.rs`, `trust/scan.rs` |
| 9 | **Trust Kernel** | audit ledger (hash-chain), policy-as-code, cost governor, kill switch, Trust score, Proof Pack | `trust/*` |
| 10 | **Research** | `/research`: প্ল্যান → ৩–৫ সার্চ → পেজ পড়া (sub-agent) → `[n]` সোর্সসহ উত্তর | `agent/research.rs`, `agent/web.rs`, `agent/search.rs` |
| 11 | **Ops / Agency layer** | Safe Deploy, Night Guardian, Health Watch, Playbooks, Staging, Shadow DB, X-ray, Team roles | `ops/*` |
| 12 | **SDC Anywhere** | দূর থেকে (ফোন/ব্রাউজার) approval, chat, terminal, preview — E2E এনক্রিপ্টেড relay | `anywhere/*`, `sdc/cloud`, `sdc/web` |

---

## ২. SDC Agent — বিস্তারিত

**Loop:** মডেলকে প্রশ্ন → উত্তরে text ও tool call → প্রতি call: card → (checkpoint) → gate → run → result → পরের message। যতক্ষণ না মডেল tool call বন্ধ করে।
**সীমা (runaway-এর বিল ঠেকাতে):** runaway detector (একই call বারবার), cost governor, Stop বাটন, ঐচ্ছিক `maxSteps`।

### Tools (`agent/tools.rs`)
| Tool | কাজ | অনুমতি |
|------|-----|--------|
| `read_file` (line range সহ), `list_dir`, `search`, `grep`, `glob`, `git_diff` | পড়া/খোঁজা | কখনো জিজ্ঞেস করে না |
| `write_file`, `edit_file`, `apply_patch` | ফাইল বদলানো (`apply_patch` = Codex-এর edit format, যে মডেল তাতে ট্রেনড) | Ask মোডে জিজ্ঞেস |
| `run_command` | কমান্ড চালানো | Ask/Pro-তে জিজ্ঞেস; বিপজ্জনক দেখালে সবসময় |
| `start_process`, `process_output`, `stop_process` | dev server/watcher background-এ চালানো (`agent/background.rs`) | run-এর নিয়মে |
| `web_fetch`, `web_search` | ডক পড়া/খোঁজা | web gate-এর নিয়মে |
| `browser`, `screenshot`, `view_image` | headless Chrome/Edge চালিয়ে পেজ খোলা, click, type, ছবি দেখা (`agent/browser.rs`) | ছবি বোঝে এমন মডেলে |
| `ask_user` | কাজের মাঝে choice-সহ প্রশ্ন | — |
| `remember` | স্মৃতি লিখে রাখা (UI: `kernel/MemoryDialog.tsx`) | — |
| `update_plan` | plan card আপডেট (UI: `panels/turns/PlanCard.tsx`) | কখনো জিজ্ঞেস করে না |
| `task` | **sub-agent**: নিজের আলাদা conversation, শুধু read-only tool, এক card-এ রিপোর্ট; একাধিক হলে সমান্তরালে চলে | — |
| `mcp__<server>__<tool>` | প্রজেক্টের `.sdc/mcp.json` থেকে MCP server-এর tool (VPS হলে ssh দিয়ে) | `run` হিসেবে গেট হয় |

### Permission gate (`agent/gate.rs`)
| স্তর | ফাইল বদল | কমান্ড | বিপজ্জনক কমান্ড |
|------|----------|--------|-----------------|
| Ask | জিজ্ঞেস | জিজ্ঞেস | জিজ্ঞেস |
| Pro | চলে | জিজ্ঞেস | জিজ্ঞেস |
| Auto | চলে | চলে | জিজ্ঞেস |
deny list ও file guard এর আগেই refuse করে — কোনো স্তরে বন্ধ হয় না।

### Skills (`agent/skills.rs`)
`.sdc/skills/<name>/SKILL.md` (এবং `.claude/skills`, `.codex/skills`, `.agents/skills`)। system prompt-এ শুধু name + description যায়; পুরো ফাইল দরকার হলে `read_file` দিয়ে পড়ে।

### Sub-agent মডেল বাছাই
turn-এর মডেল balanced/deep হলে sub-agent একই provider-এর সবচেয়ে নতুন `fast` মডেল ব্যবহার করে (একই key/endpoint)। সর্বনিম্ন reasoning effort। তার reads সমান্তরালে।

### স্থানীয় মডেল
`agent/ollama_native.rs` (Ollama `/api/chat`), `providers/local.rs` (LM Studio, llama.cpp, llamafile, LocalAI, vLLM, SGLang, Jan, KoboldCpp — OpenAI-সামঞ্জস্য যেকোনো server)। `agent/dialect.rs`: দুই tool-calling dialect (Anthropic `/v1/messages`, OpenAI-style), thinking block signature সহ হুবহু রাখে।

---

## ৩. Intent Engine (ভাষা বোঝা) — চার ধাপ
1. **detect** — script, ভাষা, romanized/mixed, আঞ্চলিক রূপ (অফলাইন, confidence সহ)
2. **understand** — মডেল দিয়ে Task Spec: target, goal, ৩–৮ acceptance condition, কী করা যাবে না, সর্বোচ্চ ২ প্রশ্ন (ব্যবহারকারীর ভাষায়), কম confidence হলে back-translation
3. **confirm** — Intent Contract card; সংশোধন glossary term হয়ে পরেরবার কাজে লাগে (`kernel/IntentCard.tsx`)
4. **compile** — একই Task Spec থেকে প্রতি engine-এর আলাদা prompt (task, acceptance criteria, rules file, repo map, policy, glossary, "কী করবে না", test থাকলে আগে test লেখার নির্দেশ)

`understand.rs` প্রতিটি message-এ ছোট নির্দেশ জোড়ে: সব request খুঁজো, বানান পেরিয়ে পড়ো, এক লাইনে `Understood: …` লিখো, ব্যবহারকারীর ভাষায় উত্তর দাও। (এই কথোপকথনের শুরুর `Understood:` লাইন ওই ব্যবস্থারই অংশ।)

---

## ৪. Trust Kernel (`trust/*`) — সব AI action এর ভেতর দিয়ে যায়
| অংশ | ফাইল | গ্যারান্টি |
|-----|------|-----------|
| Audit ledger | `ledger.rs` | প্রতিটি action hash-chain করে লেখা; বদল ধরা পড়ে |
| Policy engine | `policy.rs` | `.sdc/policy.toml`: protected_paths, always_ask, deny_commands, max_files_per_turn, max_turn_usd, `privacy="local-only"`, `production`, `[guardian] auto_rollback`। ফাইলের list ডিফল্টকে *বাড়ায়*, মুছে না |
| Scan | `scan.rs` | secret + SAST rule (মতামত নয়, নিয়ম দিয়ে); dependency audit |
| Time Machine | `checkpoints/`, `rewind/` | প্রতি পরিবর্তনের আগে checkpoint; ফাইল *ও* কথোপকথন restore |
| Cost governor | `cost.rs` | প্রতিটি সংখ্যা `measured` / `priced` / `subscription` / estimate লেবেলসহ; turn/chat/day/month budget; সহজ কাজে সস্তা মডেল সাজেশন; "saved" শুধু measured |
| Kill switch | `kill.rs` | এক কী-তে সব turn, check, deploy, command বন্ধ |
| Scores | `score.rs` | প্রতি turn-এর Trust score, প্রতি site-এর Ops score — কারণসহ; verify না হলে সীমিত (UNPROVEN) |
| Proof Pack | `proof.rs` | একটা turn কী করল, তার প্রমাণ এক ফাইলে |

নীতিগুলো (P1–P7): P1 engine কোডের পাশে/UI আপনার কাছে, P2 SDC AI বানায় না চালায়, P4 কোনো সংখ্যা কারণ ছাড়া নয়, P5 পরিবর্তনের আগে checkpoint। (পুরো তালিকা: `docs/MASTER_SPEC.md` §2.4)

---

## ৫. Verify pipeline (`verify/mod.rs`)
1. **checks** — প্রজেক্টের নিজের typecheck/lint/test/build, `cargo`, `go`, `pytest` (manifest দেখে খোঁজে; host হলে সেখানে চলে)
2. **scans** — secret, SAST, `npm audit`/`pip-audit`/`cargo audit`/`composer audit`
3. **review** — *অন্য* engine শুধু diff পড়ে, `file:line` সহ issue; Intent Contract-এর acceptance criteria এক এক করে বিচার করে
check ফেল করলে review বাদ (টোকেন বাঁচাতে)। ফলাফল `VerifyUpdated` snapshot হিসেবে যায় (UI: `panels/right/VerifyTab.tsx`)।

---

## ৬. Ops / Agency layer (`ops/*`) — কোড লেখার পরের ধাপ
| মডিউল | কাজ |
|-------|-----|
| `deploy.rs` | Safe Deploy: site আগের চেয়ে খারাপ হলে না বলে থাকে না, ফিরে যাওয়ার পথ আছে |
| `guardian.rs` | Night Guardian: site টানা দুই check-এ down হলে পদক্ষেপ; auto-rollback শুধু policy-তে অনুমতি থাকলে |
| `health.rs` | up/speed, SSL দিন, disk, backup বয়স, error log, শেষ deploy → Ops Score |
| `playbook.rs` | অনেক site-এ একই ধাপ; প্রতিটি আলাদা backup/health/rollback সহ |
| `staging.rs` | micro-staging ও client approval পেজ |
| `shadowdb.rs` | migration আগে ডেটাবেসের throwaway কপিতে রিহার্সাল |
| `xray.rs` | অজানা server-এর read-only স্ক্যান |
| `team.rs` | Owner / Developer / Reviewer / Client role |

---

## ৭. Provider / Model বুদ্ধিমত্তা
- `providers/models.rs` — model catalogue **ডেটা**, হার্ডকোড নয়; tier (fast/balanced/deep), দাম, window
- `providers/mod.rs` — Provider Hub-এর ৬টি flow (key, subscription OAuth, Ollama, custom endpoint, registry, doctor)
- Dropdown-এ শুধু *connected* engine/model (ROADMAP-v4 সিদ্ধান্ত ৬); প্রতি family-র শেষ ২–৩ version (সিদ্ধান্ত ৭)
- `duel/mod.rs` — একই prompt দুই engine-এ পাশাপাশি
- `session_bridge/mod.rs` — turn চলাকালীন engine বদলালেও কথোপকথন হারায় না
- `engines/credentials.rs` — একবার sign in, সব জায়গায়; `engines/steer.rs` — চলমান turn-এ message পাঠানো
- `host/doctor.rs` — ১০টি পরিবেশ check; `host/tools.rs` — দরকারি tool নিজে install

---

## ৮. SDC Anywhere (`anywhere/*`, `sdc/cloud`, `sdc/web`)
- `core.rs` — প্রতি browser connection একটা state machine; `crypto.rs` — E2E session
- `router.rs` — permission gate ↔ ব্রাউজারের approval সেতু; `envelope.rs`/`canonical.rs` — approval envelope + `action_hash`
- `webauthn.rs` — passkey যাচাই; `identity.rs`, `registry.rs` — paired device
- `sdc/cloud` — Cloudflare Worker + Durable Object (Hub) + D1; শুধু ciphertext বয়ে নেয়, plaintext রাখে না
- `sdc/web` — ব্রাউজার ক্লায়েন্ট

---

## ৯. পরিকল্পনা ডকুমেন্ট (কোথায় কোন প্ল্যান)
| ফাইল | বিষয় |
|------|-------|
| `Full Plan.txt` | মাস্টার বিল্ড স্পেসিফিকেশন v3.0 (বাংলা, ১১৯১ লাইন) — মূল spec |
| `sdc/docs/MASTER_SPEC.md` | একই spec-এর repo সংস্করণ; UI নিয়মের উৎস |
| `sdc/docs/ROADMAP-v4.md` | v4: agent daemon-এ, dual path (CLI রাখা), Verify = অন্য engine + diff, CodeMirror 6, xterm.js, connected-only dropdown; Phase 0–5 |
| `sdc/docs/MASTER-PLAN-v3-TRUST-KERNEL.md` | Trust Kernel + Universal Intent Engine; ৪টি AI-র সুপারিশ; roadmap 0.12 → 2.0 |
| `sdc/docs/SDC-ANYWHERE-PLAN-v2.md` | দূর-নিয়ন্ত্রণ: নিরাপত্তা মডেল, ৬ ব্যবহার-উদাহরণ, ফেজ ০.১৭–০.২১+ |
| `sdc/docs/ANALYSIS-local-model-and-research.md` | লোকাল মডেল (Qwen3.5-9B) ও `/research`-এর গ্যাপ বিশ্লেষণ, P0/P1/P2 |
| `sdc/docs/PERF-1.0.md` | কেন agent ধীর ছিল (১০–১২ ঘণ্টা বনাম ~১ ঘণ্টা) ও 1.0-তে কী বদলাল |
| `sdc/docs/REMOTE.md`, `SSH-CONNECT.md` | VPS/Remote SSH; ৮ স্তর, ৭ নিরাপত্তা নিয়ম, ব্যর্থতার তালিকা |
| `sdc/docs/NEEDS-FROM-OWNER.md` | মালিকের কাছ থেকে যা বাকি |
| `NOTES.md` | ধাপে ধাপে কাজের নোট (STEP 35 পর্যন্ত, ১৫৬৫ লাইন) |
| `CHANGELOG.md` | রিলিজ ইতিহাস |

---

## ১০. PERF-1.0 থেকে শেখা (agent-কে আরও "বুদ্ধিমান/দ্রুত" করার প্রমাণিত কারণ)
১. মডেল key-র region-এর বাইরে চলছিল (first token ১২–২০ s)। ২. শুধু system prompt cache হত, পুরো কথোপকথন নয়। ৩. window ধরা ছিল 128K, আসলে 393K — ঘন ঘন fold, prefix cache ভাঙা, একই ফাইল ১২ বার পড়া। ৪. `parallel_tool_calls` চাওয়াই হয়নি — ৬০% step-এ একটা call। ৫. DeepSeek V4 সবসময় `high` thinking। ৬. sub-agent পরপর পড়ত। ৭. একটা permission-এর অপেক্ষায় ৭ ঘণ্টা ৪০ মিনিট গেছে (রাতে)।

---

## ১১. Claude-কে যা জিজ্ঞেস করা যায় (বিশ্লেষণের প্রশ্ন)
1. Agent loop (`agent/mod.rs`, ২৭৫৯ লাইন) ও `tools.rs` (২২১৩ লাইন) কি ভাঙার সময় হয়েছে?
2. Pace (`orient.rs`) এখন কথা দেখে ঠিক করে (মডেল call ছাড়া) — আরও নির্ভুল করার উপায়? Banglish-এ ভুল ধরা পড়ে কি?
3. Sub-agent শুধু read-only। লেখার কাজেও সমান্তরাল sub-agent নিরাপদে দেওয়া যায়? (worktree/checkpoint দিয়ে)
4. `remember` ও Memory: কী সংরক্ষণ হয়, কখন ভোলা হয়, প্রজেক্ট-ভিত্তিক না সব প্রজেক্টে? (কোডে `store/` দেখতে হবে — এই ফাইলে যাচাই হয়নি)
5. Verify-র reviewer engine বাছাই কীভাবে হয়? একটাই engine থাকলে কী হয়?
6. Cost governor-এর "route" (সস্তা মডেল সাজেশন) কি স্বয়ংক্রিয় হওয়া উচিত?
7. স্থানীয় ছোট মডেলে tool-calling নির্ভরযোগ্যতা (ANALYSIS ডক দেখুন) — fallback কী?
8. Night Guardian-এর auto-rollback ঝুঁকি, ও policy-র ডিফল্ট।
9. Anywhere-এ approval envelope replay/phishing থ্রেট মডেল (T28 ইত্যাদি) সম্পূর্ণ কি?
10. কোন স্তরগুলো ইতিমধ্যে আছে আর কোনগুলো শুধু প্ল্যানে (ROADMAP-v4 "বাস্তবায়নের অবস্থা" অংশ আর CHANGELOG মিলিয়ে দেখুন)?

## ১২. সীমাবদ্ধতা (এই ডকের)
- ফাইলের হেডার-কমেন্ট ও প্ল্যান ডকের শিরোনাম পড়ে বানানো; প্রতিটি ফাংশনের ভেতর পুরো পড়া হয়নি।
- সংখ্যা/ক্ষমতা সময়ের সাথে বদলায়; CHANGELOG.md মিলিয়ে নেওয়া ভালো।
- `app/src` (React UI)-এর ভেতরের বুদ্ধিমত্তা-সংশ্লিষ্ট অংশ শুধু ফাইলের নাম দেখে ধরা (`kernel/*`, `panels/prompt/*`, `store/agentIntents.ts`, `store/chatModel.ts`)।
