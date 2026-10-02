# বিশ্লেষণ রিপোর্ট — SDC-তে লোকাল মডেল (Qwen3.5-9B) ও কমান্ড-ভিত্তিক `/research`

তারিখ: রিপোর্ট প্রস্তুত হয়েছে কোডবেস (`H:\SDC\sdc`) পড়ে এবং লাইভ ওয়েব ডকুমেন্টেশন যাচাই করে।
উদ্দেশ্য: "লোকাল মডেল + কমান্ড-ভিত্তিক রিসার্চ" প্ল্যানটি SDC-র বর্তমান অবস্থার সাথে মিলিয়ে দেখা —
**এখন কী আছে, কী নেই, কী আপগ্রেড করতে হবে**, এবং বর্তমান run flow অটুট রেখে শুধু
provider-এর মতো করে লোকাল মডেল যোগ করার রাস্তা কী।

> এটি একটি বিশ্লেষণ ডকুমেন্ট। এখানে কোনো কোড বদলানো হয়নি।

---

## ০. এক লাইনে সিদ্ধান্ত

**আপনার প্ল্যানের প্রায় ৭০% SDC-তে ইতিমধ্যে তৈরি আছে** — Ollama provider, লোকাল স্ট্যাটাস-চেক UI,
স্ট্রিমিং, কনটেক্সট বাজেট, টুল-কলিং (হ্যাঁ, লোকাল মডেলেও), `web_search`/`web_fetch`, browser টুল,
sub-agent (`task`), প্ল্যান-টুল, cost governor, সেটিংস স্টোর, slash-command সিস্টেম — সব আছে।

**যা নেই, সেগুলো ছোট কিন্তু গুরুত্বপূর্ণ:**

| # | গ্যাপ | গুরুত্ব |
|---|---|---|
| 1 | `num_ctx` পাঠানো হয় না → Ollama তার **VRAM-ডিফল্ট** context-এ চলে (24 GiB-এর কম VRAM = **4k**), অথচ SDC ধরে নেয় **32k** → নীরবে ছাঁটাই | 🔴 ক্রিটিক্যাল |
| 2 | `qwen3.5:9b` catalogue-এ নেই | 🔴 ক্রিটিক্যাল |
| 3 | `/research` কমান্ড নেই | 🔴 মূল ফিচার |
| 4 | রিসার্চ লুপ (query → search → fetch → per-page summary → synthesis → sources) নেই | 🔴 মূল ফিচার |
| 5 | রিসার্চ লুপে **কোনো থামার সীমা নেই** (`DEFAULT_STEPS = usize::MAX`), এবং ফ্রি মডেলে cost budget থামাতে পারে না | 🔴 নিরাপত্তা |
| 6 | লোকাল মডেলে টুল-কলিংয়ের **এন্ড-টু-এন্ড টেস্ট নেই** (কোড-পথ আছে) | 🟡 যাচাই দরকার |
| 7 | সার্চ প্রোভাইডার pluggable নয় — DuckDuckGo HTML হার্ডকোডেড, ফলাফলে **তারিখ নেই** | 🟡 |
| 8 | উত্তরে structured source/citation নেই | 🟡 |
| 9 | "কমান্ড ছাড়া ইন্টারনেট নয়" — নীতিটা আছে (`privacy_local`), কিন্তু সেটি **স্থায়ী প্রজেক্ট-নীতি**, কমান্ড-ট্রিগার ওভাররাইড নেই | 🟡 |
| 10 | main-content extraction নেই (শুধু regex tag-strip); `PAGE_CAP`/`RESULT_CAP` লোকাল উইন্ডোর চেয়ে বড় | 🟡 |
| 11 | VRAM/OOM-এর আলাদা স্পষ্ট বার্তা নেই | 🟢 |
| 12 | ধাপভিত্তিক মডেল (সারাংশ=লোকাল, চূড়ান্ত=API) নেই — `sub_agent` একই backend/model ব্যবহার করে | 🟢 ঐচ্ছিক |

---

## ১. SDC-তে এখন ঠিক কী আছে (কোড প্রমাণসহ)

### ১.১ Provider সিস্টেম — ডাটা-ড্রিভেন, হার্ডকোডেড নয়

- ক্যাটালগ: `SDC/sdc/protocol/models.json` — এতে `providers` অ্যারে, প্রতিটির
  `id / label / live / protocol / models[]`।
- আজকের প্রোভাইডার তালিকা (১৪টি):
  `ollama`, `anthropic-api`, `openai-api`, `deepseek`, `groq`, `openrouter`, `google`,
  `xai`, `moonshot`, `mistral`, `qwen`, `qwen-coding`, `zai`, `custom`।
- **`ollama` provider ইতিমধ্যেই আছে** (`models.json`):
  ```json
  { "id": "ollama", "label": "Ollama",
    "live": "http://127.0.0.1:11434/api/tags",
    "protocol": "ollama",
    "models": [
      { "id": "llama3.2:3b",        "name": "Llama 3.2 3B",       "tier": "fast",     "ctx": 128000, "cost": "free" },
      { "id": "deepseek-coder:6.7b","name": "DeepSeek Coder 6.7B","tier": "balanced", "ctx": 16000,  "cost": "free" },
      { "id": "qwen2.5-coder:7b",   "name": "Qwen 2.5 Coder 7B",  "tier": "balanced", "ctx": 32768,  "cost": "free" } ] }
  ```
  (ফিল্ড: `id / name / tier / ctx / cost` — `tier` হলো `fast | balanced | deep` শ্রেণি,
  যা ModelSelector pill-এ দেখানো হয়।)
- `live` ফিল্ড = **স্ট্যাটাস চেক + ইনস্টল করা মডেলের লাইভ তালিকা** — প্ল্যানের "স্ট্যাটাস চেক"
  ফিচারটি এখানেই বাস্তবায়িত।
- `custom` provider আছে (মডেল সংখ্যা 0) → যেকোনো OpenAI-compatible base URL-এ যাওয়ার পথ খোলা।

### ১.২ ⭐ সবচেয়ে বড় আবিষ্কার: লোকাল মডেলে টুল-কলিং ইতিমধ্যে কাজ করে

কোডবেসে Ollama-র **দুটি সম্পূর্ণ আলাদা পথ** আছে, এবং এটা বোঝা অত্যন্ত জরুরি:

| পথ | ফাইল | এন্ডপয়েন্ট | টুল-কলিং | কীসে ব্যবহৃত |
|---|---|---|---|---|
| A | `sdcd/src/engines/ollama.rs` | `/api/chat` (NDJSON) | ❌ নেই | সাধারণ চ্যাট / এক-শট |
| B | `sdcd/src/agent/mod.rs` → `Backend::Ollama` | `/v1/chat/completions` (OpenAI-compatible) | ✅ **আছে** | পূর্ণ agent loop |

`agent/dialect.rs`-এর নিজের ডকুমেন্টেশন এটি নিশ্চিত করে (হুবহু উদ্ধৃত):
> *"**Anthropic** (`/v1/messages`): tools are `{name, description, input_schema}`, the reply is a list
> … and **Ollama's own `/v1` endpoint**): tools are `{type: "function", function: {…}}`,
> a tool call arrives …"*

এবং Ollama-র একটি পরিচিত খুঁটিনাটি ইতিমধ্যে হ্যান্ডেল করা আছে:
> *"**Ollama and some local servers leave the id out**; the loop needs one to pair the answer."*

অর্থাৎ লোকাল মডেলে টুল-কলিং শুধু "থিওরিটিক্যালি সম্ভব" নয় — **বাস্তব Ollama আচরণের
বিশেষত্ব জেনে কোড লেখা হয়েছে**।

- `agent/mod.rs:73` এ `pub enum Backend` — `Api` ও `Ollama`।
- Agent loop টি Ollama হলেও `native_api::open_stream` (`engines/native_api.rs`) দিয়ে যায়,
  যেটা HTTP-এ সরাসরি POST করে SSE লাইন পড়ে এবং `tool_calls` জোড়া লাগায়
  (`agent/dialect.rs` — `fn body`, `fold_old_results`, OpenAI/Anthropic দুই ডায়ালেক্ট)।
- `agent/mod.rs` → `for step in 1..=options.max_steps { … }` — পূর্ণ মাল্টি-স্টেপ লুপ।
- ⭐ `agent/mod.rs:709` — **context window ইতিমধ্যে backend অনুযায়ী আলাদাভাবে হিসাব হয়:**
  ```rust
  let window = crate::context::window_tokens(
      if backend == Backend::Ollama { "ollama" } else { "native_api" },
      prompt.provider.as_deref(), &target.model);
  ```
  → **এটাই `num_ctx` পাঠানোর প্রাকৃতিক হুক পয়েন্ট**: `window` ইতিমধ্যে গণনা করা আছে,
  শুধু সেটি রিকোয়েস্ট বডিতে বসাতে হবে। পরিবর্তনটি ছোট এবং শুধু Ollama শাখাকে ছোঁয়।
- `agent/mod.rs:160, 709, 989, 1034` — চার জায়গায় `Backend::Ollama` রেফারেন্স।
- ⚠️ **সৎ সংশোধন:** `agent/mod.rs:1034`-এর টেস্টটি টুল-কলিং যাচাই করে **না** —
  এটি `run(Backend::Ollama, Options { autonomy: Autonomy::Ask, max_steps: 5, … })` কল করে
  শুধু এই assert করে যে ফোল্ডার খোলা না থাকলে `"Open a folder"` বার্তা আসে।
  অর্থাৎ **লোকাল ব্যাকএন্ডে টুল-কলিংয়ের কোড-পথ আছে, কিন্তু এন্ড-টু-এন্ড টেস্ট নেই** —
  আপনার চেকলিস্টের "মডেলের টুল-কলিং আপনার সংস্করণে কাজ করে" ধাপটি তাই
  **সত্যিই হাতে করে যাচাই করতে হবে** (নিচে ধাপ ১)।

**ফলাফল:** আপনার প্ল্যানের "টুল-কলিং যাচাই" ধাপটি নতুন করে বানাতে হবে না —
শুধু আপনার Ollama সংস্করণে `qwen3.5:9b`-এর সাথে **রানটাইমে যাচাই** করতে হবে।
কোড-লেভেলে পথটি খোলা আছে।

### ১.৩ টুলের পূর্ণ তালিকা (মডেল যা পায়)

`sdcd/src/agent/tools.rs` থেকে (`name:` ঘোষণা অনুযায়ী):

`browser`, `apply_patch`, `task`, `view_image`, `screenshot`, `grep`, `glob`,
`web_fetch`, `web_search`, `start_process`, `process_output`, `stop_process`,
`ask_user`, `remember`, `read_file`, `list_dir`, `search`, `git_diff`,
`write_file`, `edit_file`, `run_command`, `update_plan`

রিসার্চ মডিউলের জন্য গুরুত্বপূর্ণ চারটি **আগে থেকেই আছে**:
- `web_search` — সার্চ (ডিফল্ট ৮টি রেজাল্ট)
- `web_fetch` — পেজ নামানো
- `task` — **sub-agent স্পন করা** (রিসার্চের প্রতিটি পেজ আলাদাভাবে সারাংশ করার জন্য আদর্শ)
- `update_plan` — ধাপগুলো UI-তে দেখানো

**`task` টুলটি আক্ষরিক অর্থেই রিসার্চের জন্য লেখা** — `agent/tools.rs`-এ `fn task_spec()`-এর
বর্ণনা (হুবহু উদ্ধৃত):
> *"Hand a self-contained **research** or exploration job to a sub-agent with its own
> **fresh context** and read-only tools (read, list, grep, glob, web). It answers with a report…
> several task calls in one reply run in **parallel**."*

ডেমন-সাইডে এর বাস্তবায়নও আছে: `agent/mod.rs`-এ `fn sub_agent(…)`, `fn run_tasks(…)`,
`fn sub_agent_prompt(…)`, এবং `tools::specs_for(Caps { vision, subagent: true, patch: false })`
→ sub-agent শুধু **read-only** টুল পায় (webসহ), ফাইল লিখতে পারে না।

🔴 **একটি সীমাবদ্ধতা জানা জরুরি:** `sub_agent(backend, target, …)` — অর্থাৎ sub-agent
**একই backend ও একই model** ব্যবহার করে। তাই প্ল্যানের "চূড়ান্ত বিশ্লেষণে আলাদা API মডেল"
আজ সম্ভব নয়; সেটি ধাপ ১২-এর কাজ।
✅ **তবে ভালো খবর:** প্রতিটি sub-agent-এর **নিজস্ব fresh context** থাকায়, প্ল্যানের
"৯B মডেলের কনটেক্সট ছোট, তাই প্রতিটি পেজ আলাদাভাবে সারাংশ করতে হবে" শর্তটি
**এই অবকাঠামোতেই প্রাকৃতিকভাবে পূরণ হয়** — এক পেজ = এক `task` = এক আলাদা context উইন্ডো,
এবং একাধিক `task` সমান্তরালে চলে।

### ১.৪ Web টুলের বর্তমান বাস্তবায়ন

`sdcd/src/agent/web.rs`:
- সার্চ: `https://html.duckduckgo.com/html/?q=…` — **কী লাগে না, অ্যাকাউন্ট লাগে না**
  (ফাইলের মন্তব্যেই লেখা: *"from DuckDuckGo's HTML endpoint (no key, no account)"*)।
  `web_search` ডাকে `super::web::search(query, 8)` → **৮টি রেজাল্ট** (hard-coded)।
- রেজাল্ট পার্স: `class="result__a"` / `result__snippet` — অর্থাৎ **HTML স্ক্র্যাপিং**,
  `//duckduckgo.com/l/?uddg=` রিডিরেক্ট আনর‍্যাপ করা হয়।
- পেজ ফেচ (`web.rs`, যাচাই করা ধ্রুবক):
  - `const PAGE_CAP: usize = 60_000;` (লেখার অক্ষর-সীমা)
  - `const BYTES_CAP: u64 = 4 * 1024 * 1024;` (ডাউনলোড সীমা)
  - নিজস্ব `USER_AGENT` (Chrome + `SDC-Agent`)
  - **SSRF গার্ড আছে এবং টেস্টসহ**: `localhost`, `.localhost`, `127.*`, `169.254.*`,
    `192.168.*`, `172.x`, `[::1]`, `file://` — সব প্রত্যাখ্যাত
    (`fn private_addresses_are_refused`)। শুধু `http://` ও `https://` চলবে।
  - ট্যাগ-স্ট্রিপিং করা হয় (regex), কিন্তু
    **Readability/trafilatura-র মতো real main-content extraction নেই** — মেনু, ফুটার,
    nav সব একসাথে ঢোকে।
- **JavaScript রেন্ডারিং:** `web_fetch`-এ নেই। তবে আলাদা `browser` টুল আছে
  (`sdcd/src/agent/browser.rs`) — headless ব্রাউজার, screenshot, click, type।
  `browser.rs:60`-এ vision-capable মডেলের তালিকায় **`qwen3.5` ইতিমধ্যে উল্লেখ আছে**।

### ১.৫ কনটেক্সট ব্যবস্থাপনা — ভালো কাঠামো, কিন্তু একটি মারাত্মক mismatch

`sdcd/src/context.rs`-এর ডকুমেন্টেশন (ফাইলের শুরুতে) তিনটি কৌশল বর্ণনা করে —
Claude Code-এর মতো: **resume** (CLI নিজের কথোপকথন চালিয়ে যায়), **compact** (`/compact`
সারাংশ রাখে, পরের টার্নগুলো সেখান থেকে শুরু), এবং **fit** (*"whatever is sent is fitted to
the model's window: the newest turns verbatim, the older ones as a digest"*).

- `const HISTORY_SHARE: f64 = 0.45;` — উইন্ডোর ৪৫% কথোপকথন, বাকিটা system prompt,
  tools, পড়া ফাইল ও উত্তরের জন্য।
- `pub fn window_tokens(engine, provider, model) -> u64` — catalogue-এর `ctx` থেকে নেয়;
  না পেলে engine-ভিত্তিক ডিফল্ট: `claude_code → 200_000`, `codex → 272_000`,
  `gemini → 1_000_000`, **`ollama → 32_000`**, বাকি `→ 128_000`।
- `pub fn history_budget(window)` = `window × 0.45`
- `pub fn fit(turns, summary, budget) -> Fitted` ও `fn digest(turns, budget)` —
  নতুন টার্ন হুবহু, পুরনোগুলো digest আকারে।
- `keep_small` (`agent/mod.rs`) — বড় টুল আউটপুট ছাঁটাই;
  `fold_old_results` (`agent/dialect.rs`) — পুরনো টুল রেজাল্ট ভাঁজ করা।
- `/compact` কমান্ড আছে (`app/src/panels/prompt/slash.ts:39` →
  `return { kind: 'send', prompt: '/compact', compact: true }`), এবং সারাংশ SQLite-তে
  `compact.{session_id}` key-তে সংরক্ষিত হয় (`save_compaction`)।

🔴 **মারাত্মক mismatch (এটাই P0-১-এর আসল যুক্তি):**

| | মান |
|---|---|
| SDC ধরে নেয় ollama উইন্ডো = | **32,000 টোকেন** (`context.rs`, `window_tokens`-এর ডিফল্ট) |
| → তাই history budget ধরে নেয় = | 32,000 × 0.45 = **14,400 টোকেন** |
| Ollama আসলে দেয় (`num_ctx` না পাঠালে, VRAM-নির্ভর) = | **< 24 GiB VRAM-এ 4,096** (২.১-এ টেবিল) |

অর্থাৎ একটি সাধারণ ল্যাপটপে (24 GiB-এর কম VRAM) SDC **৮ গুণ বেশি** পাঠানোর পরিকল্পনা করে,
কিন্তু Ollama সেটি **নীরবে কেটে দেয়**।
`fit()` কাজ করছে — শুধু **ভুল সংখ্যা দিয়ে**। এটি একটি সাধারণ "ফিচার নেই" সমস্যা নয়,
এটি একটি **নির্ভুলতার (correctness) সমস্যা**: ছাঁটাই/ভাঁজ করার পুরো যুক্তিই অকার্যকর হয়ে যায়,
কারণ SDC জানে না যে প্রাপক আসলে কত ধরে। (উচ্চ-VRAM মেশিনে 32k/256k পেলে সমস্যা কম,
কিন্তু SDC তা জানেও না, তাই নির্ভরযোগ্যভাবে ঠিক থাকে না।)

**সমাধান দুই অংশে, এবং দুটিই ছোট:**
1. `num_ctx = window` (বা একটি সিলিং) রিকোয়েস্টে পাঠানো → Ollama-কে SDC-র ধারণার সাথে মেলে।
   হুক পয়েন্ট আগে থেকেই আছে: `agent/mod.rs:709`-এ `window` গণনা করা হয়।
2. catalogue-এ `qwen3.5:9b`-এর `ctx` একটি **বাস্তবসম্মত মান** (যেমন 16000) দেওয়া —
   তাহলে `window_tokens` catalogue থেকে সেটিই নেবে (loop-টি catalogue আগে দেখে) এবং
   VRAM-ও নিরাপদ থাকবে।

⚠️ **দুইটি token-counter অসামঞ্জস্য** (জানলে ভালো):
- `context::tokens_of` → `ascii/4 + other + 1` (বাংলা ≈ **১ টোকেন/অক্ষর**)
- `trust::cost::tokens_in` → `latin/4 + other/2 + 1` (বাংলা ≈ **০.৫ টোকেন/অক্ষর**)

প্রথমটি budget/fold-এর জন্য, দ্বিতীয়টি খরচ-অনুমানের জন্য। রিসার্চে বাংলা টেক্সট নিয়ে
কাজ করলে **budget-গণনাটিই (context.rs) গুরুত্বপূর্ণ** এবং সেটি রক্ষণশীল (বেশি ধরে) —
ভালো দিক। তবে দুটি এক করা ভবিষ্যতে বিবেচ্য।

### ১.৬ কমান্ড সিস্টেম

`app/src/panels/prompt/slash.ts` — slash menu, autocomplete, এবং দুই ধরনের কমান্ড
(নির্ভুল তালিকা `slash.ts:35-92`-এর `switch` থেকে):
- বিল্ট-ইন: `/compact`, `/init`, `/review`, `/remember`, `/memory`, `/clear`, `/help`
- **প্রজেক্ট কমান্ড:** `.sdc/commands/<name>.md` (বা `.claude/commands`) থেকে
  ডাইনামিকভাবে তালিকা হয়, `$ARGUMENTS` প্রতিস্থাপিত হয় —
  মানে কমান্ড সিস্টেম **এক্সটেনসিবল, কোড বদল ছাড়াই নতুন কমান্ড চালানো যায়**।
- অচেনা `/…` → `return { kind: 'send', prompt: text }` — engine-এ যেমন আছে তেমনই চলে যায়।
- `/research` নেই; `/model`-ও slash কমান্ড নয় (মডেল বাছাই toolbar dropdown দিয়ে, নিচে ১.৭)।

### ১.৬(ক) মডেল বাছাই (Model picker)

- `app/src/panels/prompt/ModelSelector.tsx` — toolbar-এর pill: `[tier icon] Balanced · engine · model`
- `app/src/panels/prompt/ModelDropdown.tsx` — তালিকা; `app/src/store/model.ts` — ক্যাটালগ স্টোর
- `app/src/store/intents.ts:321` `chooseModel(modelId, providerId)` →
  `sdcpCall('models.select', { modelId, providerId })`
  → **ডেমন সেটিংস + উইন্ডোর নিজের store, দুই জায়গাতেই লেখে** (একটি পুরনো বাগের
  মন্তব্যসহ নথিবদ্ধ)। `releaseModel(… remove: true)` দিয়ে তালিকা থেকে সরানো যায়।
- অর্থাৎ আপনার প্ল্যানের **`/model` কমান্ডের কাজটি ইতিমধ্যে UI-তে সম্পূর্ণ আছে** —
  শুধু একটি slash এন্ট্রি যোগ করলে কীবোর্ড থেকেও হবে।

### ১.৭ সেটিংস স্টোর

- SQLite-এ `settings (key, value, updated_at)` টেবিল — `store/` এ
  `pub fn setting(&self, key)` ও `pub fn set_setting(&self, key, value)`।
- SDCP মেথড: `settings.set` (`sdcp/methods.rs`), UI থেকে
  `app/src/store/kernelIntents.ts` → `daemonSetting(key, value)` → `sdcpCall('settings.set', …)`।
- প্রোভাইডার কী: `sdc.provider.<id>` প্যাটার্নে (যেমন `sdc.provider.deepseek`)।
- মডেল বাছাই: `models.select` মেথড (`sdcp/methods.rs`), `models.list { refresh: true }`
  দিয়ে লাইভ তালিকা + ক্যাশ।
- **নতুন সেটিংস যোগ করা খুবই সহজ** — শুধু একটি নতুন key, কোনো schema বদল লাগে না।

### ১.৮ UI — Provider Hub ও Local ট্যাব

`app/src/modals/ProviderHub.tsx` — provider তালিকা, API key ইনপুট, এবং
**Local/Ollama-র জন্য status + doctor** (ডেমন চলছে কিনা, কোন মডেল নামানো আছে)।
Settings-এ Agent ট্যাব (`app/src/kernel/AgentSettings.tsx`) — autonomy, budget ইত্যাদি।

### ১.৯ Cost governor — ⭐ pre-flight estimate আগে থেকেই আছে ও UI-তে দেখায়

`sdcd/src/trust/cost.rs` — প্ল্যানের "API মডেল বাছা থাকলে সম্ভাব্য খরচ আগে জানাবে" নীতির
**পুরো কাঠামোটি ইতিমধ্যে তৈরি**:

- `pub fn estimate(engine, provider, model, prompt, history_chars, agent) -> Value` —
  মন্তব্য: *"What a turn will probably cost, **before it runs**."*
  ফেরত দেয় `inputTokens`, `outputTokens`, `usd`, `source` (`estimate`/`subscription`/`unknown`),
  `complexity`।
- `pub fn price_of(provider, model)` — মন্তব্য: *"**A local model is free, and says so**"* —
  `cost == "free"` হলে `(0.0, 0.0)`। অর্থাৎ **লোকাল মডেল = খরচ ০, স্বয়ংক্রিয়ভাবে**।
- `pub fn tokens_in(text)` — ⭐ **বাংলার জন্য বিশেষভাবে টিউন করা token counter!**
  মন্তব্য: *"about four characters per token for Latin script, **about two for the scripts a
  tokenizer splits finer (Bengali, Devanagari, Arabic, CJK)**"* →
  `latin / 4 + other / 2 + 1`।
- `check_before(store, session, max_turn_usd, estimate_usd)` — বাজেট ছাড়ালে টার্ন আটকে দেয়।
- `pub fn cheaper(…)` — সস্তা বিকল্প মডেল প্রস্তাব করে (router)।
- SDCP মেথড: `cost.estimate` → UI intent `estimateCost(...)`
  (`app/src/store/kernelIntents.ts:107-109`)
- UI: `app/src/kernel/CostMeter.tsx` (StatusBar-এ বসে, `running?.estimate?.usd` —
  অর্থাৎ **টার্ন চলাকালীন** estimate দেখায়, শুরুর আগে নয়),
  `app/src/kernel/ProofTab.tsx`-এ `estimateOnly(usd)` = *"Estimated $x before it ran"*,
  বাংলা অনুবাদ: `app/src/locales/bn.ts` → `চলার আগে আনুমানিক ${usd}`

⚠️ **সঠিক অবস্থা (গুরুত্বপূর্ণ):** ব্যাকএন্ড (`cost.estimate`) ও UI intent
(`estimateCost`) দুটোই আছে, **কিন্তু `estimateCost` কোথাও কল হয় না** —
পুরো `app/src`-এ grep করলে শুধু তার ঘোষণাটিই মেলে, কোনো ব্যবহার নয়।
অর্থাৎ pre-flight estimate **পাইপ তৈরি কিন্তু সংযুক্ত নয়**।

✅ **তবু ভালো খবর:** P1-এর ধাপ ৯ ("pre-flight খরচ কার্ড") প্রায় বিনামূল্যে পাওয়া যাবে —
`/research` শুরুর আগে বিদ্যমান `estimateCost(...)` কল করে ফলাফল দেখালেই হবে
(নতুন ডেমন-কাজ লাগবে না)। শুধু research loop-এর জন্য `steps` অনুমান
(বর্তমানে `estimate`-এ hard-coded) বদলে প্রকৃত max_searches/max_pages দিতে হবে।

### ১.৯(ক) বাংলা ও Banglish সাপোর্ট — প্রত্যাশার চেয়ে ভালো অবস্থা

আপনার প্ল্যানে লেখা ছিল *"Banglish বা বাংলা প্রশ্নে মডেলের সাড়া এখনও পরীক্ষা করা হয়নি"* —
কোডবেস পড়লে দেখা যায় **SDC নিজেই বাংলা-সচেতন**:

- **১০টি ভাষা প্যাক:** `app/src/locales/` → `ar, bn, es, fr, hi, id, pt, ur, zh` (+ `en` ডিফল্ট)।
  **`bn.ts` = 56 KB — সবচেয়ে বড় প্যাক**, অর্থাৎ বাংলা SDC-র প্রথম শ্রেণির ভাষা
  (`providers: { title: 'প্রোভাইডার ও মডেল' }` ইত্যাদি)।
- **Banglish শব্দ কোডে স্বীকৃত:** `trust/cost.rs`-এর `complexity()`-এর heavy-word তালিকায়
  `"banao"`, `"baniye"`, `"toiri"`, `"তৈরি"`, `"সম্পূর্ণ"`, `"পুরো"` — অর্থাৎ Roman-হরফে
  লেখা বাংলা নির্দেশ SDC বুঝতে শেখানো হয়েছে।
- **Token counter বাংলায় আলাদা হিসাব করে** (উপরে ১.৯) — এটি context budget-এর জন্য
  অত্যন্ত গুরুত্বপূর্ণ, কারণ বাংলা টেক্সট প্রতি অক্ষরে বেশি টোকেন নেয়।

⚠️ **তবুও যা অপরীক্ষিত:** লোকাল `qwen3.5:9b` বাংলা/Banglish প্রম্পটে কতটা ভালো
**সারাংশ ও query decomposition** করে — এটি মডেলের গুণ, SDC-র নয়; অবশ্যই রানটাইমে পরীক্ষা করতে হবে।
তবে `qwen3.5` মাল্টিলিংগুয়াল হিসেবেই পরিচিত, তাই সম্ভাবনা ভালো।

### ১.১০ Autonomy / network policy

`sdcd/src/trust/policy.rs` → `pub fn privacy_local(&self) -> bool { self.privacy == "local-only" }`।
`agent/tools.rs`-এ তিন জায়গায় এটি web বন্ধ করে (নির্ভুল উদ্ধৃতি):
```rust
fn web_fetch(…) { …
    if context.policy.privacy_local() {
        return failed_with(context, call_id, "This project's policy keeps everything on this
            machine (privacy = local), so the web is not read.");
    } … }
fn web_search(…) { … "…so the web is not searched." … }
// browser `open` একটি পাবলিক URL-এ: "…so the browser does not open public sites."
```

⚠️ **গুরুত্বপূর্ণ সূক্ষ্মতা:** `privacy` আজ একটি **প্রজেক্ট-স্তরের স্থায়ী নীতি** —
এটি `.sdc/policy.toml` ফাইলের `privacy = "local-only"` লাইন থেকে পড়া হয়
(`policy.rs`, টেস্ট: `production_caps_auto_and_privacy_keeps_code_local`)।
এটি প্রতি-টার্ন/সেশন ওভাররাইড নয়। একই নীতি **মডেল বাছাইও আটকায়**:
> *"This project is private (privacy = "local-only" …): **only a local model may read it.**
> Pick an Ollama model, or change the policy."*

**এর অর্থ আপনার প্ল্যানের জন্য:** ভিত্তিটা ঠিক আছে, কিন্তু এটি "সবসময় বন্ধ" মডেল —
"শুধু `/research` কমান্ডে খুলবে" নয়। তাই যা করতে হবে:
1. `privacy_local` কে ডিফল্ট-**অন** রেখে একটি **রানটাইম ওভাররাইড** যোগ করা
   (যেমন `ToolContext`-এ `network_override: bool`, যা শুধু একটি সক্রিয় research job চলাকালীন `true`),
   অথবা
2. research job-কে আলাদা একটি `ToolContext`/policy snapshot দেওয়া যেখানে `privacy` শিথিল,
   কিন্তু job শেষে মূল সেশন আবার পুরনো নীতিতে ফিরে যায়।

দুটি পথেই **বাকি সব ফ্লো (কোডিং, সাধারণ চ্যাট) ১০০% অপরিবর্তিত থাকবে** — আপনার মূল শর্ত পূরণ হয়।

### ১.১১ Cancellation ও ব্যাকগ্রাউন্ড জব

- Stop বাটন → SDCP cancel → engine পর্যন্ত পৌঁছায় (`engines/cancel.rs`)।
- `agent/background.rs` — `start/output/stop/stop_all/list`, `MAX_RUNNING = 12` →
  **দীর্ঘস্থায়ী রিসার্চ জব চালানোর রানার আগে থেকেই আছে**।

---

## ২. ওয়েব রিসার্চ — যাচাই করা তথ্য

### ২.১ Qwen3.5-9B ও Ollama

- Ollama লাইব্রেরিতে **`qwen3.5:9b` সত্যিই আছে** (tags: `9b`, ভিন্ন quant), এবং এটি
  **tools/function-calling সাপোর্ট করে**; মডেলটি context-এ **256K পর্যন্ত** সক্ষম।
- 🔴 **SDC আজ `num_ctx` পাঠায় না** — পুরো কোডবেসে `num_ctx` / `num_predict` /
  `OLLAMA_CONTEXT_LENGTH` **একটিও মিল নেই** (grep-এ যাচাই করা; শুধু অপ্রাসঙ্গিক মিল)।

- ⚠️ **সংশোধিত তথ্য (Ollama-র অফিসিয়াল doc থেকে হুবহু):** ডিফল্ট context একটি
  স্থির সংখ্যা নয়, এটি **VRAM-নির্ভর**:

  | VRAM | Ollama-র ডিফল্ট context |
  |---|---|
  | < 24 GiB | **4k** |
  | 24–48 GiB | 32k |
  | ≥ 48 GiB | 256k |

  অর্থাৎ বেশিরভাগ সাধারণ ল্যাপটপ/ডেস্কটপে (24 GiB-এর কম VRAM) আপনি **4k** পাবেন।

- 🚨 **সবচেয়ে গুরুত্বপূর্ণ লাইনটি** (Ollama doc, হুবহু উদ্ধৃত):
  > *"Tasks which require large context like **web search, agents, and coding tools** should be
  > set to **at least 64000 tokens**."*

  → **আপনার প্ল্যানে লেখা "৮k–১৬k টোকেনের মধ্যে ইনপুট রাখা" — Ollama নিজেই বলেছে যে
  web-search/agent কাজের জন্য সেটি যথেষ্ট নয়, কমপক্ষে 64k দরকার।**
  এটি প্ল্যানের একটি অনুমান যা **পুনর্বিবেচনা করতে হবে** (নিচে বিস্তারিত)।

- **SDC-র সাথে সংঘর্ষ:** SDC ধরে নেয় ollama উইন্ডো = **32,000** (`context.rs`),
  Ollama 24 GiB-এর কম VRAM-এ দেয় **4,096** → ৮ গুণ ফারাক, নীরবে ছাঁটাই (১.৫-এ বিস্তারিত)।

- **সেট করার তিনটি উপায়** (doc অনুযায়ী):
  1. Ollama অ্যাপের settings slider
  2. `OLLAMA_CONTEXT_LENGTH=64000 ollama serve` (env)
  3. প্রতি-রিকোয়েস্টে `options: { "num_ctx": N }`
  → **SDC-র জন্য ৩ নম্বরই সঠিক**, কারণ ১ ও ২ ব্যবহারকারীর মেশিন সেটিং —
  SDC সেটি নিয়ন্ত্রণ করে না, এবং একই Ollama-তে অন্য অ্যাপও চলতে পারে।

- **যাচাই:** `ollama ps`-এর `CONTEXT` ও `PROCESSOR` কলাম —
  doc-এর উদাহরণ: `gemma4:latest … 9.6 GB … 100% GPU … 131072 … 2 minutes from now`।
  `PROCESSOR`-এ CPU অংশ দেখা মানে **offloading** → গতি নাটকীয়ভাবে কমবে।

- Ollama-র নিজস্ব web-search capability আছে (ডেমন-সাইড `search`/`fetch` টুল) — কিন্তু এটি
  SDC-র pluggable-provider নকশা, privacy gate ও cost governor-এর বাইরে চলে যাবে,
  তাই **সুপারিশ নয়**।

#### ⚖️ 8k–16k বনাম 64k — সৎ দ্বন্দ্ব ও সমাধান

| দিক | যুক্তি |
|---|---|
| প্ল্যান বলে 8k–16k | VRAM সাশ্রয়; ৯B মডেল সাধারণ ল্যাপটপে চলবে; প্রতি পেজ আলাদাভাবে সারাংশ করলে ছোট উইন্ডোই যথেষ্ট |
| Ollama বলে ≥64k | agent loop-এ system prompt + ~22 টুলের JSON schema + কথোপকথন + টুল ফলাফল একসাথে জমে |

**সমাধান (এবং এটি আপনার প্ল্যানের ধাপ ৪-কেই সঠিক প্রমাণ করে):**
উইন্ডো বড় না করে **প্রতিটি ধাপে পাঠানো জিনিস ছোট করা** —
1. **`task` sub-agent ব্যবহার করুন** (আগে থেকেই আছে, ১.৩-এ দেখুন): প্রতিটি পেজের
   সারাংশ **আলাদা fresh context**-এ চলে, তাই একটি পেজের ২৪k অক্ষর মূল এজেন্টের
   উইন্ডোতে কখনো জমে না — শুধু ছোট সারাংশটি ফেরে।
2. **টুলের তালিকা ছাঁটুন:** research-এর জন্য ২২টি টুল নয়, শুধু
   `web_search`, `web_fetch`, `task`, `update_plan`, `read_file` — schema-র আকার
   কয়েক গুণ কমে যায়।
3. **`PAGE_CAP`/`RESULT_CAP` লোকাল প্রোফাইলে কমান** (আজ 60,000 / 24,000 অক্ষর —
   4k উইন্ডোর চেয়ে বড়!)।
4. তারপর একটি **মাঝামাঝি `num_ctx`** (যেমন 16,384–32,768) বাস্তবসম্মত;
   64k লাগবে না যদি উপরের তিনটি করা হয়।

**এটাই P0-১ ও P1-এর প্রকৃত নকশা-সিদ্ধান্ত** — শুধু `num_ctx` বাড়ালেই হবে না,
বরং *কতটুকু পাঠানো হচ্ছে* সেটাও কমাতে হবে, নইলে VRAM শেষ হয়ে CPU offload হবে।

### ২.২ সার্চ প্রোভাইডার — যাচাই করা দাম ও লিমিট

| প্রোভাইডার | ফ্রি লিমিট | দাম | কী লাগে | মন্তব্য |
|---|---|---|---|---|
| **DuckDuckGo HTML** (SDC-র বর্তমান) | অসীম (অনানুষ্ঠানিক) | ০ | কিছুই না | ✅ শূন্য খরচ, কিন্তু HTML স্ক্র্যাপিং → bot-block/rate-limit হতে পারে, গঠন বদলালে ভাঙবে, কোনো তারিখ মেটাডেটা নেই |
| **SearXNG** (self-host) | অসীম (নিজের সার্ভার) | ০ (হোস্টিং ছাড়া) | Docker/সার্ভার | ✅ সবচেয়ে নমনীয়; `GET /search?q=…&format=json` — **কিন্তু `settings.yml`-এ `json` format চালু করতে হয়**, না হলে `403 Forbidden`। পাবলিক ইনস্ট্যান্সে সাধারণত বন্ধ |
| **Tavily** | **1,000 credits/মাস**, কার্ড লাগে না | PAYG **$0.008/credit**; Project প্ল্যানে 4,000 credits/মাস | API key | ✅ agent-কেন্দ্রিক — search + extract এক API-তে, credit মাসের ১ তারিখে রিসেট; স্টুডেন্টদের জন্য ফ্রি |
| **Brave Search API** | **প্রতি মাসে $5 ফ্রি ক্রেডিট** | **$5 / 1,000 রিকোয়েস্ট** (৫০ qps); `Answers` $4/1,000 + $5/1M টোকেন | API key + **কার্ড লাগে** (anti-fraud, চার্জ হয় না) | ✅ নিজস্ব ইনডেক্স (40B+ পেজ); `res/v1/llm/context` নামে আলাদা agentic endpoint আছে |
| **Serper** | **2,500 ফ্রি কোয়েরি** (এককালীন, কার্ড লাগে না) | তারপর সস্তা per-query | API key | Google SERP; 1–2s latency; `organic[]` এ `title/link/snippet/date` |

**সুপারিশ:** ডিফল্ট **DuckDuckGo** (শূন্য খরচ, কিছু লাগে না) রেখে **provider interface** বানানো,
যাতে ব্যবহারকারী Settings থেকে SearXNG / Tavily / Brave / Serper বেছে নিতে পারে।
গুণমান ও নির্ভরযোগ্যতার দিক থেকে **Serper (2,500 ফ্রি) → Tavily (1,000/মাস)** সবচেয়ে ভালো
এন্ট্রি পয়েন্ট; সম্পূর্ণ প্রাইভেসি চাইলে **SearXNG**।

### ২.৩ পেজ এক্সট্রাকশন

- `trafilatura` (Python) — সবচেয়ে ভালো main-content extraction, কিন্তু **Python নির্ভরতা**
  যোগ করে (SDC একটি Rust ডেমন; নতুন ভাষা-রানটাইম বড় সিদ্ধান্ত)।
- **`readability` (Rust crate)** বা হাতে লেখা heuristic — Rust-এ থাকে, নতুন বাইনারি লাগে না।
- JS-নির্ভর সাইট: SDC-র `browser` টুল (headless) **আগে থেকেই আছে** → Playwright আলাদা করে
  বসাতে হবে না, শুধু fallback হিসেবে যুক্ত করতে হবে।

---

## ৩. গ্যাপ বিশ্লেষণ — প্ল্যানের প্রতিটি ফিচার vs SDC

### ৩.১ "লোকাল মডেল সংযোগের ৬টি ফিচার"

| প্ল্যানের ফিচার | SDC-তে আছে? | কোথায় / কী করতে হবে |
|---|---|---|
| **লোকাল সার্ভার সংযোগ** | ✅ আছে | `engines/ollama.rs` (`/api/chat`) + `agent/mod.rs` `Backend::Ollama` (`/v1/chat/completions`)। `live: http://127.0.0.1:11434/api/tags` |
| **স্ট্যাটাস চেক** | ✅ আছে, ভালোভাবে | `engines/ollama.rs`: `pub const ENDPOINT = "127.0.0.1:11434"`, `pub fn daemon_running()` (মন্তব্য: *"the Local flow's **first doctor row**"*), `GET /api/tags` দিয়ে ইনস্টল করা মডেল (*"the Local flow's **second doctor row**"*), `providers/mod.rs`-এ `"local" if daemon_running() => "connected"`। ⚠️ শুধু catalogue-এ `qwen3.5:9b` নেই |
| **স্ট্রিমিং উত্তর** | ✅ আছে | `native_api::open_stream` → SSE লাইন → UI token delta |
| **কনটেক্সট সীমা নিয়ন্ত্রণ** | ⚠️ আংশিক | `context.rs` + `keep_small` + `fold_old_results` আছে, **কিন্তু `num_ctx` পাঠানো হয় না** → 🔴 যোগ করতেই হবে |
| **টুল-কলিং যাচাই** | ✅ কোড-পথ আছে, ❌ এন্ড-টু-এন্ড টেস্ট নেই | `dialect.rs` OpenAI-স্টাইল `tools`/`tool_calls` পাঠায় এবং মন্তব্যে Ollama-র `/v1` ও "id বাদ পড়া"র বিশেষত্ব স্বীকার করে (উদ্ধৃতি ১.২-এ)। **কিন্তু `agent/mod.rs:1034`-এর `Backend::Ollama` টেস্টটি টুল-কলিং যাচাই করে না** — তাই রানটাইম যাচাই বাধ্যতামূলক (ধাপ ১) |
| **ত্রুটি বার্তা** | ⚠️ বেশিরভাগ আছে | ✅ **সার্ভার বন্ধ:** `fn not_running()` → *"Ollama is not running. Start it with `ollama serve` (expected at http://127.0.0.1:11434)."* — একটি timeout-এর চেয়ে ভালো উত্তর, কোডের মন্তব্যেই লেখা। ✅ **2s connect + 60s read timeout**। ✅ **HTTP error body:** `value.get("error")` হলে সেটিই `EngineEvent::Failed` হয়ে যায়। ✅ **ফাঁকা উত্তর:** *"Ollama answered with nothing usable."* (কারণসহ: turn শেষ, timeout-এর জন্য অপেক্ষা নয়)। ❌ **VRAM/OOM শনাক্ত করা নেই** (error টেক্সটে "out of memory"/"VRAM" খোঁজা হয় না) |

### ৩.২ কমান্ড সিস্টেম

| কমান্ড | অবস্থা | কী করতে হবে |
|---|---|---|
| `/research <প্রশ্ন>` | ❌ নেই | `app/src/panels/prompt/slash.ts`-এ একটি `case 'research'` + ডেমনে একটি research driver |
| `/research stop` | ⚠️ ভিত্তি আছে | Stop/cancel পুরোপুরি কাজ করে (`engines/cancel.rs`); শুধু research job-এ ম্যাপ করতে হবে (`agent/background.rs`-এর `stop/stop_all` ব্যবহারযোগ্য) |
| `/model` | ❌ slash কমান্ড হিসেবে নেই, কিন্তু **কাজটা UI দিয়ে হয়** | মডেল বাছাই হয় prompt toolbar-এর `ModelSelector` dropdown (`app/src/panels/prompt/ModelSelector.tsx`, `ModelDropdown.tsx`) → `chooseModel()` (`app/src/store/intents.ts:321`) → `sdcpCall('models.select', { modelId, providerId })`। চাইলে `/model` কমান্ডটি যোগ করে একই intent কল করা যায় (সহজ কাজ) |

**বিদ্যমান slash কমান্ডের প্রকৃত তালিকা** (`app/src/panels/prompt/slash.ts:35-92`, যাচাই করা):
`/compact`, `/init`, `/review`, `/remember <fact>`, `/memory`, `/clear`, `/help`,
এবং `/<project cmd>` (`.sdc/commands/<name>.md` বা `.claude/commands`, `$ARGUMENTS` প্রতিস্থাপিত)।
অচেনা `/…` টেক্সট **যেমন আছে তেমনই engine-এ চলে যায়** — অর্থাৎ `/research` যোগ না করলেও
একটি প্রজেক্ট-কমান্ড ফাইল দিয়েই শুরু করা সম্ভব।

**নিয়মগুলো:**
- *"কমান্ড ছাড়া কোনো ইন্টারনেট অনুরোধ যাবে না"* → ভিত্তি আছে: `policy.privacy_local()`
  ইতিমধ্যে `web_search`/`web_fetch`/`browser`(পাবলিক URL) ব্লক করে। **কিন্তু** এটি আজ
  একটি স্থায়ী প্রজেক্ট-নীতি (`.sdc/policy.toml`-এর `privacy = "local-only"`),
  টার্ন-ভিত্তিক সুইচ নয় — তাই লাগবে একটি **রানটাইম ওভাররাইড**, যা শুধু
  `/research` চলার সময় খুলবে এবং শেষ হলে আবার বন্ধ হবে (বিস্তারিত ১.১০-এ)।
- *"রিসার্চ শুরুর আগে কোন মডেল ও কোন সার্চ সার্ভিস দেখাবে"* → ❌ নেই, নতুন UI কার্ড লাগবে।
- *"API মডেল বাছা থাকলে সম্ভাব্য খরচ আগে জানাবে"* → ⚠️ **অর্ধেক তৈরি**:
  price table, budget, `cost.estimate` মেথড ও `estimateCost` UI intent সবই আছে
  (`trust/cost.rs`, ১.৯), **কিন্তু intent-টি কোথাও কল হয় না** — পাইপ আছে, সংযোগ নেই।
  তাই কাজটি ছোট: `/research` শুরুর আগে সেই বিদ্যমান কলটি চালিয়ে কার্ড দেখানো।

### ৩.৩ `/research` মডিউলের ৪টি অংশ

| অংশ | SDC-তে আছে? | বিবরণ |
|---|---|---|
| **সার্চ সার্ভিস** | ⚠️ আছে কিন্তু pluggable নয় | DuckDuckGo HTML হার্ডকোডেড (`agent/web.rs:123`)। trait/enum দিয়ে বদলানোর ব্যবস্থা করতে হবে |
| **পেজ ফেচার** | ⚠️ আংশিক | `web_fetch` + `PAGE_CAP`/`BYTES_CAP` + SSRF গার্ড আছে; **main-content extraction নেই** (শুধু regex tag-strip)। JS সাইটের জন্য `browser` টুল আছে কিন্তু `web_fetch`-এর fallback হিসেবে যুক্ত নয় |
| **রিসার্চ লুপ** | ❌ নেই | তবে কাঁচামাল সব আছে: agent loop (`for step in 1..=max_steps`), `task` sub-agent, `update_plan`, `background.rs` |
| **সোর্স দেখানো** | ❌ নেই | কোনো structured citation/source type নেই; আজ সোর্স শুধু টেক্সটের ভিতরে URL হিসেবে থাকে |

**রিসার্চ লুপের ৬ ধাপ — কী লাগবে:**
1. *প্রশ্ন ভেঙে ৩–৫টি কোয়েরি* → সাধারণ একটি LLM কল (কোনো নতুন অবকাঠামো লাগে না)
2. *প্রতিটি কোয়েরিতে সার্চ* → `web_search` আছে; শুধু pluggable provider লাগবে
3. *পেজ নামিয়ে মূল লেখা বের করা* → `web_fetch` আছে; **readability-style extraction যোগ করতে হবে**
4. *প্রতিটি পেজ আলাদাভাবে সারাংশ* → 🔴 **এখানেই `num_ctx` সমস্যা**; `task` sub-agent ব্যবহার
   করলে প্রতিটি সারাংশ আলাদা context-এ চলবে (ঠিক যেটা আপনি চান)
5. *সব সারাংশ মিলিয়ে চূড়ান্ত বিশ্লেষণ* → একটি LLM কল; এখানেই **ঐচ্ছিক API মডেল সুইচ** লাগবে
6. *প্রতিটি দাবির পাশে সোর্স লিংক ও তারিখ* → ❌ নতুন structured output + UI

**থামার শর্ত:** ⚠️ `DEFAULT_STEPS: usize = usize::MAX` (`agent/mod.rs:55`) — অর্থাৎ
**ডিফল্টে কোনো স্টেপ-সীমা নেই!** কোডের নিজের যুক্তি (হুবহু উদ্ধৃত, `agent/mod.rs:50-54`):
> *"With older tool output folded as a turn grows (`keep_small`), a long turn does not outgrow the
> model's window, so **a count of steps protects nothing a person wants**. What stops a turn that
> goes wrong is still there: **Stop, the cost governor (a budget set in Settings) and the runaway
> detector** (the same call again and again). A caller that wants a bound — `sdcd run --max-steps`
> — still passes `maxSteps`."*

অর্থাৎ বিদ্যমান সুরক্ষা তিনটি: Stop বাটন, cost budget, runaway detector।
**কিন্তু লোকাল মডেল ফ্রি → cost budget সবসময় $0 → এই সুরক্ষাটি কাজ করবে না।**
তাই রিসার্চ লুপে আলাদা সীমা বাধ্যতামূলক (সর্বোচ্চ সার্চ, সর্বোচ্চ পেজ, সর্বোচ্চ সময়)।
ভালো খবর: `maxSteps` SDCP/CLI প্যারামিটার হিসেবে **আগে থেকেই পাঠানো যায়**
(`sdcd run --max-steps`, এবং `methods.rs`-এ `.unwrap_or(crate::agent::DEFAULT_STEPS)`) —
অর্থাৎ সীমা বসানোর plumbing তৈরি, শুধু `/research` থেকে মান পাঠাতে হবে।
`keep_small` (`agent/mod.rs`) ও `fold_old_results` (`agent/dialect.rs`) —
পুরনো টুল আউটপুট ভাঁজ করার ব্যবস্থাও আছে, যা ছোট উইন্ডোতে সাহায্য করবে।

**ব্যর্থতার আচরণ:** ✅ নীতিগতভাবে আছে — SDC-র সব টুল "not found"-এ স্পষ্ট বার্তা দেয়,
অনুমান করে না। রিসার্চে একই নিয়ম প্রয়োগ করতে হবে।

### ৩.৪ মডেল সুইচার ও সেটিংস

| প্ল্যানের দাবি | অবস্থা |
|---|---|
| ডিফল্ট লোকাল, চূড়ান্ত বিশ্লেষণে ঐচ্ছিক API | ❌ নেই — আজ **একটি সেশনে একটি মডেল**। per-stage মডেল override নেই |
| লোকাল সার্ভারের ঠিকানা ও মডেলের নাম | ⚠️ ঠিকানা হার্ডকোডেড `127.0.0.1:11434`; `custom` provider দিয়ে অন্য base URL সম্ভব |
| সার্চ সার্ভিস বেছে নেওয়া + API কী | ❌ নেই |
| API মডেলের কী (ফাঁকা = শুধু লোকাল) | ✅ **আছে** — `sdc.provider.<id>` key store, ফাঁকা থাকলে ইঞ্জিন স্পষ্ট বার্তা দেয় |
| রিসার্চের সীমা (সর্বোচ্চ সার্চ/পেজ) | ❌ নেই |

---

## ৪. কী কী আপগ্রেড করতে হবে — অগ্রাধিকার অনুযায়ী

### 🔴 P0 — না করলে কিছুই কাজ করবে না

**১. `num_ctx` সাপোর্ট (সবচেয়ে গুরুত্বপূর্ণ, সবচেয়ে ছোট পরিবর্তন)**
- **কেন:** বিস্তারিত ১.৫-এ — SDC ধরে নেয় ollama উইন্ডো 32,000, Ollama দেয় 4,096।
  পুরো `fit()`/`digest()`/`keep_small` যুক্তি ভুল সংখ্যার উপর চলছে।
- **কোথায়:** হুক পয়েন্ট আগে থেকেই তৈরি — `agent/mod.rs:709`-এ
  `let window = crate::context::window_tokens(if backend == Backend::Ollama { "ollama" } else { … })`
  ইতিমধ্যে গণনা হয়। সেই `window` মানটিকে রিকোয়েস্ট বডিতে বসাতে হবে
  (`agent/dialect.rs`-এর `fn body`, Ollama শাখায়)।
- **কী:** লোকাল ব্যাকএন্ডে রিকোয়েস্ট বডিতে যোগ হবে
  `"options": { "num_ctx": <window> }` (Ollama native `/api/chat`) বা
  OpenAI-compatible `/v1` পথে সমতুল্য (Ollama-র `/v1` `options`-ও নেয়;
  বিকল্প `OLLAMA_CONTEXT_LENGTH` env — কিন্তু সেটি ব্যবহারকারীর মেশিন সেটিং, তাই কোডে
  per-request দেওয়াই নির্ভরযোগ্য)।
- **মান:** catalogue-এর `ctx` থেকে, কিন্তু **একটি সিলিং দিয়ে** (যেমন 16,384) — না হলে
  256K context VRAM শেষ করে দেবে। একই সাথে `context.rs`-এর `ollama → 32_000` ডিফল্টটি
  catalogue-এর সাথে সামঞ্জস্যপূর্ণ রাখতে হবে।
- **যাচাই:** নামানোর পর `ollama ps`-এ CONTEXT কলাম দেখা।

**২. `qwen3.5:9b` catalogue-এ যোগ**
- কোথায়: `SDC/sdc/protocol/models.json` → `providers[ollama].models[]`
  (বর্তমানে সেখানে আছে `llama3.2:3b` ctx 128000, `deepseek-coder:6.7b` ctx 16000,
  `qwen2.5-coder:7b` ctx 32768)
- প্রস্তাবিত এন্ট্রি (উদাহরণ):
  ```json
  { "id": "qwen3.5:9b", "name": "Qwen3.5 9B", "tier": "balanced",
    "ctx": 16000, "cost": "free" }
  ```
  (`ctx` এখানে **SDC-র কার্যকর বাজেট**, মডেলের সর্বোচ্চ (256K) নয় — `window_tokens`
  catalogue-এর `ctx`-কে অগ্রাধিকার দেয়, তাই এটিই `num_ctx` ও `history_budget` দুটোকেই
  সঠিক সীমা দেবে।)
- এটি শুধু ডাটা বদল; `protocol/check.mjs` চালাতে হবে (schema-first প্রজেক্ট)।
- একই সাথে `app/src/panels/prompt/ModelDropdown.tsx` ইত্যাদি স্বয়ংক্রিয়ভাবে নতুন
  মডেলটি দেখাবে, কারণ তালিকা catalogue থেকে আসে (`refreshCatalog`, `groupCatalog`,
  `useModelStore`)।
- ⚠️ **একটি nuance:** ModelDropdown-এর নিজের মন্তব্য (v4 decision 6 অনুযায়ী):
  *"a provider that is **not connected** no longer has a group. Its models could not run,
  so offering them was the menu lying."* → Ollama ডেমন বন্ধ থাকলে `qwen3.5:9b`
  মেনুতে **দেখাই যাবে না**, বদলে `2 providers not connected · Manage in Provider Hub`
  লেখা দেখাবে। এটি ভালো আচরণ, কিন্তু ব্যবহারকারীকে Local ট্যাবের doctor পর্যন্ত নিয়ে
  যাওয়ার পথ স্পষ্ট রাখতে হবে।

**৩. `/research` কমান্ড + ড্রাইভার**
- UI: `app/src/panels/prompt/slash.ts`-এ এন্ট্রি; `strings.ts`-এ লেবেল/বর্ণনা।
- ডেমন: একটি নতুন মডিউল (যেমন `sdcd/src/agent/research.rs`) যা উপরের ৬ ধাপ চালায়।
  বিকল্প: `.sdc/commands/research.md` প্রজেক্ট-কমান্ড হিসেবে শুরু করা (দ্রুততম পথ,
  কোড বদল ছাড়াই পরীক্ষা করা যাবে) — পরে ডেমন-সাইড মডিউলে উন্নীত করা।

### 🟡 P1 — ভালো ফলাফলের জন্য দরকার

**৪. Pluggable search provider**
- একটি `trait SearchProvider { fn search(&self, q, n) -> Vec<Hit> }` +
  implementations: `duckduckgo` (ডিফল্ট), `searxng`, `tavily`, `brave`, `serper`।
- সেটিংস key: `sdc.research.search.provider`, `sdc.research.search.key`,
  `sdc.research.searxng.url`।
- `Hit`-এ **তারিখ** রাখতে হবে (প্ল্যানের ধাপ ৬-এর জন্য) — DuckDuckGo HTML-এ তারিখ নেই,
  তাই API provider-গুলো এখানে এগিয়ে থাকবে।

**৫. Main-content extraction**
- `agent/web.rs`-এ tag-strip-এর বদলে readability-style স্কোরিং
  (`<article>`, `<main>`, লিংক-ডেনসিটি, টেক্সট-ডেনসিটি) — Rust crate
  `readability` ব্যবহারযোগ্য; Python `trafilatura` এড়িয়ে চলাই ভালো (নতুন রানটাইম)।
- JS-নির্ভর সাইটে fallback: বিদ্যমান `browser` টুল।

**৬. Structured sources + UI**
- protocol-এ একটি `sources: [{ title, url, date }]` ফিল্ড (types.ts regenerated)।
- UI-তে ফলাফলের নিচে ক্লিকযোগ্য সোর্স তালিকা; প্রতিটি দাবির পাশে `[1]`-স্টাইল রেফারেন্স।

**৭. রিসার্চ-নির্দিষ্ট থামার শর্ত**
- `DEFAULT_STEPS = usize::MAX` রিসার্চে চলবে না। আলাদা:
  max_searches (ডিফল্ট ৫), max_pages (ডিফল্ট ১০), max_minutes, এবং
  প্রতি-ধাপে প্রগ্রেস ইভেন্ট।
- সেটিংস key: `sdc.research.max_searches`, `sdc.research.max_pages`।

**৮. নেটওয়ার্ক গেট — "শুধু কমান্ডে"**
- বিদ্যমান `policy.privacy_local()`-কে একটি **অস্থায়ী ওভাররাইড** দিন:
  `/research` শুরু হলে `network_allowed = true`, শেষে আবার `false`।
- এতে বাকি সব ফ্লো (কোডিং, চ্যাট) অপরিবর্তিত থাকবে — **আপনার মূল শর্ত পূরণ হয়**।

**৯. Pre-flight কার্ড**
- রিসার্চ শুরুর আগে একটি ছোট কার্ড: মডেল, ব্যাকএন্ড, সার্চ প্রোভাইডার, সীমা,
  এবং API মডেল বাছা থাকলে আনুমানিক খরচ (`trust/cost.rs`-এর price table থেকে)।

### 🟢 P2 — পালিশ

**১০. VRAM/OOM বার্তা** — Ollama-র এরর টেক্সটে `out of memory` / `CUDA` / `VRAM` /
`resource exhausted` খুঁজে মানুষের ভাষায় বার্তা + পরামর্শ (ছোট quant, `num_ctx` কমানো)।
**১১. "মডেল নামানো নেই" শনাক্ত করা** — `/api/tags`-এ না পেলে `ollama pull qwen3.5:9b`
কমান্ডসহ নির্দেশনা (Local ট্যাবে "Install" বাটন)।
**১২. Per-stage মডেল** — কোয়েরি+সারাংশ = লোকাল, চূড়ান্ত বিশ্লেষণ = ঐচ্ছিক API।
এটি **সবচেয়ে বড় আর্কিটেকচারাল বদল** (এক সেশনে এক মডেল → ধাপভিত্তিক), তাই সবার শেষে।
**১৩. বাংলা/Banglish যাচাই** — টেস্ট কেস হিসেবে রাখুন; কোড বদল লাগবে না,
তবে system prompt-এ ভাষা-নির্দেশনা যোগ করতে হতে পারে।

---

## ৫. আপনার মূল শর্ত: "বর্তমান flow অটুট থাকবে, শুধু provider-এর মতো লোকাল মডেল যোগ হবে"

**সুসংবাদ: এটি সম্ভব, এবং প্রায় P0 কাজগুলো এই শর্তই মেনে চলে।**

কারণ SDC-র provider সিস্টেম ডাটা-ড্রিভেন (`models.json`) এবং Ollama ইতিমধ্যে একটি
নিবন্ধিত provider। অর্থাৎ:

- `qwen3.5:9b` যোগ করা = **একটি JSON এন্ট্রি**, কোনো flow পরিবর্তন নয়।
- `num_ctx` যোগ করা = **শুধু লোকাল ব্যাকএন্ডের রিকোয়েস্ট বডিতে** একটি ফিল্ড;
  API provider-দের বডি অছোঁয়া থাকবে।
- নেটওয়ার্ক গেট = বিদ্যমান `privacy_local()` পলিসির উপর একটি ওভাররাইড;
  ডিফল্ট আচরণ একই থাকবে।
- `/research` = একটি নতুন কমান্ড; সাধারণ চ্যাট/কোডিং পাথে কিছুই বদলাবে না।

⚠️ **একমাত্র সতর্কতা:** P2-এর **per-stage মডেল সুইচার** (ধাপ ১২) এই শর্ত ভাঙতে পারে,
কারণ এটি সেশন-মডেলের মূল অনুমান বদলে দেয়। সুপারিশ: প্রথমে **`/model` দিয়ে হাতে বদল**
(ইতিমধ্যে কাজ করে), এবং চূড়ান্ত-বিশ্লেষণ ধাপে API মডেল যুক্ত করা একটি
**স্পষ্ট opt-in সেটিংস** রাখা — সেশনের মডেল না বদলে শুধু সেই একটি কলে আলাদা target ব্যবহার।

---

## ৬. বাস্তবায়নের প্রস্তাবিত ক্রম

**ধাপ ১ (যাচাই, কোনো কোড নয়):** Ollama চালু করে `ollama pull qwen3.5:9b`,
তারপর সরাসরি কল করে দেখুন tool-calling কাজ করে কিনা ও context কত ধরে:
```
curl http://127.0.0.1:11434/api/tags
curl http://127.0.0.1:11434/v1/chat/completions -d "{\"model\":\"qwen3.5:9b\",\"messages\":[…],\"tools\":[…],\"stream\":true}"
```
`ollama ps` দিয়ে CONTEXT কলাম মিলিয়ে নিন (ডিফল্ট 4096 দেখাবে)।

**ধাপ ২:** `models.json`-এ এন্ট্রি + `num_ctx` সাপোর্ট → SDC থেকেই সাধারণ চ্যাট ও
বিদ্যমান `web_search` দিয়ে ম্যানুয়াল রিসার্চ পরীক্ষা।

**ধাপ ৩:** `.sdc/commands/research.md` দিয়ে দ্রুত একটি প্রম্পট-ভিত্তিক রিসার্চ ফ্লো
(কোড বদল ছাড়া) — এতে Banglish/বাংলা সাড়া ও সোর্স-মান সম্পর্কে ধারণা মিলবে।

**ধাপ ৪:** P1 — pluggable search, extraction, structured sources, থামার শর্ত, নেটওয়ার্ক গেট।

**ধাপ ৫:** P2 — ত্রুটি-পালিশ, pre-flight খরচ কার্ড, per-stage API মডেল (opt-in)।

---

## ৭. আপনার পরীক্ষার চেকলিস্ট — SDC-র প্রেক্ষিতে কীভাবে মাপবেন

| চেক | কীভাবে |
|---|---|
| Ollama চালু + `qwen3.5:9b` নামানো, SDC দেখাতে পারে | Provider Hub → Local ট্যাব (doctor) + `models.list { refresh:true }` |
| মডেলের টুল-কলিং কাজ করে | SDC-তে Ollama মডেল বেছে একটি `read_file`/`grep` চাহিদা দিন; টুল কল দেখা গেলে পাস। ⚠️ কোড-পথ আছে কিন্তু এন্ড-টু-এন্ড টেস্ট নেই (১.২) — তাই এটি হাতে করে দেখতেই হবে |
| কমান্ড ছাড়া ইন্টারনেট যায় না | `privacy_local` চালু রেখে সাধারণ টার্ন চালান; `web_search` কল গেট হচ্ছে কিনা দেখুন |
| `/research` সহজ প্রশ্নে সোর্সসহ উত্তর দেয় | ধাপ ৪-এর structured sources UI সহ |
| সোর্স না পেলে অনুমান না করে জানায় | একটি অসম্ভব প্রশ্ন দিয়ে পরীক্ষা |
| থামার সীমা ছুঁলে লুপ বন্ধ হয় | `max_searches=1` সেট করে পরীক্ষা (⚠️ আজ `DEFAULT_STEPS = usize::MAX` — এটা ছাড়া পাস করা অসম্ভব) |
| ৩–৪টি Banglish প্রশ্নে সাড়া | ধাপ ৩-এর `.sdc/commands` ফ্লো দিয়ে আগেই দেখা শুরু করা যাবে |

---

## ৮. ঝুঁকি ও সীমাবদ্ধতা (সৎ মূল্যায়ন)

1. **Context mismatch** — `num_ctx` না দিলে পুরো রিসার্চ নকশা ভেঙে পড়বে। এটাই প্রথম কাজ।

   **সংখ্যা দিয়ে প্রমাণ** (SDC-র নিজের ধ্রুবক + Ollama doc, সব যাচাই করা):
   - Ollama ডিফল্ট context = **VRAM-নির্ভর**: < 24 GiB → **4k**, 24–48 GiB → 32k,
     ≥ 48 GiB → 256k। Ollama doc-ই বলে: *"web search, agents … should be set to
     **at least 64000 tokens**"* — অর্থাৎ আপনার প্ল্যানের 8k–16k অনুমানটি
     Ollama নিজেই অপর্যাপ্ত বলে (২.১-এ সমাধানসহ আলোচনা)।
   - SDC-র ধরে নেওয়া ollama উইন্ডো = **32,000** (`context.rs`) → history budget 14,400।
     4k-তে Ollama এটি নীরবে কেটে দেয় → **৮ গুণ ফারাক**।
   - একটি টুল ফলাফলের সীমা `RESULT_CAP` = **24,000 অক্ষর** (`agent/tools.rs:38`)
   - একটি ফেচ করা পেজের সীমা `PAGE_CAP` = **60,000 অক্ষর** (`agent/web.rs:13`)
   - একটি `web_search` **৮টি** রেজাল্ট ফেরত দেয় — hard-coded (`agent/tools.rs:1184`:
     `match super::web::search(query, 8)`)

   → অর্থাৎ **একটি মাত্র `web_fetch`-এর ফলাফলই (২৪,০০০ অক্ষর ≈ 6,000 টোকেন)
   4K context-এর চেয়ে বড়।** system prompt ও ~22 টুলের schema তো বাদ-ই।
   বাংলা পেজ হলে আরও খারাপ, কারণ `context::tokens_of` বাংলাকে ≈১ টোকেন/অক্ষর ধরে।
   ফলে প্রথম রিসার্চ-ধাপেই Ollama ইতিহাস নীরবে কেটে দেবে → মডেল প্রশ্ন ও পূর্বের ফলাফল
   "ভুলে যাবে" → hallucination। **`num_ctx` ছাড়া `/research` কাগজে-কলমেই অসম্ভব।**

   বাস্তব ন্যূনতম বাজেট (সব টুল চালু রেখে): system + ~22 টুলের schema ≈ 3–5K,
   একটি `web_fetch` ফলাফল 6K, ৮টি সার্চ রেজাল্ট 1–2K, কথোপকথন → **≥16K লাগবেই**,
   4K-তে কিছুই হবে না। তাই দুই কাজ একসাথে: `num_ctx` বাড়ানো **এবং**
   পাঠানো জিনিস কমানো (টুল ছাঁটাই, sub-agent, cap কমানো)।
2. **VRAM** — 9B মডেল + বড় context-এ যথেষ্ট VRAM লাগে (quant-ভেদে ~7–10 GB+;
   context বাড়ালে আরও)। Ollama doc: *"avoid offloading the model to CPU"* —
   `ollama ps`-এর `PROCESSOR` কলামে CPU অংশ দেখা মানে গতি নাটকীয়ভাবে কমবে।
   রিসার্চ লুপে এটি ৫–১০ গুণ ধীর হতে পারে; থামার শর্তে **সময়-সীমাও** রাখা জরুরি।
   ⚠️ এখানেই দ্বন্দ্ব: Ollama বলে ≥64k context, VRAM বলে যত কম তত ভালো —
   তাই `task` sub-agent দিয়ে প্রতি-ধাপ ছোট রাখাই একমাত্র টেকসই সমাধান।
3. **DuckDuckGo HTML স্ক্র্যাপিং fragile** — বট-ব্লক হলে সার্চ চুপচাপ ফাঁকা ফেরাবে
   (অফিসিয়াল API নয়, কোনো চুক্তি নেই)। তাই pluggable provider শুধু সুবিধা নয়,
   **নির্ভরযোগ্যতার প্রয়োজন**।
4. **ছোট মডেলের সারাংশে ভুল** — আপনার প্ল্যানেই আছে, সঠিক। প্রতিটি দাবির পাশে
   সোর্স + তারিখ বাধ্যতামূলক করা হলে ব্যবহারকারী নিজে মিলিয়ে নিতে পারবেন।
5. **পেজের তারিখ** — DuckDuckGo HTML রেজাল্টে নেই; API provider বা পেজের meta থেকে তুলতে হবে।
6. **`DEFAULT_STEPS = usize::MAX`** — ফ্রি লোকাল মডেলে cost budget থামাতে পারবে না
   (খরচ সবসময় $0), তাই রিসার্চের নিজস্ব সীমা ছাড়া অনন্ত লুপের ঝুঁকি বাস্তব।
   বাকি দুই সুরক্ষা (Stop বাটন, runaway detector) আছে, কিন্তু সেগুলো মানুষের হস্তক্ষেপ
   বা একই কল বারবার হওয়ার উপর নির্ভরশীল।
7. **লোকাল মডেলে টুল-কলিংয়ের এন্ড-টু-এন্ড টেস্ট নেই** — কোড-পথ আছে (১.২),
   কিন্তু বিদ্যমান `Backend::Ollama` টেস্টটি টুল-কলিং যাচাই করে না।
   তাই ধাপ ১-এর ম্যানুয়াল যাচাই এড়িয়ে যাওয়া যাবে না।

---

## ৯. সূত্র

- SDC কোডবেস: `sdcd/src/{agent,engines,providers,trust,context.rs,sdcp}`,
  `app/src/{panels/prompt,modals,kernel,store}`, `protocol/models.json`
- Ollama ডকুমেন্টেশন: `qwen3.5` library পেজ ও tags, `/api/chat`, `/v1` OpenAI compatibility,
  context-length গাইড, web-search capability
- সার্চ প্রোভাইডার: tavily.com/pricing, brave.com/search/api, serper.dev,
  docs.searxng.org/dev/search_api.html
