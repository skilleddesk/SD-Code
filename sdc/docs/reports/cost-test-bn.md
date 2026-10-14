# খরচ কি কমল না বাড়ল? — DeepSeek দিয়ে ছোট পরীক্ষা

তারিখ: 2026-10-14 · খরচ হয়েছে মোট **$0.026** (৩ রান + ১টি cache probe) · key macOS Keychain থেকেই পড়া, কোথাও ছাপা হয়নি।

## ১. প্রশ্ন ও পদ্ধতি

প্রশ্ন: SDC 2.0 (v3) একই কাজে SDC 1.0-র চেয়ে কম খরচ করে, না বেশি?

পদ্ধতি: একই harness, একই তিনটি bench task, একই model — শুধু **daemon** আলাদা।

| দিক | বেসলাইন | প্রার্থী |
|---|---|---|
| daemon | `bench/bin/v1/sdcd` (tag `sdc-baseline-v1`) | HEAD-এর `target/release/sdcd` |
| tag/commit | `274ec6a` | `69367a2` |
| model | `deepseek:deepseek-chat` ($0.14/$0.28 প্রতি 1M) | একই |
| task | `bugfix-01`, `edit-01`, `explain-01` (smoke tier) | একই |
| runs | প্রতিটি task-এ ১ | একই |

কমান্ড (দুটি, লেবেল ছাড়া আলাদা):

```sh
./target/release/sdcd bench run --models deepseek:deepseek-chat \
  --tasks bugfix-01,edit-01,explain-01 --runs 1 --label cost-v1-keychain \
  --max-usd 0.50 --binary ../bench/bin/v1/sdcd

./target/release/sdcd bench run --models deepseek:deepseek-chat \
  --tasks bugfix-01,edit-01,explain-01 --runs 1 --label cost-smoke-v2 --max-usd 0.50
```

**একটা বাধা ছিল, সেটাও ঠিক করা হয়েছে:** প্রথমে `bench/bin/v1/sdcd` কোনো key পাচ্ছিল না ("No API key for deepseek"),
কারণ ওই বাইনারি `keychain` feature ছাড়া বানানো ছিল — অথচ macOS/Windows-এ যাওয়া daemon সবসময় ওই feature নিয়ে
বানানো হয় (`app/scripts/package-daemon.mjs`)। তাই বেসলাইনটি ঠিক একই ভাবে আবার বানানো হয়েছে, আর পুরোনো বাইনারিটি
`bench/bin/v1/sdcd-file-store` নামে রাখা হয়েছে। কারণ `bench/bin/v1/BUILD.txt`-এ লেখা আছে।

## ২. ফলাফল — প্রতিটি token provider-এর নিজের হিসাব

v1 (`bench/results/cost-v1-keychain`, `ledger: false`):

| task | in | cached | out | $ | pass |
|---|---|---|---|---|---|
| `bugfix-01` | 93,372 | — | 2,431 | 0.013753 | ✅ |
| `edit-01` | 26,058 | — | 428 | 0.003768 | ✅ |
| `explain-01` | 15,967 | — | 941 | 0.002499 | ✅ |
| **মোট** | **135,397** | — | **3,800** | **0.020020** | 3/3 |

v2 (`bench/results/cost-smoke-v2`, `ledger: true`):

| task | in | cached | out | $ | pass |
|---|---|---|---|---|---|
| `bugfix-01` | 71,807 | 61,952 | 2,216 | 0.002868 | ✅ |
| `edit-01` | 26,481 | 20,224 | 522 | 0.001305 | ✅ |
| `explain-01` | 22,072 | 15,104 | 994 | 0.001465 | ✅ |
| **মোট** | **120,360** | **97,280** | **3,732** | **0.005638** | 3/3 |

`sdcd bench compare cost-v1-keychain cost-smoke-v2 --report-bn` বলছে:

- billable সাশ্রয় **৬৫.৪%** (median), range ৪১.৪%–৭৯.১%
- input token সাশ্রয় **−১.৬%** (median), range −৩৮.২%–২৩.১%
- pass rate **+০.০ pt** — দুদিকেই ৩/৩, অর্থাৎ কাজ খারাপ হয়নি

মোটের হিসাবে: input **−১১.১%**, output **−১.৮%**, খরচ **−৭১.৮%** ($0.020020 → $0.005638)।

সুতরাং সরাসরি উত্তর: **খরচ বাড়েনি, কমেছে।** আর pass rate একই থেকেছে।

## ৩. কেন কমল — আর কোথায় সতর্ক থাকতে হবে

দুটি আলাদা কারণ, দুটোই মাপা:

1. **v2 এখন cache-hit token পড়ে।** v2-র ledger-এ দেখা যায় step 1-এ `cached = 0`, step 2-এ `4,480/5,276`,
   step 4-এ `7,552/7,718` — অর্থাৎ একই turn-এর step-গুলো একই prefix বারবার পাঠায়, আর DeepSeek সেটা cache থেকে দেয়
   (cache-hit token-এর দাম প্রায় ১/১০)। v1-র protocol-এ cache-এর ঘরই নেই, তাই v1 সব input-কে "cache ছাড়া" ধরে
   হিসাব করত — v1-র invoice সম্ভবত $0.0200-র চেয়ে কম ছিল।
2. **v2 কিছু কাজে কম token পাঠায়।** `bugfix-01`-এ input ৯৩.৪k → ৭১.৮k (**−২৩%**), `edit-01`-এ প্রায় সমান,
   `explain-01`-এ বরং বেশি (১৬.০k → ২২.১k, **+৩৮%**)। অর্থাৎ token-স্তরে লাভ task-ভেদে কমবেশি, median কিছুটা নিচে।

তাই সৎ পাঠ: **SDC-র নিজের receipt এখন সত্যি** (আগে cache-এর ছাড় ধরা পড়ত না), আর প্রকৃত invoice-স্তরের সাশ্রয়
৬৫%-এর চেয়ে ছোট — তবে **বাড়েনি**, কারণ output দুই দিকেই সমান আর `bugfix`-এর মতো ভারী কাজে input সত্যিই কমেছে।
invoice-স্তরে নিশ্চিত তুলনার জন্য DeepSeek-এর billing/dashboard দেখা দরকার; bench কখনো নিজে টাকা গোনে না,
সে provider-এর ফেরত দেওয়া token থেকেই হিসাব করে।

**cache সত্যিই চালু কি না — সেটাও আলাদা করে পরীক্ষা করা হয়েছে** (`_verify/cache-probe.mjs`, খরচ ~$0.001):
একই prompt পরপর দুইবার পাঠানো হয়েছে; দ্বিতীয়বার DeepSeek বলেছে ৪,৫৮৪ input token-এর মধ্যে **৪,৩৫২ cache-hit**
(`prompt_cache_hit_tokens: 4352`, `cache_miss_tokens: 232`)। অর্থাৎ এই অ্যাকাউন্টে prefix cache কাজ করে, আর
v1-র হিসাবটি সেটি গোনেনি — উপরের সীমাটা তাই স্বাভাবিক।

## ৪. যাচাই

- `bench/results/cost-v1-keychain/` ও `bench/results/cost-smoke-v2/` — কাঁচা রেকর্ড: `runs.jsonl`, `ledger.jsonl`, `meta.json`।
- `bench/results/cost-smoke-v2/compare-cost-v1-keychain-bn.md` — `bench compare`-এর নিজের রিপোর্ট।
- `bench/bin/v1/BUILD.txt` — বেসলাইন বাইনারিটির পুনর্নির্মাণের কারণ।
- খরচ: v1 arm $0.0200 + v2 arm $0.0056 + cache probe $0.0007 ≈ **$0.026** (প্রতিটি কমান্ডে `--max-usd 0.50` cap ছিল)।
