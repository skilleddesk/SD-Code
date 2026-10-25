PADMA — শেষ পর্যন্ত কাজ: SDC-তে ছোট মডেলে সম্পূর্ণ কাজের পরিকল্পনা
Oct 25, 2026 · @Mehedi Hasan
সারাংশ
PADMA-র একটাই প্রতিশ্রুতি: ছোট প্রম্পটে বড় কাজ দিলেও SDC কাজ শেষ না করে, বা সৎভাবে "blocked" না বলে, কখনো থামবে না। পদ্মা যেমন সাগর পর্যন্ত পৌঁছায়, তেমনি কাজও শেষ পর্যন্ত যাবে। ট্রিগার শব্দ: PADMA।
মূল দর্শন — চালক SDC, শ্রমিক মডেল। এখন মডেল নিজেই ঠিক করে কখন থামবে। কোলন লিখে থেমে গেলেও SDC ধরে নেয় কাজ শেষ। PADMA-তে এই ক্ষমতা মডেলের হাত থেকে কোডের হাতে যায়। কাজ কোন ধাপে চলবে, কোন ফাইল দেখবে, কখন শেষ হবে, সব ঠিক করে SDC-র Conductor (একটা state machine)। 9B প্রতিবার শুধু একটা ছোট, স্পষ্ট, যাচাইযোগ্য কাজ করে।
তিনটি স্তম্ভ:
1. Conductor + Finish Gate: turn শেষ হয় কোডের সিদ্ধান্তে, মডেলের নীরবতায় নয়।
2. Workflow Recipe: "রিভিউ করো", "পেজ বানাও", "বাগ ঠিক করো" — প্রতিটির জন্য আগে থেকে বানানো ধাপের নকশা। ছোট মডেলকে খোলা লুপে ছেড়ে দেওয়া হয় না।
3. Model Adapter: যেকোনো মডেলের টুল-ফরম্যাট, thinking আর ফাঁকা-উত্তরের সমস্যা কোড সামলায়। ফলে শুধু Qwen নয়, যেকোনো ছোট মডেলে কাজ চলে।
MEGHNA ও NAVARATNA-র সাথে সম্পর্ক: PADMA এদের বদলায় না, এদের উপরে বসে। NAVARATNA-র ২৯টি ইন্টেলিজেন্স (তাজা ধাপ, skeleton, symbol-প্যাচ, test-first, Best-of-N) এখন মূলত বড় প্রম্পটে আর সুইচ চালু থাকলে চলে। PADMA ঠিক করে কখন সেগুলো চালু হবে: প্রম্পটের দৈর্ঘ্য দেখে নয়, কাজের আকার দেখে। MEGHNA-র R1–R11 আর NAVARATNA-র N1–N7 নিয়ম হুবহু প্রযোজ্য।
ফাইলের শুরুতে বসানোর skill-হেডার:
---
name: padma
description: "PADMA plan for SDC: a code-driven Conductor, Finish Gate and workflow recipes so any small local model (9B) finishes short-prompt big tasks (review a project or website, build a page) without stopping midway. Builds on MEGHNA and NAVARATNA. Whenever the user's message contains the word PADMA, read this file and PADMA-PROMPTS.md (same folder) completely before doing anything else."
---
সৎ কথা: 9B-র বুদ্ধি এতে বাড়বে না। তবে "অল্প কাজ করে থেমে যাওয়া" কাঠামোগত সমস্যা, আর কাঠামো দিয়েই এটা বন্ধ করা যায়। কোন কাজ 9B সত্যিই পারে না, সেটা রোটেশনে যাবে, অথবা সৎভাবে blocked দেখাবে।
ঘটনা-বিশ্লেষণ: ২৫ অক্টোবরের ব্যর্থতার ১০টি মূল কারণ
দুই লাইনের প্রম্পট ছিল: "পুরো SDC প্রজেক্ট দেখো, error রিপোর্ট করো"। ফলে ছয়টি turn-এ একটা লাইনও রিপোর্ট আসেনি। প্রথম turn ২৫ স্টেপ চলে ContextOverflow-এ থামে। তখন ধাপটা ছিল ১২,৩৯৭ টোকেন, সাথে উত্তরের জন্য ৪,০৯৬ রাখা — মোট ১৬,৪৯৩, যেখানে জানালা ১৬,৩৮৪। প্ল্যানে ছিল "0 of 4 done"। পরের চারটি উত্তর ছিল 1 step · tools 0.0s, প্রতিটি কোলনে শেষ, অথচ Trust 100।
ID
মূল কারণ
প্রমাণ (স্ক্রিনশট)
ধরন
RC1
মডেল থামলেই turn শেষ। SDC-র কোনো Finish Gate নেই
"build করে দেখছি:" লিখে থেমে যাওয়া, Done দেখানো
কাঠামো
RC2
টুল-কল হারিয়ে যায়: qwen3.5 টেক্সটে লেখে বা শুধু thinking দেয়
tools 0.0s, "empty answer"
মডেল-অ্যাডাপ্টার
RC3
পুরো কাজ এক লম্বা চ্যাট-লুপে, টুল-আউটপুট জমে
২৫ স্টেপ পরে overflow
context
RC4
ContextOverflow চূড়ান্ত ব্যর্থতা। "This failure has no rule yet"
১০৯ টোকেন বেশি, সব কাজ বাতিল
context
RC5
বড়-কাজ শনাক্ত হয় প্রম্পটের দৈর্ঘ্যে (Brick Mode ≥৩৫% T_in)
দুই লাইনের প্রম্পটে NAVARATNA ইঞ্জিন চালু হয়নি
কাঠামো
RC6
NAVARATNA-র মূল সুইচ ডিফল্টে বন্ধ (nr.fresh_step, nr.tool_envelope)
step-executor আর prose-পুনঃপ্রশ্ন চলেনি
কনফিগ
RC7
কাজের অবস্থা সংরক্ষিত হয় না; error পেস্ট বা "ok koro" নতুন কাজ হয়ে যায়
"0 of 4 done", পরের turn-এ নতুন করে শুরু
অবস্থা
RC8
টুল-error অস্পষ্ট; পথ ভুল হলে মডেল কারণ বানায়
os error 2/3 (পথ নেই) কে "permission" বলা
টুল
RC9
Trust স্কোর প্রমাণ দেখে না
কিছু না করা উত্তরে Trust 100 · high
যাচাই
RC10
সিস্টেম নোটিশ মডেলের টেক্সট/history-তে; ভেতরের লুপ বাংলায়
GPU নোটিশ উত্তরে দুবার; history ৪০%
context
মূল শিক্ষা: NAVARATNA-র ইঞ্জিনগুলো ঠিক আছে, কিন্তু সেগুলো ভুল সময়ে ঘুমিয়ে থাকে। আর কাজ শেষ হয়েছে কি না, সেই সিদ্ধান্ত এখনো মডেলের হাতে। PADMA এই দুটো জিনিস বদলায়।
গবেষণা-ভিত্তি: কেন এই নকশা
গবেষণার রায়: ছোট মডেলকে খোলা এজেন্ট-লুপে স্বাধীনভাবে ছেড়ে দেওয়ার চেয়ে কোড-চালিত নির্দিষ্ট ধাপের pipeline বেশি নির্ভরযোগ্য। আর পুরোনো টুল-আউটপুট context থেকে সরিয়ে দেওয়া সবচেয়ে সস্তা ও কার্যকর context-ব্যবস্থাপনা।
#
ফলাফল
PADMA-তে কী নেওয়া হলো
উৎস
১
Agentless তিন ধাপে কাজ করে: localization, repair, patch validation। পরের কাজ কী হবে, তা মডেল ঠিক করে না। এই সরল pipeline তখনকার সব open-source এজেন্টকে কম খরচে ছাড়িয়েছিল।
Workflow Recipe: ধাপের ক্রম কোড ঠিক করে, মডেল শুধু প্রতিটি ধাপ করে
Agentless
২
Agentless প্রথমে ফাইল, তারপর ক্লাস/ফাংশন, শেষে সুনির্দিষ্ট লাইন — এভাবে স্তরে স্তরে খোঁজে। একাধিক patch বানিয়ে টেস্ট দিয়ে বাছে।
রিভিউ ও ফিক্স রেসিপিতে স্তরভিত্তিক localization; NAVARATNA-র Best-of-N
Agentless repo
৩
SWE-agent-এ পুরোনো টুল-আউটপুট লুকিয়ে রাখা (observation masking) খরচ অর্ধেকের বেশি কমিয়েছে। সমাধানের হার LLM-সারাংশের সমান, কখনো একটু বেশি।
Context Budget Engine: পুরোনো observation এক লাইনের হ্যান্ডেলে নামবে, সারাংশের জন্য আলাদা LLM-কল নয়
Complexity Trap, NeurIPS 2025
৪
কোডিং এজেন্টের context-এর সিংহভাগ জুড়ে থাকে environment observation (টুল-আউটপুট)।
প্রতিটি টুল-আউটপুটে কড়া সীমা; বাকিটা ভল্টে
Complexity Trap
৫
Ollama-তে qwen3.5:9b কখনো টুল-কল চালানোর বদলে টেক্সট হিসেবে লিখে দেয়।
Model Adapter: টেক্সট-রূপ <tool_call> পার্স
ollama#14745
৬
qwen3.5:9b প্রায়ই শুধু thinking দেয়, content ফাঁকা থাকে, টুল-কল হয় না। ফলে কাজ বাদ পড়ে, উত্তর অসম্পূর্ণ থাকে।
ফাঁকা-উত্তর মেরামতের ধাপ (Recovery Ladder)
zeroclaw#3079
৭
Qwen 3.5 প্রশিক্ষিত Qwen3-Coder XML টুল-ফরম্যাটে, কিন্তু Ollama পাঠায় অন্য ফরম্যাটে। thinking-সহ টুল-কলে </think> বন্ধ না হওয়ায় বহু-turn প্রম্পট নষ্ট হয়।
মডেল-প্রোফাইলে সঠিক টুল-ফরম্যাট; history-তে thinking কখনো ফেরত নয়
ollama#14493
৮
Qwen3.5 ডিফল্টে thinking mode-এ চলে।
টুল-ধাপে thinking বন্ধ, পরিকল্পনা ও মেরামতে চালু
Qwen3.5-9B model card
৯
কিছু ক্লায়েন্ট reasoning মডেলের উত্তর ভুল ফিল্ড থেকে পড়ে, ফলে উত্তর ফাঁকা দেখায়।
SDC-র Ollama ক্লায়েন্টে message.content, message.thinking, message.tool_calls তিনটিই পড়া
hermes-agent#46131
NAVARATNA-র উৎসগুলো (Aider, SWE-agent, AlphaCodium, Large Language Monkeys, RULER, Lost in the Middle) এখানেও প্রযোজ্য। সংখ্যাগুলো প্রেক্ষাপট মাত্র; SDC-র নিজের bench ছাড়া কোনো উন্নতির দাবি করা হবে না (N2)।
স্থাপত্য: Conductor কীভাবে কাজ চালায়
Conductor হলো SDC-র ভেতরের একটি state machine, যা প্রতিটি কাজকে ছোট ধাপে ভেঙে একটার পর একটা চালায়। মডেল কখন থামবে তা মডেল ঠিক করে না। প্রতিটি ধাপ শেষে Finish Gate দেখে Task Contract-এর সব done-when পূরণ হয়েছে কি না।
মডেল ফাঁকা উত্তর দিলে বা শুধু "করছি" লিখে থামলে কাজ Recovery Ladder-এ যায়, শেষ হয় না। Finish Gate "না" বললে Ledger থেকে পরের ধাপ শুরু হয়। কোনো একক সত্যিই না পারলে সেটা সৎ blocked হয়, বাকি ধাপ চলতে থাকে।
Task Contract হলো ইংরেজিতে লেখা একটি ছোট কাঠামো, যা Scope Classifier প্রতিটি অনুরোধ থেকে বানায়: goal, recipe, scope (কোন ফোল্ডার/URL), done_when[], deliverable (রিপোর্ট/ফাইল/patch), language_out। এটা Ledger-এ জমা থাকে। "ok koro", "continue" বা পেস্ট করা error এই Contract-এর সাথেই যুক্ত হয়, নতুন কাজ হয় না।
দশটি "আর কখনো নয়" গ্যারান্টি
প্রতিটি মূল কারণের বিপরীতে একটি গ্যারান্টি আছে, আর প্রতিটির সাথে একটি regression test। ২৫ অক্টোবরের ছয়টি turn রেকর্ড করা fixture হিসেবে চিরকাল CI-তে চলবে। এর কোনো একটি আবার ঘটলে build ফেল করবে।
ID
গ্যারান্টি
কীভাবে নিশ্চিত হয়
মূল কারণ
Regression test
G1
মডেল থামলেই কাজ শেষ নয়
Finish Gate: Task Contract-এর done-when পূরণ, অথবা সৎ blocked — এ ছাড়া turn শেষ হয় না
RC1
never_again::announce_then_stop
G2
"করছি:" বলে থামা ধরা পড়বে
Intent-without-Action ডিটেক্টর → সর্বোচ্চ ২ বার "এখনই টুল ডাকো" → তারপর Recovery Ladder
RC1, RC2
never_again::colon_end_no_tool
G3
টুল-কল হারাবে না
Model Adapter: নেটিভ tool_calls, টেক্সট-রূপ <tool_call>, XML/JSON — সব পার্স হবে; thinking ফিল্ড পড়া হবে
RC2
adapter::qwen35_text_toolcall, adapter::thinking_only
G4
ফাঁকা উত্তরে কাজ থামবে না
Recovery Ladder: আবার চেষ্টা → think বন্ধ → ছোট context → রোটেশন → সৎ blocked
RC2
never_again::empty_answer
G5
ContextOverflow কখনো চূড়ান্ত নয়
Overflow Rule: masking → ভল্টে সরানো → ধাপ ভাঙা → তারপর আবার চেষ্টা
RC3, RC4
never_again::overflow_by_109
G6
ছোট প্রম্পটে বড় কাজ চিনবে
Scope Classifier: অভিপ্রায় + রিপোজিটরির আকার দেখে Recipe বাছবে
RC5, RC6
scope::short_prompt_full_review
G7
কাজ হারাবে না
Task Ledger: প্রতিটি ধাপ ডিস্কে; "ok koro", "continue" বা পেস্ট করা error মানে একই কাজ চালিয়ে যাওয়া
RC7
never_again::resume_after_fail
G8
পথ-ভুলে মিথ্যা রোগনির্ণয় হবে না
টুল-error-এ OS কোডের সঠিক অর্থ + cwd + কাছাকাছি আসল পথ; স্পেসওয়ালা পথ quote
RC8
tools::win_os_error_2_3_5
G9
প্রমাণ ছাড়া Trust high নয়
Evidence-based Trust: টুল-প্রমাণ নেই কিন্তু কাজের দাবি আছে → সর্বোচ্চ "low"
RC9
trust::no_evidence_claim
G10
মডেলের context পরিষ্কার থাকবে
সিস্টেম নোটিশ শুধু UI-তে; ভেতরের লুপ ইংরেজিতে, শুধু চূড়ান্ত রিপোর্ট ব্যবহারকারীর ভাষায়
RC10
context::no_ui_notice_in_history
Workflow Recipe: পাঁচটি কাজের নকশা
প্রতিটি Recipe হলো ধাপের একটি DAG, যেখানে প্রতিটি ধাপ তিন ধরনের: কোড (LLM লাগে না), মডেল (ছোট, তাজা context-এ একটি কাজ), বা গেট (যাচাই)। নিয়ম হলো: যা কোড পারে, তা কখনো মডেলকে দেওয়া হবে না। Recipe-গুলো recipes/*.toml-এ থাকবে, তাই নতুন Recipe যোগ করতে কোড বদলাতে হবে না।
R-REVIEW-CODE — "পুরো প্রজেক্ট রিভিউ করো / error খুঁজে দাও"
1. কোড — Inventory: ফাইল-তালিকা, ভাষা, আকার, entry point। .gitignore, node_modules, target, build-আউটপুট বাদ।
2. কোড — আসল error সংগ্রহ: প্রজেক্টের ধরন দেখে চালানো হবে, যেমন cargo check, cargo clippy, tsc --noEmit, eslint, npm test, cargo test। এগুলোর আউটপুট হলো নিশ্চিত error, মডেলের অনুমান নয়। প্রতিটি error Error Slicer দিয়ে কাটা হবে।
3. কোড — ঝুঁকি-ক্রম: সম্প্রতি বদলানো, বড়, error-ওয়ালা আর বেশি-import-হওয়া ফাইল আগে। UI চাইলে UI ফোল্ডার আগে।
4. মডেল — ফাইল-ভিত্তিক রিভিউ (map): প্রতিটি ফাইল বা ফাইল-টুকরোর (≤ ৫০% T_in) জন্য তাজা কল। ইনপুটে থাকবে skeleton, ফাইলের বডি, ধাপ-২-এর সংশ্লিষ্ট error, আর একটি চেকলিস্ট। আউটপুট নির্দিষ্ট ফরম্যাটে: file:line · severity · category · one-line problem · evidence line। কিছু না পেলে NONE।
5. গেট — প্রমাণ-যাচাই: প্রতিটি finding-এর evidence line ফাইলে হুবহু আছে কি না কোড মিলিয়ে দেখবে। না মিললে বাদ (বানানো বাগ বন্ধ)।
6. কোড — Findings Ledger: ডিস্কে জমা, ডুপ্লিকেট বাদ, severity অনুযায়ী সাজানো। প্রগ্রেস দেখাবে: "ফাইল ৩৭/১২০ · findings ১৪"।
7. মডেল — চূড়ান্ত রিপোর্ট (reduce): Ledger থেকে, ব্যবহারকারীর ভাষায় (বাংলায়)। লম্বা হলে section ধরে লেখা হবে (N4-4)।
8. গেট — Finish: সব ফাইল দেখা হয়েছে, অথবা কোনটা বাদ গেছে তার সৎ তালিকা। তার আগে turn শেষ হবে না।
R-REVIEW-SITE — "আমার ওয়েবসাইট রিভিউ করো"
শুরুতে কোড ঠিক করবে লক্ষ্য কী: লোকাল কোড, VPS-এর ফোল্ডার, নাকি লাইভ URL।
1. কোড: লাইভ URL হলে HTTP status, redirect, ভাঙা লিংক, অনুপস্থিত meta/title/alt, ভারী ছবি, console error, mixed content। VPS হলে nginx/Apache config test (nginx -t), SSL মেয়াদ, ডিস্ক আর লগের শেষ error।
2. তারপর সোর্স কোড থাকলে R-REVIEW-CODE-এর ধাপ ৩–৮।
3. রিপোর্টে তিনটি অংশ: ভাঙা জিনিস, নিরাপত্তা-ঝুঁকি, উন্নতির পরামর্শ।
নেটওয়ার্ক বা VPS অ্যাক্সেস না থাকলে Recipe এই অংশকে সৎভাবে skipped (no access) লেখে, থামে না।
R-BUILD-PAGE — "একটা ওয়েব পেজ/অ্যাপ বানাও"
1. মডেল বা ক্লাউড — Blueprint: ফাইল-তালিকা, প্রতিটি ফাইলের দায়িত্ব, contract, done-when। ছোট প্রম্পটে থাকে না এমন সিদ্ধান্ত (রং, section) Blueprint যুক্তিসঙ্গত ডিফল্ট দিয়ে পূরণ করবে আর রিপোর্টে লিখবে। প্রশ্ন করে থামবে না।
2. কোড: Blueprint-এর বাক্য-কভারেজ আর contract যাচাই (N2-1, N6-3)।
3. মডেল: প্রতিটি ফাইল আলাদা তাজা ধাপে, symbol-প্যাচ বা নতুন ফাইল।
4. গেট: parse/lint, build, HTML validator; ধাপে ধাপে Git Shadow।
5. গেট — Finish: Blueprint-এর সব ফাইল আছে, build সবুজ, প্রতিটি R-ID প্রমাণসহ।
R-FIX — "এই বাগ/error ঠিক করো"
Agentless-ধাঁচ: reproduce (কোড) → স্তরভিত্তিক localization (ফাইল → symbol → লাইন) → patch (9B, ব্যর্থ হলে Best-of-N) → টেস্ট-বাছাই → regression → Finish।
R-ANSWER — প্রশ্ন বা ব্যাখ্যা
ছোট প্রশ্নে খোলা লুপ চলবে, কিন্তু G2 আর G9 সবসময় চালু থাকবে। উত্তর দাবি করলে টুল-প্রমাণ লাগবে, আর না থাকলে উত্তরে স্পষ্ট লেখা থাকবে "যাচাই করিনি"।
Model Adapter: যেকোনো ছোট মডেলে নির্ভরযোগ্য টুল-কল
Model Adapter হলো মডেল আর Conductor-এর মাঝের স্তর। Conductor শুধু পরিষ্কার একটা StepResult { text, tool_calls, thinking, finish_reason } দেখে। প্রতিটি মডেলের নিজস্ব অদ্ভুততা Adapter-এর ভেতরে থাকে। ফলে Qwen, Llama, Gemma, DeepSeek-এর ছোট সংস্করণ বা ভবিষ্যতের যেকোনো মডেল একই Recipe চালাতে পারে।
১. Model Profile (~/.sdc/models/<model>.toml): প্রতিটি মডেলের জন্য একটি প্রোফাইল। ফিল্ডগুলো: টুল-ফরম্যাট (native / hermes-json / qwen3-coder-xml), thinking নিয়ন্ত্রণ (ধাপ-ধরন অনুযায়ী on/off), sampling (temperature, top_p, top_k, presence_penalty), মাপা কার্যকর জানালা (N1-1), quantization, আর এক প্রম্পটে সর্বোচ্চ কতটি টুল দেওয়া যাবে। প্রথমবার মডেল পেলে SDC নিজেই একটা ছোট Capability Probe চালায়: ৫টি টুল-কল পরীক্ষা, ফাঁকা উত্তরের হার, কোন ফরম্যাট কাজ করে। ফল প্রোফাইলে লেখা হয়।
২. সহনশীল পার্সার: ক্রমানুসারে যা খোঁজা হবে:
1. Ollama-র নেটিভ message.tool_calls
2. content-এ টেক্সট-রূপ <tool_call>…</tool_call> (JSON বা XML)
3. thinking-এর ভেতরে লুকানো টুল-কল
4. code fence-এ মোড়ানো JSON
পাওয়া কল schema দিয়ে যাচাই হবে। ভুল আর্গুমেন্ট হলে ছোট error ফেরত যাবে (N7-2)।
৩. Thinking-নীতি: টুল-কল ধাপ, boilerplate আর ফাইল-রিভিউয়ের map ধাপে thinking বন্ধ। Blueprint, ব্যর্থতার পর মেরামত আর চূড়ান্ত reduce-এ চালু, সাথে বাজেট (N7-3)। আগের turn-এর thinking কখনো history-তে ফেরত যাবে না।
৪. Recovery Ladder (ফাঁকা বা অর্থহীন উত্তরে):
1. একই অনুরোধ, এক লাইনের nudge সহ: "Respond with exactly one tool call or FINAL."
2. thinking বন্ধ করে আবার
3. context আরও ছোট (masking বাড়িয়ে, ধাপ ভেঙে)
4. Best-of-N (তাপমাত্রা বদলে ২–৩টি নমুনা)
5. রোটেশন chain-এর পরের মডেল (relay.auto চালু থাকলে)
6. সৎ blocked — কারণসহ। স্বাধীন বাকি ধাপ চলতে থাকবে।
৫. Intent-without-Action ডিটেক্টর: টুল-কল নেই, কাজ শেষ নয়, আর লেখায় ভবিষ্যৎ-অভিপ্রায় আছে (যেমন "I'll", "Let me", "now checking", "দেখছি", "করছি", কোলনে শেষ)। তাহলে উত্তরটা চূড়ান্ত ধরা হবে না। এই ডিটেক্টর ভাষা-নিরপেক্ষ: শব্দ-তালিকা ছাড়াও কাঠামো দেখে — শেষ অক্ষর :, অসমাপ্ত তালিকা, "ধাপ ১" আছে কিন্তু ধাপ ২ নেই।
৬. এক ধাপে এক টুল: ছোট মডেলে প্রতিটি কল-এ শুধু সেই ধাপের ২–৪টি প্রাসঙ্গিক টুল দেওয়া হবে (MEGHNA-র M2-1 tool_search ধারণা)। ২৫টি টুলের তালিকা ছোট মডেলকে বিভ্রান্ত করে, আর context খায়।
মালিকের যাচাই: Ollama-র সংস্করণ হালনাগাদ রাখা, আর ollama show qwen3.5:9b --modelfile দিয়ে template দেখা। Probe-এর ফলে টেক্সট-রূপ টুল-কলের হার বেশি দেখালে সেটা Ollama-র template বা parser-এর সমস্যা — SDC-র পার্সার তখনও কাজ চালিয়ে যাবে।
Context Budget Engine: জানালা আর কখনো উপচাবে না
নিয়ম একটাই: প্রতিটি মডেল-কলের আগে কোড টোকেন গোনে। ১০৯ টোকেন বেশি হলেও কল পাঠানোর আগেই ছোট করা হয়, কাজ বাতিল হয় না।
১. টুল-আউটপুট সীমা: প্রতিটি টুল-আউটপুট সর্বোচ্চ obs_cap টোকেন (ডিফল্ট: T_in-এর ১০%, ১৬K জানালায় ~১.২K)। বাকি অংশ ভল্টে যায় ‹obs:17› হ্যান্ডেলসহ, আর মডেল চাইলে obs_read(17, range) দিয়ে আনতে পারে। ডিরেক্টরি-তালিকা গাছ আকারে, গভীরতা-সীমিত। ফাইল-পড়া হয় range ধরে, পুরো ফাইল নয়।
২. Observation Masking: শেষ k টি (ডিফল্ট ৩) টুল-আউটপুট পুরো থাকে। পুরোনোগুলো এক লাইনে নামে, যেমন [read_file src/app.rs 1-120 → ‹obs:12›, 3 findings noted]। মডেলের নিজের সিদ্ধান্ত-লাইনগুলো থাকে। এটা গবেষণা-৩ অনুযায়ী আলাদা LLM-সারাংশের চেয়ে সস্তা।
৩. Overflow Rule ("This failure has no rule yet" এর স্থায়ী সমাধান): আকার-গণনা জানালা ছাড়ালে, ক্রমানুসারে:
1. masking-এর k কমানো (৩ → ১)
2. Context Pack-এর বাদ-ক্রম মানা (NAVARATNA: ৭ → ৬ → ৪-এর বডি)
3. টুল-ধাপে output_reserve কমানো (৪০৯৬ → ১৫৩৬; টুল-কলে লম্বা আউটপুট লাগে না)
4. ধাপ ভাঙা (ফাইল-টুকরো ছোট করা)
5. এরপরও না আঁটলে Recipe সেই একক blocked (too large) চিহ্নিত করে, আর পরের এককে যায়
কোনো অবস্থাতেই পুরো turn ব্যর্থ হয় না।
৪. Loop Ceiling নয়, Phase Budget: "প্রতি turn-এ ২৫ স্টেপ" ধরনের সীমা পুরো কাজে নয়, প্রতিটি মডেল-ধাপে প্রযোজ্য (যেমন একটি ফাইল-রিভিউতে সর্বোচ্চ ৮ টুল-কল)। পুরো Recipe-এর মোট সময় আর ধাপ গোনা হয় Task Ledger-এ। লম্বা কাজ একাধিক turn জুড়ে চলতে পারে, আর UI-তে প্রগ্রেস দেখায়।
৫. পরিষ্কার history: UI-নোটিশ (GPU %, ধীরগতির সতর্কতা), Trust ব্যাজ আর রসিদ কখনো মডেলের বার্তায় বা history-তে ঢোকে না। এগুলো আলাদা UI ইভেন্ট। ভেতরের লুপের ভাষা ইংরেজি; ব্যবহারকারীর বাংলা/Banglish অনুরোধ একবার ইংরেজি Task Contract-এ রূপান্তর হয় (মূলটা ভল্টে থাকে)।
৬. GPU-ফিট পরামর্শ: "৮৬% GPU-তে" দেখালে K3 মেমরি-বাজেটকারী দুটি বিকল্প দেখাবে — ছোট num_ctx দিয়ে পুরোটা GPU-তে রাখা, অথবা KV cache q8_0। তবে কোনোটাই নিজে থেকে বদলাবে না, কারণ এটা ব্যবহারকারীর সিদ্ধান্ত।
কার্যপ্রণালী: ট্রিগার, নিয়ম, রিপোর্ট
Skill হিসেবে দুটি ফাইল একই ফোল্ডারে থাকবে: PADMA.md (এই পরিকল্পনা) আর PADMA-PROMPTS.md (ইংরেজি টাস্ক-কার্ড), MEGHNA আর NAVARATNA-র পাশে। বার্তায় PADMA শব্দ থাকলে AI দুটো পুরো পড়বে, তারপর কাজ করবে।
ব্যবহারকারী লিখলে
কী হবে
PADMA next
ট্র্যাকারের প্রথম todo, যার সব নির্ভরতা done
PADMA P3-2
শুধু সেই টাস্ক; নির্ভরতা বাকি থাকলে থেমে বাংলায় জানানো
PADMA status
কোড বদল নয়; ট্র্যাকার + git log থেকে বাংলায় অবস্থা
PADMA audit
কোড বদল নয়; RC1–RC10 আর G1–G10 এখনো কোথায় খোলা
PADMA <মুক্ত বাক্য>
কোন টাস্কের সাথে মেলে এক লাইনে বলে সেটা করা; না মিললে একটি প্রশ্ন
উত্তরাধিকার: MEGHNA-র কাজের চক্র আর R1–R11, আর NAVARATNA-র N1–N7 হুবহু প্রযোজ্য।
PADMA-র বাড়তি নিয়ম:
• D1 সুইচ: নতুন আচরণ padma.* সুইচে। শুধু রক্ষা-জাতীয় সুইচ ডিফল্টে চালু: padma.finish_gate, padma.intent_guard, padma.tolerant_parse, padma.overflow_rule, padma.obs_cap, padma.clean_history, padma.evidence_trust। কারণ এগুলো ক্ষতি থামায়, প্রম্পটের অর্থ বদলায় না। Recipe-রাউটার (padma.recipes) ডিফল্টে বন্ধ, bench-এর পরে চালু।
• D2 Never-Again: RC1–RC10-এর প্রতিটি fixture-test CI-তে বাধ্যতামূলক। কোনো টাস্ক এগুলোর একটিও ভাঙলে done হবে না।
• D3 কোড আগে: যে কাজ কোড পারে (গোনা, তালিকা, build, lint, error সংগ্রহ, প্রমাণ মেলানো), তা কখনো মডেলকে দেওয়া যাবে না।
• D4 Finish: turn শেষের সিদ্ধান্ত শুধু Finish Gate নেবে। মডেলের "done" দাবি শুধু একটি সংকেত।
• D5 মডেল-নিরপেক্ষ: Conductor বা Recipe-এ কোনো মডেলের নাম hardcode করা যাবে না; সব পার্থক্য Model Profile-এ।
• D6 প্রশ্ন নয়, ডিফল্ট: ছোট প্রম্পটে অস্পষ্টতা থাকলে Recipe যুক্তিসঙ্গত ডিফল্ট নেবে আর রিপোর্টে লিখবে। কাজের শুরুতে থেমে প্রশ্ন করা যাবে না। ব্যতিক্রম: ধ্বংসাত্মক কাজ (delete, deploy, push), যেখানে অনুমতি লাগবেই।
• D7 বিশ্বাসযোগ্য অগ্রগতি: প্রতিটি ধাপ Task Ledger-এ। SDC বন্ধ হলেও পরে "continue" দিয়ে ঠিক সেখান থেকে চলবে।
রিপোর্টের ফরম্যাট (বাংলায়):
PADMA <ID> — <কাজের নাম>
অবস্থা: done / in-progress (ধাপ n/m) / blocked (<কারণ>)
যা করেছি: (২–৫ লাইন)
যাচাই: (প্রতিটি কমান্ড ও আসল ফল; যা চালাইনি তা স্পষ্ট)
Never-Again: (কোন G-test পাস, কোনটি চালানো হয়নি)
নতুন সুইচ: (নাম, ডিফল্ট)
মালিকের কাজ: (থাকলে)
পরের কাজ: <ID>
Commit ফরম্যাট: padma(P3-2): <English summary>।
টাস্ক তালিকা ও ট্র্যাকার
মোট ২৬টি টাস্ক, পাঁচ ফেজে। P0 আগে, কারণ এগুলো ২৫ অক্টোবরের ব্যর্থতা সরাসরি থামায় আর ছোট। প্রতিটির ইংরেজি কার্ড PADMA-PROMPTS ডকে। AI প্রতিটি টাস্ক শেষে "অবস্থা" আর commit কলাম হালনাগাদ করবে।
ID
কাজ
গ্যারান্টি
নির্ভরতা
সুইচ (ডিফল্ট)
অবস্থা
commit
P0-1
Never-Again fixture: ছয়টি turn VCR-এ, ১০টি test (প্রথমে লাল)
সব
—
—
done

P0-2
Ollama ক্লায়েন্ট: content/thinking/tool_calls + সহনশীল পার্সার
G3
P0-1
padma.tolerant_parse (চালু)
done

P0-3
Intent-without-Action গার্ড
G2
P0-2
padma.intent_guard (চালু)
done

P0-4
Recovery Ladder
G4
P0-2, P0-3
padma.recovery (চালু)
done

P0-5
Overflow Rule + obs_cap + Observation Masking
G5
P0-1
padma.overflow_rule, padma.obs_cap (চালু)
done

P0-6
পরিষ্কার history: UI-নোটিশ বাইরে
G10
P0-1
padma.clean_history (চালু)
done

P0-7
প্রমাণভিত্তিক Trust
G9
P0-3
padma.evidence_trust (চালু)
done

P0-8
টুল-error সমৃদ্ধ করা (Win/mac/Linux OS কোড, cwd, কাছাকাছি পথ, quote)
G8
—
padma.tool_hints (চালু)
done

P1-1
Task Contract + Task Ledger (ডিস্কে, resume)
G7
P0-1
padma.ledger (চালু, শুধু হিসাব)
done

P1-2
Finish Gate
G1
P1-1, P0-3
padma.finish_gate (চালু)
done

P1-3
Scope Classifier (অভিপ্রায় + রিপোজিটরি-আকার)
G6
P1-1
padma.scope (চালু, শুধু ট্যাগ)
done

P1-4
Conductor state machine + Phase Budget
G1, G5
P1-2
padma.conductor (বন্ধ)
done

P1-5
চলমান-কাজ বাঁধাই: "ok koro / continue / পেস্ট-error"
G7
P1-1
padma.continue_bind (চালু)
done

P2-1
Model Profile + Capability Probe
G3
P0-2
padma.model_profile (চালু, শুধু পড়া)
done

P2-2
ধাপ-ধরন অনুযায়ী thinking-নীতি
G4
P2-1, N7-3
padma.think_policy (বন্ধ)
done

P2-3
ধাপ-প্রতি টুল-উপসেট
G3
P2-1, M2-1
padma.tool_subset (বন্ধ)
done

P2-4
ভেতরে ইংরেজি, বাইরে ব্যবহারকারীর ভাষা (ছোট প্রম্পটেও)
G10
N2-2, P1-1
padma.inner_english (বন্ধ)
done

P3-1
Recipe Engine (toml DAG: কোড/মডেল/গেট)
G6
P1-4
padma.recipes (বন্ধ)
done

P3-2
R-REVIEW-CODE + Findings Ledger + প্রমাণ-গেট
G1, G6
P3-1, N3-3, N4-1
padma.recipes-এর অধীনে
done

P3-3
R-REVIEW-SITE
G6
P3-2
padma.site_probe (বন্ধ)
done

P3-4
R-BUILD-PAGE
G6
P3-1, N2-3, N4-2
padma.recipes-এর অধীনে
done

P3-5
R-FIX (Agentless-ধাঁচ)
G6
P3-1, N5-1, N7-1
padma.recipes-এর অধীনে
done

P3-6
UI: Recipe-চিপ, প্রগ্রেস-কার্ড, Findings ভিউ
G7
P3-2
—
done

P4-1
NAVARATNA ইঞ্জিন Recipe-এ জোড়া (প্রম্পট-দৈর্ঘ্য শর্ত বাদ) + relay সংযোগ
G6
P3-2, M1-4
padma.recipe_engines (বন্ধ)
done

P4-2
PADMA Bench: "ছোট প্রম্পট, বড় কাজ" স্তর (≥ ১০ কাজ)
সব
P3-2, P3-4
—
done (স্ক্রিপ্টেড; লাইভ = P4-3)

P4-3
লাইভ যাচাই (Ollama + qwen3.5:9b)
সব
P4-2
—
owner-later

প্রস্তাবিত ক্রম: P0-1 → P0-2 → P0-3 → P0-5 → P0-6 → P0-8 → P0-4 → P0-7 → P1-1 → P1-5 → P1-2 → P1-3 → P1-4 → P2-1 → P2-2 → P2-3 → P2-4 → P3-1 → P3-2 → P3-6 → P3-3 → P3-4 → P3-5 → P4-1 → P4-2।
কারণ: আগে লাল test (কী ভাঙা, তা প্রমাণ), তারপর থেমে যাওয়া বন্ধ, তারপর কাজ মনে রাখা আর শেষ করা, শেষে Recipe। P0 আর P1 শেষ হলেই ২৫ অক্টোবরের মতো ঘটনা আর ঘটার কথা নয়। Recipe-গুলো তারপর মান বাড়াবে।
অবস্থার মান: todo, in-progress (ধাপ n/m), done, blocked (<কারণ>), owner-later।
মাপকাঠি, সৎ সীমা, মালিকের কাজ
সাফল্যের মূল মাপ একটাই: "ছোট প্রম্পট, বড় কাজ" bench-এ কাজ শেষ হওয়ার হার। আর টুল-ছাড়া-থামা বা overflow-এ-মৃত্যু শূন্য। সব সংখ্যা এখন লক্ষ্য, প্রমাণ নয়।
মাপ
লক্ষ্য
কীভাবে মাপা
টুল-কল ছাড়া কাজের মাঝে থামা
০
intent_guard লগ + Finish Gate
ContextOverflow-এ turn ব্যর্থ
০
overflow_rule গণনা
ফাঁকা উত্তরে কাজ থামা
০ (মেরামত বা সৎ blocked)
Recovery Ladder লগ
কাজ শেষ বা সৎ blocked
১০০% turn
Finish Gate
bench-এ কাজ সম্পূর্ণ (লোকাল 9B)
চালু > বন্ধ; লক্ষ্য ≥ ৮০%
PADMA Bench
বানানো finding (প্রমাণ মেলে না)
০ রিপোর্টে
প্রমাণ-গেট প্রত্যাখ্যান
প্রমাণ ছাড়া Trust high
০
evidence_trust
resume-এ কাজ হারানো
০
Never-Again resume_after_fail
লোকাল ধাপের অংশ
≥ ৯০% (NAVARATNA-র সাথে)
sdcd relay report
ছোট সাধারণ প্রশ্নে গতি
≤ ৫% ধীর
Speed Gate
সৎ সীমা:
• "আর কখনো থামবে না" মানে মডেল কখনো ভুল করবে না, তা নয়। মানে হলো ভুলটা কোড ধরবে, মেরামতের চেষ্টা করবে, আর না পারলে সৎভাবে জানাবে। নীরবে অর্ধেক কাজ করে "Done" দেখানো বন্ধ হবে।
• 9B-র রিভিউ গভীর লজিক-বাগ বা নিরাপত্তা-ত্রুটি মিস করতে পারে। আসল compile/test error কোড নির্ভুলভাবে ধরবে। কিন্তু "বাগ নেই" কখনো প্রমাণিত সত্য নয়।
• পুরো প্রজেক্ট রিভিউ লোকালে সময় নেবে, কারণ শত ফাইল মানে শত কল। প্রগ্রেস আর resume এই কারণেই জরুরি।
• ৮৬% GPU-তে চলা মানে প্রতিটি কল ধীর। বড় context আর পূর্ণ GPU-ফিট একসাথে পাওয়া যায় না (P19)।
• Ollama-র qwen3.5 টুল-ফরম্যাট সমস্যা Ollama-র দিক থেকেই মেটাতে হতে পারে। SDC-র সহনশীল পার্সার কাজ চালিয়ে নেবে, কিন্তু ভুলের হার মডেল-template-এর উপর নির্ভর করে।
মালিকের কাজ:
[ ] Ollama হালনাগাদ, তারপর ollama show qwen3.5:9b --modelfile দিয়ে template আর quantization দেখা
[ ] এখনই পরীক্ষার জন্য NAVARATNA সুইচ চালু করা: nr.fresh_step, nr.tool_envelope, nr.error_slice, nr.skeleton_pack, nr.git_shadow, nr.think_budget
[ ] বড় রিভিউয়ে relay.auto চালু রাখবেন কি না ঠিক করা
[ ] PADMA.md আর PADMA-PROMPTS.md SDC-র skills ফোল্ডারে রাখা (MEGHNA আর NAVARATNA-র পাশে)
[ ] P0 শেষ হলে একই দুই লাইনের প্রম্পট আবার দেওয়া: "পুরো SDC প্রজেক্ট দেখো, error রিপোর্ট করো"
[ ] লাইভ bench (P4-3), তারপর কোন সুইচ ডিফল্টে চালু হবে সেই সিদ্ধান্ত
উৎস
• Agentless: Demystifying LLM-based Software Engineering Agents — Xia et al.
• Agentless repo summary
• The Complexity Trap: Observation Masking vs LLM Summarization (NeurIPS 2025 DL4C) — JetBrains Research
• The Complexity Trap, arXiv
• ollama#14745 — qwen3.5:9b prints tool call instead of executing
• ollama#14493 — Qwen 3.5 tool-call format mismatch, unclosed think tag
• zeroclaw#3079 — qwen3.5 only-thinking, empty content
• hermes-agent#46131 — reasoning models return empty content via Ollama
• Qwen3.5-9B model card
• NAVARATNA আর MEGHNA ডকের সব উৎস
নোট: GitHub issue-গুলো নির্দিষ্ট Ollama সংস্করণের; বর্তমান সংস্করণে কিছু ঠিক হয়ে থাকতে পারে। P2-1-এর Capability Probe সেটা আপনার মেশিনেই মাপবে।