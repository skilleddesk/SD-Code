# SDC v4 দিকনির্দেশ — "prompt দিলেই project শেষ"

তারিখ: ২৫ সেপ্টেম্বর ২০২৬ · ভিত্তি: `MASTER_SPEC.md` v3.0 (0.7.13 পর্যন্ত বাস্তবায়িত)
অবস্থা: **প্রস্তাব — approval-এর অপেক্ষায়।** Approve হলে প্রতিটা Phase আলাদা release হিসেবে নামবে,
প্রতিটার শেষে test সবুজ + CHANGELOG entry — এই repo-র এখনকার নিয়মেই।

UI প্রস্তাবের ছবি: [`design/ui-proposal-v4.html`](../design/ui-proposal-v4.html) — এই ডকুমেন্টের §৩-এর
৭টা পরিবর্তন ওখানে আঁকা আছে, প্রতিটা নম্বর-করা।

---

## ০. চাওয়াগুলো, এক নজরে — কী আছে, কী বাকি

| # | চাওয়া | এখন কী আছে | কী বাকি | কোন Phase |
|---|---|---|---|---|
| ১ | Prompt দিলেই AI পুরো project বানাবে, terminal command-ও নিজে চালাবে | **আংশিক।** CLI engine (`claude`/`codex`/`gemini`) দিয়ে হয় — ওরা নিজেরাই agent। API/Ollama engine-এ হয় না: এক প্রশ্ন → এক উত্তর, tool চালায় না | daemon-এর নিজের **agent loop** (SDC Agent) — API আর Ollama-র জন্য | **Phase 2** |
| ২ | Manual + prompt — দুই পথই | আছে: Files tree → Preview-তে edit + Save (checkpoint-সহ), পাশে chat | Editor গভীর করা: syntax highlight, multi-file tab, project search | Phase 4 |
| ৩ | Local + VPS এক UI-তে, folder/file পুরো access | আছে (0.7.13): `fs.*`, `git.*`, `shell.run`, engine, checkpoint, rewind — সব host-এও একই shape-এ | আপনার আসল VPS-এ **end-to-end প্রমাণ** (আজ port 8443 reachable ✓; key install-এ password একবার লাগবে) | **Phase 0** |
| ৪ | VPS connect SSH দিয়ে, test করা | আছে: host key pin, fingerprint dialog, Install key, doctor | ঐ একই end-to-end রান | Phase 0 |
| ৫ | Chat-এ AI model বদলের option | আছে: Tier/Engine/Model dropdown | — (gating নিচের ৮ নম্বরে) | — |
| ৬ | এক AI লেখে, আরেক AI verify করে | **Surface আছে, pipeline নেই।** Verify tab আজ static rows + toast (`VerifyTab.tsx` নিজেই বলে "the rows are the daemon's to fill in") | `verify.run`: mechanical checks + **অন্য engine দিয়ে diff review** | **Phase 3** |
| ৭ | CLI-supporting AI → CLI + subscription; API-only → API | আছে: ৫টা adapter, `cli_login::RECIPES`, keychain, `provider.test` (verified:true লাইভ প্রমাণিত) | আসল CLI-র বিরুদ্ধে sign-in exercise; provider OAuth exchange | Phase 0, 5 |
| ৮ | Dropdown-এ **শুধু connected** model, আর প্রতি family-র **শেষ ২–৩ version** | **নেই।** `store/model.ts`-এর catalog static — connect থাকুক না থাকুক সব দেখায় | connected-গেট + version cap | **Phase 1** |
| ৯ | Live thinking — Claude Code-এর মতো বা তার চেয়ে ভালো | ভিত আছে: P3 structured stream, `ThinkingBlock.tsx`, `live.ts`, tool cards | Streaming polish + **checkpoint timeline rail** (Claude Code-এ যেটা নেই) | **Phase 1** |

সারমর্ম: আপনার ৯টা চাওয়ার ৫টার ভিত তৈরি, ২টা আংশিক, ২টা (agent loop, verify pipeline) নতুন বানাতে হবে।
নতুন দুটোই daemon-এর **বিদ্যমান** যন্ত্রাংশ দিয়ে বানানো যায় — নিচে প্রমাণ।

---

## ১. মূল স্থাপত্য সিদ্ধান্ত — এবং কেন এটাই best (প্রমাণসহ)

### সিদ্ধান্ত ১ — Agent loop টা daemon-এ (`sdcd`) বসবে, UI-তে নয়

Agent-এর চারটা verb — পড়া, লেখা, চালানো, দেখা — daemon-এ **আগে থেকেই আছে**:
`fs.read` / `fs.write` / `shell.run` / `git.status`+`git.diff`+`fs.search`। আর 0.7.13-এর পর এই
সবগুলোই local **এবং** SSH host-এ একই shape-এ উত্তর দেয় (`remote_for(envelope)` → `ssh::ops`)।

**প্রমাণ কেন best:** loop-টা daemon-এ বসালে VPS-এ agent চালানো **বিনামূল্যে** পাওয়া যায় — একই আটটা
tool, শুধু `hostId` আলাদা। Cline/Cursor/Copilot-এর tool গুলো local machine-এ বাঁধা; "আমার VPS-এর ভিতরে
agent, আমার laptop-এ UI" — এটা SDC-র নিজস্ব জায়গা। দ্বিতীয়ত: permission gate, deny list, file guard,
checkpoint-আগে-mutation — সব daemon-এ already আছে, তাই agent-ও আপনা থেকেই P4 (মিথ্যা নয়) আর
P5 (সব ফেরানো যায়) মেনে চলবে। UI-তে loop বানালে এই সব guard **দ্বিতীয়বার** বানাতে হত।

### সিদ্ধান্ত ২ — CLI-গুলোকে replace করা হবে না: dual path

- CLI installed + signed-in (subscription) → **CLI-ই engine** (আজ যেমন)। Claude Pro/Max-এর টাকা শুধু
  CLI দিয়েই কাজে লাগে; নিজের loop-এ API চালালে আলাদা bill।
- শুধু API আছে (Anthropic/OpenAI key, Ollama) → **SDC Agent** loop, একই tool, একই permission।

**প্রমাণ:** এটা নীতি P2 ("SDC AI বানায় না, চালায়")-র সরাসরি ফল, আর ব্যবহারকারীর টাকার হিসাবেও সস্তা পথ।
দুই পথের UI এক — composer-এর **Chat | Agent** switch (§৩.১), মানুষকে জানতে হয় না ভিতরে কোন যন্ত্র।

### সিদ্ধান্ত ৩ — Verify মানে **অন্য** engine, আর input হবে diff, পুরো repo নয়

`verify.run { chatId, reviewerEngine }`: turn-এর checkpoint→এখন `git.diff` + কাজের বিবরণ reviewer
engine-কে দেওয়া হবে; সে structured verdict দেবে (pass / issues[], প্রতিটায় file:line + বাক্য + suggested fix)।

**প্রমাণ:** (ক) same-model self-review দুর্বল — নিজের ভুল নিজের চোখে পড়ে না; ভিন্ন lab-এর model-এর
ভুলের ধরন ভিন্ন, তাই cross-model review বেশি ধরে। (খ) diff-based review সস্তা আর নির্ভুল — reviewer
শুধু যা বদলেছে তাই পড়ে; আর diff টা daemon-এ **ইতিমধ্যে** আছে (shadow git, `git.diff`), নতুন storage লাগে না।
(গ) mechanical check (typecheck/build/test/lint) আগে চলবে, AI review পরে — যেটা যন্ত্রে ধরা যায় তাতে
token খরচ নয়।

### সিদ্ধান্ত ৪ — Editor-এর জন্য CodeMirror 6, Monaco নয়

**প্রমাণ:** Monaco = VS Code-এর editor — ~৫ MB, আর আপনি স্পষ্ট বলেছেন "VS Code-এর মতো চাই না"।
CodeMirror 6 modular (~৩০০ KB ব্যবহৃত অংশে), tree-shakeable, touch-friendly; SDC-র দরকার
preview + edit + highlight — LSP/debugger নয় (§2.2: SDC IDE নয়)। CM6 ঠিক ততটুকুই।

### সিদ্ধান্ত ৫ — Terminal tab-এ xterm.js + `ssh -tt`

**প্রমাণ:** xterm.js-ই VS Code-এর নিজের terminal — জগতের সবচেয়ে বেশি পরীক্ষিত web terminal emulator।
daemon-এ `pty.write` **আছেই**; বাকি শুধু remote-এ `-tt` flag আর window-তে emulator। এতে `vim`/`htop`
পর্যন্ত চলবে — README-র Next step ৫-এর উত্তর।

### সিদ্ধান্ত ৬ — Dropdown-এ শুধু connected engine/model

**প্রমাণ:** নীতি P4 — যে model select করলে turn fail করবে, তাকে দেখানোই UI-র মিথ্যা। আর তিনটা সত্যের
উৎস daemon-এ আছেই: `cli.recipes` (installed?), sign-in অবস্থা, keychain + `provider.test`
(verified?)। নতুন কিছু আবিষ্কার নয় — তিনটা উত্তর এক জায়গায় জড়ো করা: `models.connected`।

### সিদ্ধান্ত ৭ — প্রতি family-র শেষ ২–৩ version, বাকিটা "Older…" -র পিছনে

`protocol/models.json`-এ দুটো নতুন field: `family` আর `current: true|false`। Dropdown দেখাবে প্রতি
family-র current গুলো (২টা, flagship family হলে ৩টা); বাকি সব এক লাইনের "Older versions…" disclosure-এ।

**প্রমাণ:** কম পছন্দ = কম ভুল; deprecated model-এ ভুল করে টাকা খরচ আটকায়; আর তালিকাটা ছোট বলে
এক নজরে পড়া যায়। "সব version" চাওয়া power user disclosure খুলে পাবে — কিছুই লুকানো নয় (P4)।

---

## ২. Phase পরিকল্পনা

প্রতিটা Phase একটা release; শুরু হয় আগেরটার test সবুজ থাকলে। ক্রমটার যুক্তি: **আগে প্রমাণ, তারপর ছোট
দৃশ্যমান জয়, তারপর বড় নির্মাণ** — যাতে বড় কাজটা (agent) প্রমাণিত ভিতের উপর হয়।

### Phase 0 — ভিত প্রমাণ (কোনো নতুন feature নয়)
1. 0.7.13-এর অসম্পূর্ণ কাজ commit (৪৩ file, ~৪১০০ লাইন এখন uncommitted — ঝুঁকি)।
2. আপনার VPS-এ end-to-end: `host.add` → fingerprint → `host.trust` → **Install key** (password একবার)
   → doctor সবুজ → remote folder-এ chat → `fs.list`/Preview → `shell.run` → Terminal → checkpoint → rewind।
3. একটা paid API key দিয়ে **streaming turn** (README-র Next step 2 — transport প্রমাণিত, turn নয়)।
4. পারলে: আসল `claude` CLI install করে sign-in recipe exercise।

**Acceptance:** উপরের প্রতিটা ধাপের ফল NOTES.md-তে STEP হিসেবে, ব্যর্থ হলে সেটাও (এ repo-র নিয়ম)।

### Phase 1 (0.8.0) — Connected-only dropdown + live thinking সম্পূর্ণ
- daemon: নতুন method `models.connected` → engine-প্রতি `{connected, how: cli|api|local, models[]}`;
  উৎস: recipes + keychain + provider.test cache। `protocol/models.json`-এ `family`/`current`।
- app: `store/model.ts` static catalog → daemon-driven; `ModelDropdown` নতুন নকশা (mockup #২);
  disconnected engine তালিকায় নেই — footer-এ "N providers not connected · Manage…"।
- Turn stream: thinking block live-stream polish (আসা মাত্র আঁকা, শেষে auto-collapse, সময়-গোনা),
  tool card-এ live status; **checkpoint timeline rail** (mockup #৪) — প্রতিটা mutation-এর পাশে dot,
  click → Time Machine সেই checkpoint-এ।

**Acceptance:** disconnected provider dropdown-এ অনুপস্থিত (test); প্রতি family ≤৩ current (test);
thinking র প্রথম অক্ষর আসা থেকে আঁকা শুরু (live.ts test)।

### Phase 2 (0.9.0) — SDC Agent: daemon-এর নিজের loop
`sdcd/src/agent/`:
- `registry.rs` — v1-এ **ঠিক ৮টা tool, বেশি নয়**: `read_file`, `write_file`, `list_dir`, `search`,
  `run_command`, `git_diff`, `ask_user`, `done`। প্রতিটা বিদ্যমান daemon path-এ বসে (guard/deny/checkpoint ফ্রি)।
- `planner.rs` — tool-use loop: native_api-র দুই dialect (Anthropic tools / OpenAI tools) + Ollama।
  প্রতিটা iteration: model → tool call → gate → execute → result ফেরত → আবার model, `done` পর্যন্ত।
- `gate.rs` — mutation-এর আগে বিদ্যমান Permission প্রবাহ; mode-অনুযায়ী নিয়ম:
  Simple = প্রতিটা লেখায় জিজ্ঞেস, Pro = folder-এর ভিতরে লেখা auto + `run_command` জিজ্ঞেস,
  Auto = deny-list ছাড়া সব auto। (Auto-তেও deny list আর file guard **কখনো** বন্ধ হয় না।)
- `budget.rs` — token/৳ hard cap + iteration cap (default ২৫); ছুঁলে থেমে জিজ্ঞেস — চুপচাপ চালিয়ে যাওয়া নয়।
- UI: composer-এ **Chat | Agent** switch (mockup #১); turn-এর মাথায় **plan card** (mockup #৫) —
  model-এর ঘোষিত ধাপ, live tick।

**Acceptance:** API engine-এ "একটা express server বানাও, test সহ, test চালিয়ে দেখাও" এক prompt-এ
শেষ হয় — প্রতিটা লেখা checkpoint-সহ, প্রতিটা command tool card-সহ, rewind কাজ করে; budget ছুঁলে থামে (test);
SSH host-এ একই prompt একই ফল দেয়।

### Phase 3 (0.9.x) — Verify pipeline
- daemon: `verify.run { chatId, reviewerEngine? }` — ধাপ ১ mechanical (typecheck/build/test/lint,
  project type চিনে), ধাপ ২ reviewer engine-কে diff + বিবরণ → structured verdict।
- UI: `VerifyTab` দুই স্তর (mockup #৭); turn footer-এ **Verify with…** (mockup #৬); verdict-এর প্রতিটা
  issue → click → Preview সেই file:line-এ + "Fix this" এক-click prompt (Error Translator-এর জ্ঞাতি)।

**Acceptance:** জানা-bug-ওয়ালা diff-এ ভিন্ন engine অন্তত সেই bug টা নাম ধরে বলে (fixture test);
mechanical fail হলে AI ধাপ চলে না (token বাঁচে)।

### Phase 4 (0.10.0) — Workbench গভীর
xterm.js Terminal (`-tt`) · CodeMirror 6 highlight + edit · multi-file tab (Preview-তে tab strip) ·
project-wide search (daemon-এর `fs.search` UI পায়) · tree-তে rename/delete (checkpoint-সহ)।

### Phase 5 — 1.0-এর পথ
Onboarding wizard (প্রথম চালু: doctor → provider connect → প্রথম chat) · provider OAuth exchange
(client registration — মানুষের কাজ) · palette/Permission/Time Machine-এ screen-reader sweep ·
installer signing (certificate লাগবে)।

---

## ৩. UI প্রস্তাব — ৭টা পরিবর্তন (mockup-এ নম্বর মিলিয়ে)

দেখুন [`design/ui-proposal-v4.html`](../design/ui-proposal-v4.html)। বিদ্যমান design token
(`design/tokens.json`) হুবহু ব্যবহৃত — রঙ, radius, Inter + JetBrains Mono। প্রতিটা নতুন জিনিসে
নম্বর-করা chip আছে, নিচে ব্যাখ্যা।

| # | পরিবর্তন | কোথায় | কেন |
|---|---|---|---|
| ১ | **Chat \| Agent** switch | Composer, বাঁয়ে | দুই পথ (কথা / কাজ) এক click-এ; engine অনুযায়ী ভিতরের যন্ত্র (CLI নিজে agent, নাকি SDC Agent) আপনিই ঠিক হয় |
| ২ | **Connected-only model dropdown** | Composer-এর model pill | শুধু connected engine, প্রতি family শেষ ২–৩ version, connection badge (CLI ✓ / API ✓), footer-এ "not connected · Manage…" |
| ৩ | **Live thinking** | Turn stream | অক্ষর-অক্ষর stream, চলাকালীন dim italic + timer, শেষে auto-collapse এক লাইনে — Claude Code-এর সমান |
| ৪ | **Checkpoint timeline rail** | Turn stream-এর বাঁ কিনারা | প্রতিটা mutation-এর dot; hover = কী বদলেছে, click = Time Machine — **Claude Code-এ এটা নেই**, এটাই "তার চেয়ে ভালো" অংশ |
| ৫ | **Agent plan card** | Agent turn-এর মাথায় | Model-এর ঘোষিত ধাপ live tick হয় — মানুষ জানে এখন কী হচ্ছে, কতটা বাকি |
| ৬ | **Verify chip + "Verify with…"** | Turn footer | এক click-এ অন্য engine দিয়ে review; ফল chip হয়ে turn-এই থাকে |
| ৭ | **Verify tab দুই স্তর** | Right panel | উপরে mechanical checks (আজকের rows, এবার সত্যি), নিচে AI reviewer-এর verdict — প্রতিটা issue click-able |

পুরনো যা **বদলায় না**: grid (46px / 1fr / 30px), topbar-এর ৮ বোতাম, host-grouped sidebar, right
panel-এর ৬ tab, status bar-এর ৭ segment, সব রঙ/টাইপ token। এই প্রস্তাব spec §7-এর **উপরে যোগ**, ভাঙা নয়।

---

## ৪. ঝুঁকি ও সীমা

| ঝুঁকি | জবাব |
|---|---|
| Agent scope creep | v1-এ tool ঠিক ৮টা, iteration cap ২৫, নতুন tool = নতুন release |
| API খরচ পালিয়ে যাওয়া | `budget.rs` hard cap, status bar-এ live ৳/token, ছুঁলে থামা |
| Model তালিকার curation বাসি হওয়া | `models.list`-এর live উৎস আগে থেকেই আছে; `family`/`current` bundled JSON-এ, live উত্তর এলে সেটাই জেতে |
| CLI recipe drift (আগের নোট বহাল) | UI raw output দেখায় — আগের সিদ্ধান্ত, বদলাচ্ছে না |
| Reviewer engine-ও ভুল বলতে পারে | Verdict-এ engine-এর নাম সবসময় লেখা থাকে; verdict মানে suggestion, merge-এর মালিক মানুষ (P4) |

## ৫. আপনার কাছ থেকে যা লাগবে

1. এই plan-এ **approve / বদলের কথা** — বিশেষ করে Phase-ক্রম আর ৮-tool সীমা।
2. Mockup-এর ৭টা নম্বর ধরে **UI approve** (`design/ui-proposal-v4.html`)।
3. Phase 0-র জন্য: VPS-এর password **একবার** (Install key ধাপ — আপনি নিজেও চালাতে পারেন, ধাপগুলো
   `docs/SSH-CONNECT.md`-তে আছে), আর একটা paid API key (streaming test)।
