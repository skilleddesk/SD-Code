# SDC Master Plan v3 — Trust Kernel + Universal Intent Engine

Sep 27, 2026 · @the owner

> নোট: এই doc `MASTER_SPEC.md` (v3.0, UI build spec) থেকে আলাদা — এটা business/product strategy ও
> roadmap doc, চারটা AI-র সুপারিশ যাচাই করে তৈরি। বর্তমান বাস্তবায়ন অবস্থা `ROADMAP-v4.md`-এ (0.11.9
> পর্যন্ত)। এই doc-এর 0.12+ ধাপগুলো `ROADMAP-v4.md`-এর পরের ধাপ হিসেবে ধরতে হবে।

## সারকথা

সিদ্ধান্ত: SDC হবে "AI coding-এর Trust Layer — real server-এর জন্য, যেকোনো ভাষায়"। চারটা AI-র সুপারিশ
দুবার পড়ে যাচাই করেছি: প্রায় ৭০% সত্যিই app-কে বড় করবে এবং plan-এ যোগ হয়েছে; কিছু সুপারিশ বদলে নিরাপদ করা
হয়েছে (যেমন রাতে AI নিজে production-এ patch চালাবে না, শুধু rollback করবে ও fix তৈরি রাখবে); আর কিছু
সংখ্যা যাচাই করা যায়নি, তাই plan-এ "তথ্য" হিসেবে রাখা হয়নি।

v3-এর ৫টা বড় বদল:

1. **Trust Kernel** — Time Machine, Verify, Guard rails, audit log আর rollback আলাদা feature নয়, একটা
   ভিত্তি-স্তর; প্রতিটা AI কাজ এর মধ্য দিয়ে যাবে।
2. **Universal Intent Engine** — যেকোনো ভাষা, আঞ্চলিক ভাষা (যেমন সিলেটি, চাটগাঁইয়া, ভোজপুরি, মিসরীয়
   Arabic), মিশ্র ভাষা ও romanized লেখা বা ভয়েস বুঝে একটা structured Task Spec বানায়, user-এর ভাষায়
   নিশ্চিত করে, তারপর প্রতিটা engine-এর জন্য আলাদা "perfect prompt" বানিয়ে কাজ শুরু করে।
3. **Pipeline ৬ ধাপে**: বোঝা → তৈরি → যাচাই → চালু → রক্ষা → প্রমাণ।
4. **Safe Deploy আগে আনা হলো** — 0.14-এ (আগে ছিল 1.0-এ), কারণ চারটা AI-ই একমত যে agency-র আসল কাজ
   deploy ও rollback।
5. **2.0 "Autonomous Agency"** — Night Guardian, Takeover X-ray (পুরনো server scan), shadow DB
   migration, multiverse Time Machine।

এই doc-এ আরো দুটো tab ছিল উৎসে: Claude Code-এর জন্য সম্পূর্ণ prompt (design নির্দেশনাসহ), আর plan-টা
repo ও server-এ রাখার command — সেগুলো এখনো পাওয়া যায়নি, পরে যোগ করা হবে।

## ৪টা AI-র সুপারিশ: কোনটা সত্যিই app-কে বড় করবে

চারটা AI মূল দিকে একমত — trust, deploy-safety, cost আর proof — এবং এই ঐকমত্য নিজেই একটা শক্ত signal।
তবে কয়েকটা সুপারিশ SDC-র নিজের নীতি (P4 never lies, P5 undo) ভাঙত বা যাচাইহীন সংখ্যার উপর দাঁড়িয়ে ছিল,
তাই সেগুলো বদলে বা বাদ দিয়েছি।

নাম সংক্ষেপ: AI-১ = "Server Ops Cockpit" বিশ্লেষণ, AI-২ = "Autonomous Agency" বিশ্লেষণ, AI-৩ = "Trust
Fabric" বিশ্লেষণ, AI-৪ = "Research-verified" বিশ্লেষণ।

| সুপারিশ | কে বলেছে | রায় | কারণ / কীভাবে নেওয়া হলো |
|---|---|---|---|
| Trust-কে feature নয়, architecture বানানো (Trust Kernel) | AI-৩ | গ্রহণ | SDC-র সবচেয়ে বড় moat; সব feature এক ভিত্তিতে বসে |
| Pipeline-এ "যাচাই" ও "প্রমাণ" আলাদা ধাপ | AI-৩ | গ্রহণ, বাড়ানো | শুরুতে "বোঝা" ধাপও যোগ করলাম → ৬ ধাপ |
| Verify-তে SAST, secret, dependency scan বাধ্যতামূলক | AI-২, AI-৩ | গ্রহণ | Sonar 2026: ৯৬% AI code পুরো বিশ্বাস করে না, মাত্র ৪৮% সবসময় যাচাই করে — যাচাই automatic হতে হবে |
| Safe Deploy আগে আনা | চারটাই | গ্রহণ | 0.14-এ; Appaloft-এর মতো AI-deploy tool বাজারে আসছে, দেরি করলে জায়গা হারাবে |
| Cost dashboard + budget + model router | চারটাই | গ্রহণ | Gartner (জুন ২০২৬): ২০২৮ নাগাদ AI coding খরচ গড় developer বেতন ছাড়াবে |
| Guard rails, protected path (wp-config.php, .pem, backup) | AI-১ | গ্রহণ | path তালিকা default policy-তে যোগ হলো |
| Trust Score প্রতি turn + Agency Ops Score প্রতি site | AI-১, AI-৪ | গ্রহণ, একত্র | একটা score engine: turn-এর ঝুঁকি ও site-এর স্বাস্থ্য |
| Checkpoint label ("Before deploy"), single-file restore | AI-১ | গ্রহণ | Time Machine-কে product-এর hero বানানো |
| Site Health Watch (SSL, disk, backup-এর বয়স) | AI-১ | গ্রহণ | backup age ও disk যোগ হলো |
| Agency Mode: role, approval, client read-only | AI-১, AI-২ | গ্রহণ | 1.0-এ |
| Client approval-এর জন্য micro-staging link | AI-২ | গ্রহণ | একই VPS-এ অস্থায়ী subdomain/port; client দেখে approve করলে deploy |
| Takeover mode: পুরনো server scan করে map ও doc | AI-২ | গ্রহণ | নাম "Takeover X-ray"; শুধু read-only scan, 2.0-এ |
| Shadow DB migration (নমুনা data-য় আগে চালানো) | AI-২ | গ্রহণ, পরে | 2.0; বড় কাজ, কিন্তু DB migration-ই সবচেয়ে বড় ভয় |
| রাতে AI নিজে rollback ও patch করবে | AI-২ | সংশোধিত | নিজে শুধু "শেষ ভালো version"-এ rollback (policy-তে চালু থাকলে); fix তৈরি থাকবে কিন্তু production-এ যাবে approval-এর পরে — নইলে P5 ও client-এর দায় ভাঙে |
| AI swarm: ৪ agent-এর কথোপকথন দেখানো | AI-২ | সংশোধিত | chat দেখানো খরচ বাড়ায় ও "theater"; বদলে role-ভিত্তিক pipeline ধাপ (Plan → Build → SecReview → SRE), ভিন্ন vendor-এর model |
| সব project থেকে শেখা RAG memory | AI-২ | সংশোধিত | এক client-এর code অন্য client-এ ফাঁস হওয়ার ঝুঁকি; বদলে প্রতি project-এর memory + owner-এর লেখা "Agency Style Guide" |
| প্রতি agent session-এর আলাদা identity, অল্প সময়ের credential | AI-৩ | গ্রহণ, পরে | 1.0-এ team mode-এর সাথে; এখন session-ভিত্তিক audit identity |
| Pricing: active site প্রতি | AI-১ | গ্রহণ (hypothesis) | agency site-এ ভাবে; pilot দিয়ে যাচাই |
| Bangladesh-এ "নিরাপদ" নয়, "সহজ ও আনন্দের" message | AI-৩ | পরীক্ষা করে দেখা | একটা গবেষণার দাবি, যাচাই করিনি; pilot-এ দুই message A/B test |
| Local Ollama দিয়ে ৪০–৬০% খরচ বাঁচবে; Devin Fusion ৩৫% | AI-২, AI-৩ | সংখ্যা বাদ | feature রাখলাম (hybrid router), সংখ্যা যাচাই হয়নি — SDC নিজে মেপে দেখাবে |
| $29–49 / $99 দাম | AI-২ | বাদ (এখন) | অনুমান; ৫–১০ agency-র pilot-এর পরে ঠিক হবে |
| "Claude 3.5 Sonnet" দিয়ে coder | AI-২ | বাদ | পুরনো model নাম; SDC model-নিরপেক্ষ থাকবে |

আমার আগের একটা ভুল সংশোধন: v1-এ লিখেছিলাম Gartner বলেছে "৪০% প্রতিষ্ঠানের খরচ বাজেটের দ্বিগুণ হবে" — এটা
আমি একটা aggregator site থেকে নিয়েছিলাম, AI-৪ ঠিকই বলেছে এটা মূল উৎসে পাওয়া যায় না। আসল ও যাচাই করা
Gartner তথ্য দুটো: ৪০%-এর বেশি agentic AI project ২০২৭-এর মধ্যে বাতিল হবে — কারণ বাড়তি খরচ, অস্পষ্ট
মূল্য ও দুর্বল ঝুঁকি-নিয়ন্ত্রণ; আর ২০২৮ নাগাদ AI coding খরচ গড় developer বেতন ছাড়াবে। দুটোই SDC-র
কৌশলকে আরো শক্ত করে।

সবচেয়ে গুরুত্বপূর্ণ নতুন তথ্য (global বাজারের জন্য): Gartner-এর analyst বলেছেন বর্তমান token খরচ
ইতিমধ্যে India-র বেশিরভাগ বেতনের চেয়ে বেশি (The Register)। মানে Asia, Africa, Latin America-র
developer-দের কাছে খরচ নিয়ন্ত্রণই SDC-র সবচেয়ে বড় বিক্রয় যুক্তি।

## নতুন ভিশন: SDC = AI coding-এর Trust Layer

Positioning: "SDC হলো developer ও agency-র জন্য AI cockpit — যেকোনো ভাষায় বলুন, যেকোনো AI model দিয়ে
কাজ হবে, অন্য AI যাচাই করবে, নিরাপদে server-এ যাবে, ভাঙলে এক ক্লিকে ফেরাবে, আর খরচসহ প্রমাণ দেবে।"

ছোট slogan: **"Prompt the work. Watch the proof. Protect the server."** (AI-১-এর প্রস্তাব, রাখা হলো)

উপরের ছয়টা ধাপের (বোঝা → তৈরি → যাচাই → চালু → রক্ষা → প্রমাণ) প্রতিটা নিচের kernel ব্যবহার করে; কোনো
ধাপ kernel এড়িয়ে যেতে পারে না — এটাই SDC-কে "feature-এর তালিকা" থেকে "বিশ্বাসের ব্যবস্থা"-য় পরিণত করে।

**Kernel-এর ছয়টা অংশ:**

- **Audit ledger** — প্রতিটা কাজের অপরিবর্তনীয় লেখা: কোন session, model, prompt, command, diff, test,
  review, খরচ, approval; hash-chain করা, তাই পরে বদলালে ধরা পড়ে।
- **Policy engine** — policy-as-code (`.sdc/policy.toml`): নিষিদ্ধ path, সবসময় অনুমতি চাওয়া command,
  production-এ Auto mode বন্ধ, এক turn-এ সর্বোচ্চ কত file বদলানো যাবে (blast radius limit — নতুন যোগ)।
- **Verify engine** — project-এর test/build/lint, SAST, secret ও dependency scan, আর ভিন্ন vendor-এর
  AI review; ফল সবসময় structured (PASS / FAIL / NO_CHECKS / UNPROVEN)।
- **Time Machine** — checkpoint, label, single-file বা পুরো restore, journal করা atomic restore, engine
  session-ও পিছানো।
- **Cost governor** — প্রতি turn/chat/project/site-এর খরচ, আগাম আনুমান, budget cap, runaway loop ধরা,
  model router।
- **Kill switch** — একটা keyboard shortcut (যেমন Ctrl+Shift+.) চাপলে সব agent ও remote command সঙ্গে
  সঙ্গে থামে ও অবস্থা checkpoint-এ সংরক্ষিত হয় (নতুন যোগ, P7)।

**কী SDC হবে না** (AI-১-এর তালিকা, গ্রহণ): Cursor-এর মতো autocomplete IDE, Lovable/Bolt-এর মতো no-code
builder, শুধু terminal wrapper, বা backup/rollback ছাড়া production automation।

## Universal Intent Engine: যেকোনো ভাষা ও আঞ্চলিক ভাষা থেকে perfect prompt

এটাই must-have feature: user যে ভাষায়, যে আঞ্চলিক রূপে, যে লিপিতে বা মিশিয়ে লিখুক বা বলুক — SDC প্রথমে
বোঝে, নিশ্চিত করে, তারপর engine-এর জন্য নিখুঁত prompt বানিয়ে কাজ শুরু করে। কোনো বড় competitor এটা করে না।

উদাহরণ (একই কাজ, ভিন্ন ভাষা): Banglish "ei site er contact form ta kaj kortese na, thik kore dao";
সিলেটি "ই সাইটর contact form খান কাম করর না, বাইক্কা কইরা দাও"; Hinglish "is site ka contact form kaam
nahi kar raha, fix karo"; Arabizi "el contact form mesh sha8al, sale7o"; Spanish "el formulario de
contacto no funciona, arréglalo" — সব কটাই একই Task Spec দেবে: target = বাঁধা site, সমস্যা = contact
form, শর্ত = form submit হলে email যায় ও সফলতার বার্তা দেখায়।

**যা যা থাকবে:**

- ভাষার কোনো সীমাবদ্ধ তালিকা নেই — লিপি (Bengali, Devanagari, Arabic, CJK, Latin …), ভাষা, আঞ্চলিক রূপ
  (সিলেটি, চাটগাঁইয়া, নোয়াখালী, ভোজপুরি, মিসরীয়/উপসাগরীয় Arabic, Swiss German …), romanized রূপ
  (Banglish, Hinglish, Arabizi, Romanized Urdu) ও মিশ্র ভাষা (Taglish, Spanglish) — সব সমর্থিত।
- Confidence প্রতি field-এ — target, লক্ষ্য, শর্ত প্রতিটার আলাদা নিশ্চয়তা; কম হলে সর্বোচ্চ ২টা প্রশ্ন,
  user-এর নিজের ভাষায়। কম-resource ভাষায় back-translation দেখাবে: "আমি এটা বুঝেছি — ঠিক?" (P4)।
- Reply নিয়ম — user-এর ভাষায় উত্তর; আঞ্চলিক রূপে উত্তর দেবে নাকি প্রমিত রূপে, সেটা user Settings-এ
  বেছে নেবে; code, comment, commit, branch name থাকবে project-এর নিয়মে (default English)।
- Glossary শেখা — user যখন card সংশোধন করে ("ghor মানে page"), সেটা project/agency glossary-তে যায়;
  পরের বার আর জিজ্ঞেস করে না।
- Voice — local speech-to-text (অনলাইনেও), তারপর একই pipeline।
- Prompt Compiler — একই Task Spec থেকে Claude Code, Codex, Gemini ও SDC Agent-এর জন্য আলাদা template;
  সাথে যোগ হয় project-এর rules file, repo map, policy, glossary, acceptance criteria ও "কী করবে না"।
  পুরো compiled prompt user চাইলে দেখতে পারবে (P4)।
- Contract থেকে test আগে (নতুন যোগ) — acceptance শর্ত থেকে সম্ভব হলে আগে test লেখা, তারপর code; Verify
  সেই test দিয়েই pass/fail বলে।
- মাপা — প্রতি release-এ golden set: অন্তত ৩০টা ভাষা/আঞ্চলিক রূপে একই কাজ; মাপা হবে Task Spec মিলেছে
  কি না, কতবার প্রশ্ন করতে হলো, আর user কতবার card সংশোধন করল।

## সম্পূর্ণ feature catalog

সব AI-র গ্রহণযোগ্য সুপারিশ, আগের v1/v2 plan আর নতুন যোগ — এক জায়গায়, module ধরে। "কখন" কলামটা নিচের
roadmap-এর সাথে মেলে।

| Module | যা থাকবে | কখন | Size |
|---|---|---|---|
| Release safety | installed-app E2E CI, live provider test, beta/stable channel, update rollback, CLI self-check, opt-in crash report | 0.12–0.13 | M |
| Time Machine | TM-1…TM-8 fix, journal করা atomic restore, engine session rewind, checkpoint label, single-file restore, before/after তুলনা, irreversible চিহ্ন | 0.12 | M |
| Universal Intent Engine | যেকোনো ভাষা/dialect/লিপি, Task Spec, confidence, Intent Contract card, Prompt Compiler, glossary শেখা | v1: 0.12, পূর্ণ: 0.15 | M–L |
| Trust Kernel ভিত্তি | audit ledger (hash-chain), policy-as-code, protected path, dangerous command approval, production-এ Auto বন্ধ, blast radius limit, kill switch, secret scanner | 0.12 | M |
| Verify Layer | VR-1…VR-8 fix, SAST, secret ও dependency scan, cross-vendor review, contract থেকে test, সীমিত auto-fix loop (সর্বোচ্চ ২ বার), Preview screenshot review | 0.13 | M |
| Cost governor | turn/chat/project/site খরচ, আগাম আনুমানিক খরচ, budget cap, runaway loop ধরা, hybrid router (local Ollama সহজ কাজে), "আজ কত বাঁচল" (মাপা, অনুমান নয়) | 0.13 | M |
| Safe Deploy | preflight, backup (file + DB), deploy steps, HTTP/SSL/text health check, auto-rollback, one-click rollback, "AI দিয়ে fix" checkpoint সহ, Undo preview | 0.14 | L |
| Health Watch | uptime, response time, SSL মেয়াদ, disk, backup-এর বয়স, error log, শেষ deploy-এর অবস্থা, alert user-এর ভাষায় | 0.14 | M |
| Scores | প্রতি turn Trust/Risk Score; প্রতি site Agency Ops Score (backup, health, খরচ, violation) | 0.14–1.0 | S |
| Proof | Proof Pack (JSON + user-এর ভাষায় HTML), client report আগে/পরে screenshot সহ | 0.15 | S–M |
| প্রথম অভিজ্ঞতা | signed installer, ১০ মিনিটের wizard (local / VPS / নতুন app / WordPress site), Environment doctor, sample project, rollback demo | 0.15 | M |
| Global UI | ICU i18n (প্রথমে ১০টা ভাষা), RTL, IME-safe input, local voice input, language settings | 0.15 | M |
| Agency Mode | Owner / Developer / Reviewer / Client read-only, production-এর আগে approval, micro-staging link-এ client approve, fleet view, playbooks, per-site খরচ, headless sdcd run, session identity | 1.0 | L |
| Autonomous Agency | Night Guardian (নিজে শুধু rollback, fix approval-এর অপেক্ষায়), Takeover X-ray (read-only scan → map + doc), shadow DB migration, role-pipeline (Plan → Build → SecReview → SRE), Agency Style Guide | 2.0 | L |
| উন্নত Time Machine | Multiverse timeline, parallel worktree কাজ, Duel ও Session Bridge এক ধারণায় | 2.0 | L |
| অন্যান্য | Privacy mode (sensitive repo-তে শুধু local model), low-bandwidth mode, MCP in SDC Agent, long-task memory (rules file, repo map, plan যা restart-এও টেকে), mobile read-only status | 1.x–2.0 | M |

ইচ্ছা করে বাদ: LSP/autocomplete, no-code app builder, VPS-এ বাধ্যতামূলক daemon, দাম ঠিক করা
(pilot-এর আগে)।

## Roadmap v3: 0.12 থেকে 2.0

ক্রমের যুক্তি: বিশ্বাস ছাড়া deploy বিপজ্জনক, আর deploy ছাড়া agency-র কাছে বিক্রি কঠিন — তাই Safe Deploy
(0.14) এখন বাইরের customer launch-এর (0.15) আগে। উপরের feature catalog-এর "কখন" কলামই gate; সংখ্যাগুলো
প্রস্তাবিত লক্ষ্য, বদলানো যায়।

সমান্তরালে এখনই শুরু করুন (কোনো release-এর অপেক্ষা ছাড়া): code signing certificate ও Apple Developer
account-এর আবেদন, আর ৫–১০টা agency (বাংলাদেশ ও বাইরের দেশ মিলিয়ে) pilot-এর জন্য খোঁজা।

## UI/UX plan: নতুন screen, পুরনো design-এর সাথে এক সুরে

নতুন ১২টা screen লাগবে, কিন্তু কোনোটাই নতুন design ভাষা আনবে না: Claude Code প্রথমে বর্তমান app থেকে
design system বের করে লিখবে (রং, font, spacing, component, icon, animation), তারপর শুধু সেগুলো দিয়েই
নতুন screen বানাবে।

| Screen | কাজ | মূল উপাদান | কখন |
|---|---|---|---|
| Intent Contract card | ভুল বোঝা শুরুতে ধরা | ধরা ভাষা/dialect chip, ৩–৮টা টিক-দেওয়া শর্ত, confidence, "compiled prompt দেখুন", সর্বোচ্চ ২ প্রশ্ন | 0.12 |
| Policy / Permission modal | বিপজ্জনক কাজে থামা | সরল বাক্যে কারণ, কোন policy নিয়ম, Allow once / Always / Deny, keyboard shortcut | 0.12 |
| Kill switch | সব থামানো | সবসময় দৃশ্যমান ছোট বোতাম + shortcut, থামার পর অবস্থা বার্তা | 0.12 |
| Timeline (Time Machine) | যেকোনো মুহূর্তে ফেরা | checkpoint label, before/after diff, single-file restore, irreversible চিহ্ন, branch | 0.12 |
| Proof Panel | প্রতি turn-এর প্রমাণ | changed file, command, test, SAST, reviewer verdict, Trust Score, rollback point | 0.13 |
| Cost meter + budget | খরচ নিয়ন্ত্রণ | live meter, turn/chat/project খরচ, "আনুমানিক" লেবেল, cap slider, সস্তা model পরামর্শ | 0.13 |
| Deploy pipeline view | নিরাপদ deploy | ধাপের progress (preflight → backup → deploy → health), প্রতিটার log, rollback বোতাম | 0.14 |
| Health Watch dashboard | site-এর স্বাস্থ্য | site কার্ড: uptime, SSL দিন, disk %, backup বয়স, শেষ deploy; Agency Ops Score | 0.14 |
| Onboarding wizard | ১০ মিনিটে প্রথম সাফল্য | লক্ষ্য বাছাই, engine/key, doctor, sample task, rollback demo | 0.15 |
| Language settings | ভাষার নিয়ন্ত্রণ | UI ভাষা, reply ভাষা/dialect, code ভাষা, voice, glossary editor | 0.15 |
| Client approval page | client-এর সম্মতি | staging link, আগে/পরে screenshot, client-এর ভাষায় সারাংশ, Approve / প্রশ্ন | 1.0 |
| Takeover X-ray map | পুরনো server বোঝা | service, site, DB, cron, dependency-এর ছবি; ঝুঁকির তালিকা | 2.0 |

**Design-এর বাধ্যতামূলক নিয়ম:**

- শুধু বর্তমান Tailwind token, রং ও component; নতুন token লাগলে আগে design system file-এ যোগ করে কারণ
  লেখা।
- প্রতিটা screen-এর ৫টা অবস্থা: loading, empty, success, error (সরল বাক্যে), degraded/offline (P6)।
- সব কাজ keyboard-এ, শর্টকাট palette-এ দেখা যায় (P7); axe-core ০ violation।
- সব লেখা i18n key দিয়ে; লম্বা ভাষার জন্য ৪০% বেশি জায়গা; RTL-এ mirror layout, কিন্তু code/terminal LTR।
- রং দিয়ে একা অর্থ নয় — Trust Score, status-এর সাথে icon ও লেখা থাকবে।
- প্রতিটা নতুন screen-এর CDP screenshot light/dark ও একটা RTL ভাষায় নেওয়া হবে, পুরনো screen-এর পাশে
  রেখে মিলিয়ে দেখা হবে।

## Business ও go-to-market

বিক্রি হবে "AI chat app" হিসেবে নয়, "client project-এর নিরাপত্তা ও প্রমাণ" হিসেবে; দামের একক হবে
active site (hypothesis), আর দাম ঠিক হবে pilot-এর পরে।

| Tier (hypothesis) | কী পাবে | কার জন্য |
|---|---|---|
| Free | local project, নিজের key/subscription, Time Machine, Intent Engine, basic Verify | একা developer, শিক্ষার্থী |
| Pro | VPS, পূর্ণ Verify Layer, cost governor, guard rails, Safe Deploy (কয়েকটা site) | freelancer |
| Agency | site প্রতি দাম, Health Watch, Proof Pack/client report, role, approval, fleet, playbooks | ছোট agency |

**Pilot plan:** ৫–১০টা agency — কয়েকটা বাংলাদেশ থেকে (beachhead), বাকিগুলো অন্তত আরো ২টা ভিন্ন ভাষার
দেশ থেকে; প্রত্যেকে ১টা আসল কিন্তু কম-ঝুঁকির site-এ ৩০ দিন। মাপা হবে: প্রথম সফল turn-এর সময়, rollback
কতবার কাজে লাগল, মাসিক খরচ, কোন feature-এর জন্য টাকা দিতে রাজি।

**Message (A/B test করা হবে):** (ক) নিরাপত্তা: "AI can help your server, but it cannot destroy it."
(খ) সহজতা: "আপনার ভাষায় বলুন, কাজ হয়ে যাবে — প্রমাণসহ।" (গ) খরচ: "No surprise AI bills" — Gartner-এর
খরচ-সতর্কতার পরে non-US বাজারে সবচেয়ে শক্ত।

**Competitor নজরে রাখুন:** Appaloft-এর মতো নতুন "AI দিয়ে নিজের server-এ deploy" tool (Product Hunt) আর
vibe-deploy-এর মতো MCP deploy tool (GitHub) দেখাচ্ছে বাজারের এই কোণটা গরম হচ্ছে — SDC-র পার্থক্য হবে
Trust Kernel + যেকোনো ভাষা + Time Machine একসাথে।

## ঝুঁকি ও P1–P7 check

নতুন feature-গুলোর মধ্যে তিনটা নীতির সাথে সবচেয়ে বেশি ঘষা খায় — Night Guardian (P5), Intent Engine (P4)
আর Health Watch (P3) — তাই এগুলোর নিয়ম plan-এই বাঁধা হলো।

| ঝুঁকি / নীতি | কোথায় | নিয়ম |
|---|---|---|
| P5 undo | Night Guardian, Safe Deploy, DB migration | নিজে শুধু "শেষ ভালো version"-এ ফেরা; নতুন code production-এ কখনো approval ছাড়া নয়; DB migration-এর আগে dump বাধ্যতামূলক |
| P4 never lies | Intent Engine, cost, router | কম confidence = প্রশ্ন বা back-translation; খরচ "আনুমানিক" লেবেল; "টাকা বাঁচল" শুধু মাপা সংখ্যা |
| P3 structured | Health Watch, log scan | HTTP status, exit code, SAST JSON ব্যবহার; log-এ regex দিয়ে "সফল" ঘোষণা নয় |
| P1 | Takeover X-ray, Health Watch | ssh exec দিয়ে read-only scan; server-এ কিছু install নয় |
| P7 keyboard | সব নতুন screen, kill switch | প্রতিটা কাজ shortcut-এ, axe-core ০ violation |
| Client data privacy | Agency Style Guide, memory | এক client-এর code অন্য project-এর context-এ যাবে না |
| এক জনের উপর নির্ভরতা | release | E2E CI, লিখিত checklist, docs/plan/ ফাইল যাতে যেকোনো AI কাজ চালাতে পারে |
| Provider নিয়ম বদল | engine | সবসময় ≥২ API route + Ollama fallback |
| দায় | deploy feature | backup বাধ্যতামূলক, license-এ দায়সীমা (আইনজীবীর পরামর্শ নিন) |

## Sources

- Gartner — 40%-এর বেশি agentic AI project ২০২৭-এর মধ্যে বাতিল হবে (জুন ২০২৫)
- Gartner — ২০২৮ নাগাদ AI coding খরচ গড় developer বেতন ছাড়াবে (জুন ২০২৬)
- The Register — Gartner analyst: token খরচ ইতিমধ্যে India-র বেশিরভাগ বেতনের বেশি
- DevOps.com — ২৩% tech leader developer প্রতি মাসে $200–500 token-এ খরচ করে
- Sonar — 2026 State of Code survey: ৯৬% পুরো বিশ্বাস করে না, ৪৮% সবসময় যাচাই করে
- Stack Overflow 2025 Developer Survey
- GitHub Octoverse 2025
- Appaloft — Product Hunt
- vibe-deploy — GitHub
- আগের market ও competitor সূত্রগুলো পুরনো doc-এর Sources অংশে আছে।

**যাচাই করা হয়নি** (তাই plan-এ তথ্য হিসেবে নেই): Ollama-তে ৪০–৬০% খরচ কমা, Devin Fusion-এ ৩৫% কমা,
Claude Code-এর $8B ও Anthropic-এর $47B হিসাব, SSHepherd/PanelAlpha/Omnigent-এর বিবরণ, বাংলাদেশে
"hedonic motivation" গবেষণা, ৮৪% বাংলাদেশি freelancer-এর AI ব্যবহার, $29–99 দাম।
