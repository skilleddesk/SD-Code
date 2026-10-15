# SDC কি Claude Code / Codex-এর চেয়ে কম খরচ করে? — ছোট পরীক্ষা

তারিখ: ২০২৬-১০-১৫ · DeepSeek-এ প্রকৃত খরচ ≈ $০.০৪৬ (দুই arm + probe); Haiku-র $০.০১৪৬ Claude Code-এর নিজের নামমাত্র হিসাব, আপনার subscription থেকে গেছে · key macOS Keychain থেকে, কোথাও ছাপা হয়নি।

## ১. কী চালানো গেল, কী গেল না

| | অবস্থা | কেন |
|---|---|---|
| **SDC + DeepSeek** | চালানো হয়েছে | key আছে |
| **Claude Code + DeepSeek** (একই model) | চালানো হয়েছে | DeepSeek-এর Anthropic-সঙ্গত endpoint (`api.deepseek.com/anthropic`) দিয়ে |
| **Claude Code + Claude Haiku** (আসল Claude) | চালানো হয়েছে | আপনার claude.ai login দিয়ে |
| **SDC + Claude** | **চালানো যায়নি** | SDC-তে Anthropic API key নেই (`anthropic-api: no key`) |
| **Codex** | **চালানো যায়নি** | `codex login status` = Not logged in; OpenAI key-তে credit নেই (429) |

তাই "একই model-এ SDC বনাম Claude Code" তুলনাটা DeepSeek-এ করা হয়েছে। এতে মাপা হয় **harness-এর নিজের overhead** —
model একই, দাম একই, তফাত শুধু কে কত token পাঠায়।

## ২. পদ্ধতি

একই তিনটি task (`bugfix-01`, `edit-01`, `explain-01`), একই fixture, একই verifier, প্রতিটি ২ বার। খরচ দুদিকেই
**token থেকে DeepSeek-এর দামে** হিসাব (cache-hit input ১/১০ দামে): Claude Code নিজে যে `total_cost_usd` দেখায় (`$0.062` একটি
"pong"-এর জন্য) সেটা Claude-এর দামের তালিকায় হিসাব, DeepSeek-এ ভুল — তাই ব্যবহার করা হয়নি
(`bench::run_competitor` এখন অ-Claude model-এ token থেকে হিসাব করে)।

## ৩. ফল (৬ জোড়া: ৩ task × ২ run)

| | Claude Code | SDC | SDC কম |
|---|---|---|---|
| input token | 923,390 | 252,474 | **৭৩%** |
| cache-hit token | 802,688 (৮৭%) | 207,232 (৮২%) | — |
| output token | 15,069 | 7,876 | ৪৮% |
| **খরচ (DeepSeek দামে)** | **$0.0324** | **$0.0114** | **৬৫%** |
| pass | ৬/৬ | ৬/৬ | সমান |
| wall time (median) | ১৮.৫ s | ৯ s | — |

জোড়া ধরে: **৬টির ৬টিতেই SDC সস্তা**; জোড়া-প্রতি সাশ্রয়ের median **৭২%**, ৯৫% bootstrap ব্যবধান **[৪৯%, ৭৫%]**।
কাঁচা ফাইল: `bench/results/vs-cc-on-deepseek`, `vs-sdc-on-deepseek`; হিসাবের স্ক্রিপ্ট ও আউটপুট
`bench/results/vs-sdc-on-deepseek/compare-vs-cc.py` / `.txt`।

**আসল Claude (Haiku) এ Claude Code**, ৩ task × ১ run: input 319,782 (cache ৮৬%), output 5,362, Claude Code-এর নিজের হিসাবে
$0.0146, pass ৩/৩। এই খরচ DeepSeek-এর সাথে মেলানো যায় না (model আলাদা, দাম আলাদা); কিন্তু token-এর পরিমাণ
(৩ task ≈ ৩.২ লক্ষ) DeepSeek-এ Claude Code-এর (≈ ৪.৬ লক্ষ) কাছাকাছি, আর SDC-র (≈ ১.৩ লক্ষ) চেয়ে অনেক বেশি।

## ৪. কেন এত তফাত — যতটুকু মাপা গেছে

শুধু একটি জিনিস সরাসরি মাপা: Claude Code-কে "Reply with exactly: pong" বলতেও (একটিমাত্র model call) **15,544 input token** লাগে —
তার স্থায়ী system prompt আর tool তালিকা। তুলনায় SDC-র একটি ফাইল-পড়ার ছোট কাজ (২টি model call, মোট ≈ ৮,৮০০ input, মানে call-প্রতি ≈ ৪,৪০০)।
দুটো ঠিক সমান কাজ নয়, তবে ছোট task-এ এই স্থায়ী ভারই বড় অংশ হওয়ার ইঙ্গিত দেয়।
**কোন নির্দিষ্ট কারণে বাকি তফাত, সেটা আমি মাপিনি**, তাই দাবি করছি না।

## ৫. সীমা — এই ফল কতটা বিশ্বাসযোগ্য

- **নমুনা ছোট:** ৩টি task, ৬ জোড়া, ছোট ছোট fixture (কয়েকটি ফাইল)। বড় প্রজেক্টে লম্বা কাজে অনুপাত বদলাতে পারে —
  Claude Code-এর স্থায়ী overhead তখন মোট খরচের ছোট অংশ হবে, SDC-র সুবিধা কমবে।
- **এক model:** DeepSeek। Claude, GPT, Gemini, Qwen-এ এই তুলনা হয়নি।
- **Claude Code-এর "আসল" ক্ষেত্র নয়:** Claude Code Claude model-এর জন্য বানানো; DeepSeek endpoint-এ চললে তার কিছু আচরণ
  (যেমন পাঠানো tool-এর ধরন) অপ্টিমাইজড নয়, এতে সে বেশি token পাঠাতে পারে। অর্থাৎ ৬৫% কিছুটা Claude Code-কে প্রতিকূল অবস্থায় ফেলার ফল হতে পারে।
- **Codex অনুপস্থিত**, তাই "Claude Code / Codex-এর চেয়ে কম" — Codex অংশ **মাপা হয়নি**।
- Cache-hit দুই দিকেই provider-এর নিজের রিপোর্ট (`prompt_cache_hit_tokens`), ধরে নেওয়া নয়।

## ৬. Assumption-এ প্রভাব

- **A-09** (Claude Code/Codex আগে থেকেই caching ব্যবহার করে): **Claude Code-এর জন্য প্রমাণিত** — input-এর ৮৭% (DeepSeek-এ) ও ৮৬% (Haiku-তে) cache থেকে। Codex মাপা হয়নি।
- **A-22** (একই model-এ Claude Code/Codex-এর চেয়ে ≥৪০% কম খরচ): **আংশিক** — DeepSeek-এ Claude Code-এর চেয়ে ৬৫% কম, ৬/৬ জোড়ায়; Codex ও অন্য model বাকি, নমুনা ছোট।

## ৭. পুনরায় চালানো

```sh
cd sdc/bench
KEY=$(security find-generic-password -s dev.skilleddesk.sdc -a sdc.provider.deepseek -w)
ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic ANTHROPIC_AUTH_TOKEN="$KEY" \
ANTHROPIC_MODEL=deepseek-chat ANTHROPIC_SMALL_FAST_MODEL=deepseek-chat CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
  ../sdcd/target/release/sdcd bench competitors --models claude-code:deepseek-chat \
  --tasks bugfix-01,edit-01,explain-01 --runs 2 --label vs-cc-on-deepseek --max-usd 0.60
../sdcd/target/release/sdcd bench run --models deepseek:deepseek-chat \
  --tasks bugfix-01,edit-01,explain-01 --runs 2 --label vs-sdc-on-deepseek --max-usd 0.60
cd results && python3 -I vs-sdc-on-deepseek/compare-vs-cc.py
```
