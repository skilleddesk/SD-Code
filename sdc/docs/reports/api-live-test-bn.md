# API live test ও paid bench — কী পাওয়া গেল

তারিখ: 2026-10-14 · এটি SDC-র নিজের keychain থেকেই চালানো, key কোথাও ছাপা হয়নি।

## ১. এক prompt করে প্রতিটি provider-এ (শুধু যেগুলোর key আছে)

পদ্ধতি: প্রতিটি provider-এর key macOS Keychain (`dev.skilleddesk.sdc`, account `sdc.provider.<id>`) থেকে পড়া হয়,
তারপর একটি ছোট chat completion পাঠানো হয় (`"Reply with exactly: pong"`)। চারটি provider-এর মধ্যে
তিনটিতেই key আছে, একটিতে নেই।

| Provider | Model | HTTP | সময় | কী হলো |
|---|---|---|---|---|
| `deepseek` | `deepseek-flash` | **200** | 709 ms | উত্তর এসেছে — `pong`, 36 in / 16 out token। কাজ করে |
| `qwen` (Alibaba) | `qwen3.8-flash` | 403 | ~1.8 s | `AccessDenied.Unpurchased` — key ঠিক আছে, কিন্তু অ্যাকাউন্টে কোনো chat model কেনা/চালু নেই |
| `openai-api` | `gpt-5-nano` | 429 | ~1.6 s | `You have no credits remaining` — key ঠিক আছে, ক্রেডিট নেই |
| `openrouter` | — | — | — | key সঞ্চিতই নেই (status `available`), তাই prompt পাঠানো যায়নি |

সিদ্ধান্ত: **এখন SDC-তে সত্যিকারের কাজ করছে শুধু DeepSeek**। বাকি দুটির key বৈধ (401 নয়, 403/429),
কিন্তু অ্যাকাউন্টের পক্ষ থেকে বাধা। কাজ চালানোর আগে Alibaba-তে একটি chat model কেনা এবং OpenAI-তে
ক্রেডিট যোগ করা দরকার — SDC-র কোডে কিছু ভাঙা নেই।

চালানোর স্ক্রিপ্ট: `_verify/api-test.mjs`, `_verify/api-test2.mjs` (ডিপো-র বাইরে, ইচ্ছাকৃতভাবে)।

## ২. Paid bench কেন টাকা লাগে

`sdcd bench` একবারে একটি prompt পাঠায় না — সে **পুরো একটা কাজের কেস-সেট** চালায়:

- প্রতিটি fixture (যেমন `bench/fixtures/ts-shop`) একটি ছোট প্রকল্প, তাতে কতগুলো কেস থাকে (A-11/A-12/A-14 …)।
- প্রতিটি কেস = SDC-র একটি সত্যিকারের turn, অর্থাৎ model-এর সাথে একাধিক বার কথা (প্রতি step-এ একবার)।
- তাই এক রান = কেস × step সংখ্যা = কয়েক ডজন থেকে শত শত model call।
- `sdcd bench compare <baseline> <candidate>` একই কেস-সেট দুবার চালায় (বেসলাইন বনাম প্রার্থী), অর্থাৎ দ্বিগুণ।
- Q3 নিয়ম (p ≥ 0.1, pass-rate drop ≤ 1 point, সাশ্রয় CI > 0) মানার জন্য প্রতি model-এ অন্তত কয়েকটি পুনরাবৃত্তি লাগে।
- Q4-এর SPRT বলছে ছোট (৫ পয়েন্ট) পার্থক্য নিশ্চিত করতে ~120 জোড়া দরকার — মানে আরও অনেক call।
- ভাষা-খরচের probe (A-14) একটি parallel corpus-এ চলে, সেটিও প্রতি লাইনে একটি করে call।

এই কারণেই paid bench-এ টাকা লাগে, আর এই কারণেই আমি সেটি আপনার সম্মতি ছাড়া চালাই না।
আজকের এক-prompt পরীক্ষায় খরচ হয়েছে এক ডলারের অনেক নিচে (DeepSeek-এ ৩৬+১৬ token)।
