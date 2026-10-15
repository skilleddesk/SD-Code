# SDC 2.0.1 (SDC-v3) — চূড়ান্ত রিপোর্ট

তারিখ: ২০২৬-১০-১৫ · base `sdc-baseline-v1` (v1.0.0, `274ec6a`) → `v2.0.1` (`558e010`). `v2.0.0` tag-এর release workflow ব্যর্থ হয়েছিল, তাই 2.0.1 প্রকাশিত হয়েছে (কারণ §৬)।

এই রিপোর্টে শুধু সেই সংখ্যাই আছে যা `bench/results/`-এ ফাইল হিসেবে আছে বা যা এই সেশনে চালিয়ে দেখা হয়েছে।
যেটা মাপা হয়নি সেটা "মাপা হয়নি" বলা হয়েছে।

## ১. সবচেয়ে গুরুত্বপূর্ণ কথা (সৎভাবে)

**মূল লক্ষ্য — "cloud bill ≥৬০% কম, একই বা ভালো সাফল্যে" — প্রমাণিত নয়।** কারণ দুটি:

1. টাকা লাগে এমন bench (প্রতিটি কৌশল × প্রতিটি model-এ Q3 নিয়মের তুলনা) আপনার নির্দেশ অনুযায়ী চালানো হয়নি;
   আপনি বলেছিলেন প্রতিটি API-তে শুধু একটি করে prompt দিয়ে test করতে।
2. চালু API-র মধ্যে শুধু **DeepSeek** কাজ করছে। Alibaba-র account-এ কোনো chat model কেনা নেই (`403 AccessDenied.Unpurchased`),
   OpenAI-তে credit নেই (`429`), OpenRouter-এর key-ই সঞ্চিত নেই। তাই পাঁচ model-এর তুলনা এখন সম্ভবও নয়।
   (বিস্তারিত: `api-live-test-bn.md`)

ফলে যে কৌশলগুলো **token বাঁচায় কিন্তু model-কে যা বলা হয় তা বদলায়** (W5 `stable_prefix`/`tool_pruning`, I1 `ledger_diff`,
I2 `skeleton`, I4/L2 `recipes`, L0 `memory`, L1 `lessons`, L3 `knowledge`, W4 `repomap`, I8 `interlingua`, L4 `bandit`)
**কোডে আছে, পরীক্ষিত, কিন্তু ডিফল্টে বন্ধ**। Q3 পাস না করা পর্যন্ত চালু করা ভুল হবে (নিয়ম ২)। `v3.maxQuality` দিলে এগুলো সব একসাথে বন্ধ থাকে।

যা **চালু** আছে, তা model-কে কম বা বেশি বলে না; নির্ভরযোগ্যতা, গতি, নিরাপত্তা বাড়ায়: journal ও resume, offline অপেক্ষা,
secret redaction, untrusted-data চিহ্ন, rate limit, নতুন search, token receipt, budget, failover, privacy, router/cascade (নাম দিলে), `/plan`, CAS undo, robots.txt।

## ২. যা মাপা হয়েছে (ফাইলসহ)

| বিষয় | ফল | ফাইল |
|---|---|---|
| Search, ৫২,৫০১ ফাইল, warm cache, macOS arm64 | v1: ফাইলনাম p50 229–234 ms, লেখা p50 227–1204 ms → v3: ফাইলনাম 2.5–3.2 ms, লেখা 2.0–6.6 ms; প্রথম call 150 ms; peak RSS 58 MB | `results/2026-10-14-search-v1`, `-v3` |
| হঠাৎ kill (`kill -9`) থেকে resume, scripted provider | ১০০ kill → ১০০/১০০ সঠিক, ফাইল নষ্ট ০, command দুবার চলা ০ | `results/2026-10-14-resume` |
| Fleet clustering, synthetic | ৫ server −44.6%, ১০ −68.3%, ২০ −80.4%, লুকানো diff ০ | `results/2026-10-14-fleet-synthetic` |
| ৪২টি bench task | প্রতিটি অপরিবর্তিত fixture-এ fail, reference সমাধানে pass | `sdcd bench validate` |
| আসল model-এ একটি agent turn (installed 2.0.0, DeepSeek) | সঠিক উত্তর, ফাইল অপরিবর্তিত, journal পূর্ণ (start→model→intent→result→end), ৮,৮৪৬ in / ১৪২ out | এই সেশনের লগ |
| একই ৩টি task, v1 binary বনাম 2.0, দুদিকেই cache গোনা (paid, DeepSeek `deepseek-chat`) | খরচ $0.0075 → $0.0054 (**≈ −২৮%**), input −৩৬%, pass ৩/৩ → ৩/৩। **৯৫% CI [−২৩.৬%, +৪০.১%] শূন্য ছুঁয়েছে** — ৩ task × ১ run, তাই "বাড়েনি, সম্ভবত কমেছে", প্রমাণিত সাশ্রয় নয়। (প্রথম পরীক্ষার −৭১.৮% বাড়ানো ছিল: v1-এর cache গোনা হয়নি; সংশোধন `cost-test-bn.md`-এর শুরুতে) | `results/cost-v1-footer`, `results/cost-v2-footer`; পুরোনো `cost-v1-keychain`, `cost-smoke-v2` |

**সীমা:** Search-এর সংখ্যা শুধু macOS-এ; Windows/Linux CI থেকে আসবে। Fleet সংখ্যা synthetic (আসল Docker fleet নয়) এবং SDC-র নিজের token গণনায়।
Resume-এর পরীক্ষা scripted provider-এ — আসল model-এ নয়।

## ৩. Assumption A-01…A-22 — শেষ অবস্থা

| ID | অবস্থা | প্রমাণ / কেন |
|---|---|---|
| A-01 | আংশিক | ৯টির মধ্যে ৪ সত্য, ৩ আংশিক, ২ মিথ্যা (`ASSUMPTIONS.md`) |
| A-02 | **মিথ্যা (মাপা)** | provider-এর হিসাবে input-এর ২১.০% system, ৪১.২% tools, ২২.৮% tool result — ধারণা ছিল "~২৫% system+tools, ~৫০% tool result" (`results/cost-smoke-v2`) |
| A-03 | মাপা হয়নি | প্রতি task-এ ১ রান, variance-এর জন্য ≥২ রান লাগে (টাকা লাগে) |
| A-04 | **প্রমাণিত** | পুরো folder হাঁটাই ধীরতার কারণ; index দিয়ে 2–4 ms |
| A-05 | **প্রমাণিত (macOS)** | filename p50 ≤3.2 ms, text ≤6.6 ms; Windows/Linux CI-তে |
| A-06…A-07 | মাপা হয়নি | repo map/tool pruning ডিফল্টে বন্ধ (Q3-এর অপেক্ষায়); দুটো আলাদা arm চালালে মাপা যাবে |
| A-08 | **প্রমাণিত (DeepSeek, এক নমুনায়)** | DeepSeek-এর নিজের হিসাবে cached ৮০.৮%; step 1-এ ০, step 4-এ ৭,৫৫২/৭,৭১৮ — `results/cost-smoke-v2/ledger.jsonl` |
| A-09 | **প্রমাণিত (Claude Code)** | input-এর ৮৭% (DeepSeek-এ), ৮৬% (Haiku-তে) cache থেকে; Codex মাপা হয়নি (login নেই) — `vs-claude-code-bn.md` |
| A-10 | মাপা হয়নি | ledger-এ repeat-read মাপার ব্যবস্থা আছে, run নেই |
| A-11…A-13 | মাপা হয়নি | paid bench |
| A-14 | মাপা হয়নি | `sdcd bench lang` তৈরি; provider-এর গণনা লাগে (DeepSeek-এ চালানো যেত, আপনার সীমার বাইরে রেখেছি) |
| A-15 | মাপা হয়নি | অর্থ-রক্ষা মাপতে model লাগে |
| A-16 | আংশিক | synthetic: −80.4% (২০ server), লুকানো diff ০; আসল fleet বাকি |
| A-17 | মাপা হয়নি | ৪ সপ্তাহের শেখার পরীক্ষা সময় নেয় |
| A-18 | **সত্য (synthetic)** | ৫০টি বিষাক্ত প্রস্তাব → ৫০টিই আটকানো |
| A-19 | **আংশিক (local প্রমাণিত)** | ১০০/১০০; VPS অংশে আসল server লাগে |
| A-20 | মাপা হয়নি | server-এ canary string খুঁজে দেখার পরীক্ষা চালানো হয়নি (বিশ্লেষণ: payload sealed রাখাই নকশা) |
| A-21 | CI-র উপর | তিন OS-এর `cargo test`/UI check CI চালায় — ফল নিচে §৬ |
| A-22 | **আংশিক** | DeepSeek-এ Claude Code-এর চেয়ে ৬৫% কম ($0.0324 → $0.0114), ৬/৬ জোড়ায়, ৯৫% ব্যবধান [৪৯–৭৫]%; ছোট নমুনা, এক model, Codex অনুপস্থিত — `vs-claude-code-bn.md` |

## ৪. যা বানানো হয়েছে (প্রতিটি আইটেম, `PROGRESS.md`-এর সাথে মিলিয়ে)

সম্পূর্ণ (`done`): W1 harness, R1/R2/R3, W3, W6, W7a/W7b, W9 (আংশিক: নিচে), I3, I5, I6 (cloud অংশ), I9, I10, I11, Q2 (mechanism), Q4, Q5, X1, X2, X3, X5 (Apple secret ছাড়া), X6, R4, PAR, AUTO, L5, G34, G35, G37, G39, U0b।

বানানো কিন্তু ডিফল্টে বন্ধ (Q3-এর অপেক্ষায়): W4, W5, I1, I2, I4/L2, L0, L1, L3, I8, L4।

**আংশিক / বাকি (সৎভাবে):**
- **W1** — baseline run বাকি (paid); fleet fixture-এ আসল Docker নেই।
- **R5** — OS sleep-এর notification নেই (SIGTERM/SIGHUP/Ctrl-C আছে)।
- **W9** — Linux-এ API key এখনো ফাইলে (০৬০০); Secret Service বাকি।
- **W4** — tree-sitter নয়, লাইন-প্যাটার্নের extractor।
- **X1** — passkey ও magic link v1 থেকেই; TOTP মডিউল ও route লেখা ও পরীক্ষিত (`cloud/src/totp.ts`, `owner.ts`)।
- **X5** — signing/notarization `release.yml`-এ বসানো, কিন্তু Apple Developer ID secret না থাকায় **release-এর macOS/Windows installer সাইনবিহীন (ad-hoc)** — প্রথম খোলায় OS সতর্ক করবে।
- **Q3** — নিয়মটি `bench::stats::q3`-এ কোডে আছে ও টেস্ট করা; কিন্তু কোনো (model × কৌশল) জোড়ায় এখনো চালানো হয়নি।
- **I12, Q1, G28–G33, G38, G40** — আপনার prompt-এ এগুলোর বর্ণনা নেই এবং `SDC-PLAN-v3.md` repo-তে নেই; `blocked`। এগুলো আমি অনুমান করে বানাইনি।
- **Deferred (D-1):** W2, I6-লোকাল summariser, L6।

## ৫. SDC AI-র limit

আপনার নির্দেশে তুলে দেওয়া হয়েছে: sub-agent-এর ২৪ ধাপ, `/research`-এর ধাপ-সীমা। রাখা হয়েছে (ক্ষতি ঠেকাতে): আপনার দেওয়া budget, একই কাজ ৫ বার পরপর হলে loop-গার্ড,
content-filter retry, tool output-এর আকার-সীমা।

## ৬. Release ও যাচাই

প্রকাশিত: <https://github.com/skilleddesk/SD-Code/releases/tag/v2.0.1> — সাতটি installer ও `SHA256SUMS.txt`:
Windows `x64-setup.exe` + `x64_en-US.msi`, macOS `x64.dmg` + `aarch64.dmg`, Linux `amd64.AppImage` + `amd64.deb` + `x86_64.rpm`।

| পরীক্ষা (commit `558e010`) | ফল |
|---|---|
| CI (rust ×৩ OS, dependency scan, SDC Anywhere e2e, desktop app) | সবুজ |
| Secret scan (gitleaks + credential-file) | সবুজ |
| window check (আসল window, তিন OS) | সবুজ |
| release workflow (চারটি build → draft → publish) | সবুজ |
| `cargo audit` | পরিষ্কার; একটিমাত্র ignore, কারণসহ: `RUSTSEC-2023-0071` (`rsa`, fix নেই; SDC শুধু SSH client) |

আমি নিজে macOS `aarch64.dmg` নামিয়ে `SHA256SUMS.txt`-এর সাথে মিলিয়েছি (মিলেছে), আলাদা কপিতে খুলে চালিয়েছি
(`sdcd 2.0.1`, ১৫ kill → ১৫/১৫ resume, ৮ task বৈধ), তারপর `/Applications/SDC.app` বদলেছি (আগেরটি `~/SDC.app.previous`-এ);
ইনস্টল-করা app-এ DeepSeek দিয়ে আসল turn: সঠিক উত্তর, ফাইল অপরিবর্তিত। 

**যা এখনো সত্যি:**
- installer **সাইনবিহীন** (ad-hoc): macOS-এ প্রথমবার Privacy & Security → Open Anyway, Windows-এ More info → Run anyway। Apple Developer ID ও secret যোগ হলে workflow নিজেই sign ও notarize করবে।
- Windows ও Linux installer আমি নিজে চালাইনি — আমার মেশিন macOS। সেগুলোর জন্য প্রমাণ হলো CI ও window check (আসল window, ওই OS-এ)।
- v2.0.0 কেন ব্যর্থ: তার release run-এ Windows ও Linux build সফল হলেও **দুটি macOS build** (`aarch64` ও `x64`) "Build and upload into the draft" ধাপে ব্যর্থ হয়, তাই publish ধাপ চলেনি এবং কিছু প্রকাশিত হয়নি (আংশিক release ব্যবহারকারীর কাছে যায়নি — workflow-এর draft-then-publish নকশা এটাই চায়)। 2.0.1-এ (`558e010`) macOS installer শুধু তখনই sign করে যখন certificate আছে; এই বদলের পর সব build সফল। ব্যর্থ ধাপের ভেতরের ত্রুটি-বার্তা আমি পড়িনি — কারণ হিসেবে আমি commit-এর বার্তা ও বদলানো কোডের উপর নির্ভর করছি।

## ৭. পুনরায় চালানোর কমান্ড

```sh
cd sdc/sdcd && cargo test --no-fail-fast && cargo clippy --all-targets -- -D warnings
node sdc/protocol/check.mjs
cd sdc/bench && ../sdcd/target/release/sdcd bench validate
../sdcd/target/release/sdcd bench resume --kills 100 --label <label>
../sdcd/target/release/sdcd bench search --root <50k-tree> --label <label>   # tree: bench gen-tree
# টাকা লাগে (ছোট পরীক্ষা চালানো হয়েছে, মোট ≈ $0.04):
../sdcd/target/release/sdcd bench run --models deepseek:deepseek-chat --tier smoke --runs 1 --max-usd 0.50 --label <label>
../sdcd/target/release/sdcd bench run ... --binary bench/bin/v1/sdcd --label <baseline>   # একই task, v1 daemon
../sdcd/target/release/sdcd bench compare <baseline> <label> --report-bn
```
