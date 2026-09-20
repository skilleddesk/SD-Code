Skilleddesk Code (SDC) — মাস্টার বিল্ড স্পেসিফিকেশন v3.0
Sep 19, 2026 · @Mehedi Hasan

০. এই সংস্করণে কী বদলালো (v2.0 → v3.0)
v2.0 ছিল সিস্টেম-কেন্দ্রিক; v3.0 UI-প্রমাণিত (UI-verified)। এই সংস্করণের সব কিছু এমনভাবে লেখা হয়েছে যাতে §7–§9-এর প্রতিটি লাইন সরাসরি কোডে রূপান্তর করা যায়, আর সেই কোড বর্তমান HTML প্রোটোটাইপের সাথে এক-এক মিলে যায়।

পরিবর্তনের ধরন	বিস্তারিত
Design tokens সম্পূর্ণ নতুন	§8.1 আগের ভুল রঙ (যেমন #0D0F12, #4C8DFF) বাদ দিয়ে হুবহু CSS variable (যেমন #0A0B0F, #5B9CFF) ব্যবহার করা হয়েছে। Light theme-ও UI-এর সাথে সমন্বিত।
Layout spec নতুন করে লেখা	§7.1-এ topbar-এর ৮টি বাটন, sidebar-এর host grouping, tab strip-এর ২টি অ্যাকশন, right panel-এর ৬টি ট্যাব, status bar-এর ৭টি সেগমেন্ট — সব হুবহু।
Sidebar-এর সংগঠন বদলেছে	v2.0-এ ছিল "Sessions / Tags / Files / Prompts / Analytics" ফ্ল্যাট লিস্ট। v3.0-এ Host-grouped sessions (Local, prod-1, staging-2 …) — যেটা UI-তে আছে। Tags/Files/Prompts/Analytics sidebar-এর পাশে নয়, বরং Command palette, Search overlay, Provider hub, Right panel-এ চলে গেছে।
Provider Hub পূর্ণাঙ্গ সেকশন	§9.10 — ৭টি ন্যাভ আইটেম, provider card, API-key form, subscription OAuth, Ollama local, custom endpoint, model registry, environment doctor।
Settings ৭টি ট্যাব	§9.11 — General, Appearance, Keymap, Safety, Notifications, Backup, About — UI-এর সাথে হুবহু।
Model dropdown	§9.3 — Tier / Engine / Model তিনটি গ্রুপ, প্রতিটির আইকন ও লেবেল UI-এর সাথে মিলে।
Turn rendering	§7.4 — collapsed summary, thinking block, tool cards (Read / Edit / Run), error card, turn footer (feedback), সব UI-এর আইডেন্টিকাল।
Right panel-এর ৬টি ট্যাব	§7.7–§7.12 — Preview, Console, Time Machine, Duel, Verify, Analytics, প্রতিটি তার নিজস্ব wireframe সহ।
Empty states	§7.13 — প্রতিটি স্ক্রিনের empty state-এর হুবহু ইংরেজি টেক্সট।
Modals	§9.10–§9.14 — Add Host, Permission, Settings, Search, Palette-এর নির্দিষ্ট টেক্সট, আইকন ও কী।
Module count	৭৮ → ৯২ (১০টি UI মডিউল + ৪টি সংশোধিত)।
নীতি সংখ্যা	৭ (অপরিবর্তিত)
সব §1–§6, §11–§22 আগের কাঠামোই রাখে, তবে যেখানে UI-এর সাথে সংঘর্ষ ছিল সেখানে সংশোধন করা হয়েছে (যেমন §3.1-এ app RAM ৯০ MB, §13.9-এ Simple মোডে মডেলের নাম লুকানো, ইত্যাদি)।

১. v2.0-র ধারাবাহিকতা (কী বদলায়নি)
v2.0-এর তিনটি রিভিউ থেকে গৃহীত ৩২টি সুপারিশ, ৫টি সংশোধিত, ২টি বাতিল, এবং ১২টি নিজের যোগ করা গ্যাপ — সব সিদ্ধান্ত এই সংস্করণেও বহাল। বিস্তারিত জানতে মূল v2.0 ডকুমেন্ট দেখুন বা §22.9-এর সিদ্ধান্ত লগ। এই ডকুমেন্টে সেই সিদ্ধান্তগুলোর বাস্তবায়ন দেখানো হয়েছে।

নীতিগুলো (§2.4) অপরিবর্তিত:

P1 — ইঞ্জিন কোডের পাশে, UI তোমার কাছে

P2 — SDC AI বানায় না, চালায়

P3 — structured stream only, terminal regex নিষিদ্ধ

P4 — SDC কখনো মিথ্যা বলে না

P5 — সবকিছু পরমাণুভাবে ফেরানো যায়, redo সহ

P6 — দুর্বল হবে, মৃত নয়

P7 — প্রতিটি কাজ কীবোর্ড দিয়ে

২. প্রোডাক্ট সংজ্ঞা ও নীতি
২.১ SDC কী
Skilleddesk Code (SDC) একটি হালকা, disconnect-proof AI development cockpit। অফিসিয়াল coding CLI এবং সরাসরি model API — দুটোকেই পরস্পর-বিনিময়যোগ্য ইঞ্জিন হিসেবে চালায়, তার উপরে বসায় নিয়ন্ত্রণ, নিরাপত্তা ও বোধগম্যতা।

এক বাক্যে: ইঞ্জিন চলে যেখানে কোড, UI থাকে যেখানে তুমি, প্রতিটি ধাপ দৃশ্যমান, সবকিছু ফেরানো যায়, কিছুই লুকানো নয়।

২.২ SDC যা নয়
যা নয়	কেন
IDE	extension host নেই, debugger নেই, default LSP নেই
AI model	SDC কখনো মডেল train বা host করে না
VS Code fork	Tauri-র উপর শূন্য থেকে। Electron নেই, Monaco নেই
এক vendor-এর wrapper	প্রতিটি ইঞ্জিন pluggable, export সর্বদা আছে
২.৩ কাদের জন্য
Vibe coder / কর্মরত ডেভেলপার / Power user — এক প্রোডাক্ট, তিন গভীরতা (Simple / Pro / Auto)। UI-তে mode switch topbar-এ ডানে।

২.৪ সাতটি অপরিবর্তনীয় নীতি
(সংক্ষেপ) P1–P7 — যেমন v2.0-এ ছিল।

২.৫ পাঁচটি differentiator
Time Machine (screenshot checkpoint, file + conversation rewind + redo)

Error Translator (plain English + one-click fix)

Session Bridge (mid-task engine switch)

Preview → Agent Loop (console error → agent context)

Duel Mode (same prompt, two engines, side-by-side)

২.৬ নামকরণ
(অপরিবর্তিত — §2.6 v2.0)

২.৭ ভাষা
অ্যাপ ইংরেজি, এই ডকুমেন্ট বাংলা। সব UI string একটিমাত্র app/src/strings.ts-এ থাকবে, localisation-এর দরজা খোলা।

৩. সিস্টেম আর্কিটেকচার
৩.১ তিনটি প্রসেস, দুটি মোড
প্রসেস	সংখ্যা	কোথায়	কী নিয়ন্ত্রণ করে
SDC App	১ প্রতি ইউজার	local machine	rendering, input, keychain, preview WebView
sdcd	১ প্রতি হোস্ট	local ও/বা প্রতিটি VPS	session, engine, file, PTY, git, event log
Engine process	১ প্রতি সক্রিয় সেশনে	sdcd-র হোস্টে	আসল agent loop
লোকাল ও রিমোট মোড একই binary, একই প্রোটোকল, শুধু transport আলাদা।

৩.২ পূর্ণ চিত্র
v2.0-এর ASCII diagram অপরিবর্তিত, তবে UI স্তরে যোগ হলো:

text
╔══════════════════ LOCAL MACHINE ══════════════════════════════╗
║  SDC App — Tauri 2                              ~90 MB RAM     ║
║ ┌──────────────────────────────────────────────────────────┐  ║
║ │ React + TypeScript + Tailwind + Lucide icons             │  ║
║ │  Topbar: brand · host · mode · palette · 6 icon-btns     │  ║
║ │  Sidebar: New chat · filter · host groups · sessions      │  ║
║ │  Main: degraded-banner · tabstrip · turn stream · prompt │  ║
║ │  Right: Preview · Console · TM · Duel · Verify · Analytics║ │  ║
║ │  Status: host · engine · model · providers · chats · conn║  ║
║ └──────────────────────────────────────────────────────────┘  ║
║  Overlays: palette · search · provider hub · settings ·       ║
║            add host · permission · toast                       ║
╚═════════════════════════╤═════════════════════════════════════╝
৩.৩–৩.৬
v2.0 অপরিবর্তিত (event log একমাত্র সত্য, engine কোডের পাশে, credential trade-off, ইচ্ছাকৃত অনুপস্থিতি)।

৪. টেক স্ট্যাক ও রিপোজিটরি
৪.১ লক করা স্ট্যাক
v2.0-এর টেবিল অপরিবর্তিত। UI-স্তরের পরিশিষ্ট:

স্তর	পছন্দ	বাতিল
Icon set	Lucide React (16px default, stroke 1.5)	Heroicons — অসম্পূর্ণ
Design tokens	CSS variables + Tailwind config (make gen)	Styled-components — runtime খরচ
Font	Inter + JetBrains Mono	System UI-only — mono দরকার
৪.২–৪.৬
v2.0 অপরিবর্তিত। তবে design/tokens.json-এ v3.0-এর মান বসবে (নিচে §8.1)।

৫. SDCP প্রোটোকল
৫.১ Envelope, ৫.২ Methods, ৫.৩ Notifications, ৫.৪ Event ক্যাটালগ, ৫.৫ Capability, ৫.৬ নিয়ম, ৫.৭ Migration, ৫.৮ HTTP API, ৫.৯ CLI, ৫.১০ Schema-first
v2.0 অপরিবর্তিত। তবে §5.4-এ নতুন দুটি event যোগ হলো UI-এর সমর্থনে:

Type	data	UI-তে
host_status	host_id, status, latency_ms?	host-header dot-এর রঙ, degraded banner
provider_status	provider_id, state, account?, last_error?	Provider hub-এর card status, topbar plug-এর has-dot
৬. ডেটা মডেল
v2.0-এর SQLite স্কিমা অপরিবর্তিত, তবে দুটি ক্ষুদ্র যোগ:

sql
-- providers table (UI-এর plug has-dot ও Provider hub-এর জন্য)
CREATE TABLE providers (
  id TEXT PRIMARY KEY,               -- 'claude', 'openai-api', 'ollama' …
  kind TEXT NOT NULL,                -- 'subscription' | 'api-key' | 'local'
  label TEXT NOT NULL,
  logo TEXT NOT NULL,                -- CSS class suffix: claude/openai/gemini/…
  initial TEXT NOT NULL,             -- 'C','O','G','D','R','L' (fallback)
  status TEXT NOT NULL,              -- 'connected' | 'needs-auth' | 'available'
  account TEXT,                      -- masked account label
  keychain_ref TEXT,                 -- for api-key kind
  priority INTEGER DEFAULT 100,
  last_checked INTEGER
);

-- topbar plug has-dot indicator-এর জন্য সংক্ষিপ্ত view
CREATE VIEW providers_attention AS
  SELECT id, label, status FROM providers WHERE status = 'needs-auth';
অন্যান্য সব (hosts, projects, sessions, session_tags, events, session_fts, turns, checkpoints, rewind_stack, permissions, accounts, costs, prompts, turn_signature, locks, intent_wal, audit, blobs, meta) — অপরিবর্তিত।

৭. UI স্পেসিফিকেশন — স্ক্রিন, লেআউট ও wireframe
এই সেকশনটি এখন UI-প্রোটোটাইপের এক-এক মিল। যেখানে ডক ও UI-এর ভিন্নতা ছিল, UI জেতে।

৭.০ UI-এর সম্পূর্ণ কাঠামো (এক নজরে)
text
#app  (grid: 46px / 1fr / 30px)
├── .topbar
│   ├── .brand (brand-mark + brand-text)
│   ├── #activeHostBtn (host-pill)
│   ├── .spacer
│   ├── .mode-switch (Simple | Pro | Auto)
│   ├── #openPalette (cmd-btn ⌘K)
│   ├── #openProviders (icon-btn plug, has-dot)
│   ├── #themeToggle (icon-btn moon/sun)
│   ├── #toggleSidebar (icon-btn panel-left)
│   ├── #toggleRight (icon-btn panel-right)
│   └── #openSettings (icon-btn settings)
│
├── .workspace (grid: 280px / 1fr / 400px)
│   ├── .sidebar#sidebar
│   │   ├── .sidebar-actions (#newChatBtn)
│   │   ├── .sidebar-search (#sidebarSearch)
│   │   ├── .sidebar-scroll#hostsList
│   │   └── .sidebar-bottom (#addHostBtn)
│   ├── .main
│   │   ├── #degradedBanner (hidden by default)
│   │   ├── .tabstrip (scroll wrap + actions)
│   │   └── .main-content#mainContent
│   └── .rightpanel#rightpanel
│       ├── .panel-tabs (6 tabs)
│       └── .panel-content (6 views)
│
└── .statusbar
৭.১ Topbar
উচ্চতা ৪৬px, সীমানা নিচে 1px --border-subtle।

ক্রম	এলিমেন্ট	আইকন/টেক্সট	আচরণ
১	.brand-mark	"S" (gradient)	ক্লিক: About/settings
২	.brand-text	"SDC"	640px-এর নিচে লুকায়
৩	#activeHostBtn	● Local ▾	popover: host switcher, প্রতিটির status
৪	spacer	—	—
৫	.mode-switch	Simple / Pro / Auto	900px-এর নিচে লুকায়; Pro ডিফল্ট
৬	#openPalette	🔍 Search or jump to… ⌘K	1100px-এর নিচে শুধু আইকন
৭	#openProviders	🔌 (has-dot)	Provider Hub খোলে; has-dot তখনই যখন কোনো provider needs-auth
৮	#themeToggle	🌙 / ☀️	Dark ↔ Light
৯	#toggleSidebar	panel-left	.no-sidebar টগল
১০	#toggleRight	panel-right	.show-right ↔ .no-right
১১	#openSettings	⚙	Settings modal
৭.২ Workspace এবং প্যানেলের মাপ
প্যানেল	ডিফল্ট	ন্যূনতম	ভাঁজ	কীবোর্ড
Sidebar	280px	200px	✅	Ctrl+B
Main	flexible	480px	❌	—
Right	400px	320px	✅	Ctrl+J
Status bar	30px	—	❌ (P4)	—
Topbar	46px	—	❌	—
Responsive ব্রেকপয়েন্ট (UI-এর CSS থেকে):

>1200px: তিন কলাম সব দেখায়

≤1200px: right panel লুকিয়ে যায়, tab ক্লিকে .show-right যোগ হয়

≤900px: sidebar ও right panel overlay হয়ে যায় (drawer), .mobile-sidebar-open ক্লাস দিয়ে

≤640px: brand-text লুকায়

≤520px: host-pill label লুকায়

৭.৩ Sidebar — host-grouped session tree
UI-তে sidebar host-ভিত্তিক গ্রুপ, session/tag/file/prompt নয়। Tags, Files, Prompts, Analytics সরানো হয়েছে (দেখুন §7.13-এ কোথায় গেল)।

উপর থেকে নিচে:

New chat button (#newChatBtn)

পূর্ণপ্রস্থ, accent ব্যাকগ্রাউন্ড, + New chat, ডানে ⌘N (900px-এ লুকায়)

ক্লিক: new chat popover খোলে (§9.6)

Filter input (#sidebarSearch)

Placeholder: "Filter chats…"

ইনপুট: শুধু বর্তমান session row গুলোতে filter করে (host গ্রুপ থাকে)

Host list (#hostsList) — প্রতিটি host:

text
▼ [icon] Local           ●  3   [+]
   ● Add rate limiting             now    !
   ● Fix login bug                 2m
   ● Refactor auth                 2h
▼ [icon] prod-1          ●  2
   ● Deploy script                 1d    !
   ● Log aggregation               3d
▼ [icon] staging-2       ◐  1
   ● Update README                 5d
এলিমেন্ট	বিস্তারিত
.host-chev	chevron-down; collapsed হলে rotate(-90deg)
.host-icon	.local gradient (blue) বা .vps gradient (purple), ভিতরে laptop বা server
.host-name	এক লাইনে ellipsis
.host-status	4টি রঙ: connected (green dot + halo), degraded (amber), offline (grey), connecting (pulse)
.host-count	pill, mono font
.host-add	শুধু hover-এ দৃশ্যমান, + আইকন
.host-empty	কোনো session না থাকলে: + Start a chat, ইটালিক
Session row:

অংশ	বিবরণ
.sdot	বাম দিকে absolute, 5টি state: idle (grey), running (blue pulse), waiting (amber), success (green), error (red)
.session-title	এক লাইন ellipsis
.session-time	mono, relative (now/2m/2h/1d/3d)
.session-badge	শুধু তখনই দেখায় যখন unread > 0, waiting, বা error
active marker	বাম দিকে 2px accent বার, glow সহ
hover actions	Rename (pencil), Delete (trash-2); ডানে slide-in
Sidebar footer

+ Add host (VPS or local) — dashed border, hover-এ solid

Waiting-first নীতি (গ্যাপ #12): যে session awaiting_approval, stuck, বা budget_stop-এ আছে, সেটি তার host গ্রুপের শীর্ষে আসবে এবং amber ! badge পাবে।

৭.৪ Main — degraded banner, tab strip
Degraded banner (#degradedBanner, ডিফল্টে hidden)

text
📶 Not connected to prod-1. Files and history are still available.   [Reconnect]
--orange-subtle ব্যাকগ্রাউন্ড, --state-waiting টেক্সট

শুধু তখনই দেখায় যখন কোনো host degraded বা offline

Tab strip (৩৮px)

অংশ	বিস্তারিত
Scroll area	horizontal, scrollbar লুকানো, ডানে fade-out gradient
Tab	dot (state) + title + host chip + close (hover-এ)
Active tab	নিচে 2px accent বার, box-shadow glow
.tab-host	mono, 700px-এর নিচে লুকায়
Actions	columns-2 (split view, ⌘\), + (new chat, ⌘N)
৭.৫ Main — turn stream
প্রতিটি turn-এর কাঠামো (UI-এর সাথে হুবহু):

text
┌─────────────────────────────────────────────────────────────┐
│ ▸ Turns 1–6 collapsed · 8,420 tokens · $0.31                 │
├─────────────────────────────────────────────────────────────┤
│  YOU · 14:02                                                 │
│  Add rate limiting to the login route                        │
│  [thumb] [📷 screenshot.png] [@src/auth.ts]                  │
│                                                              │
│  Balanced · claude_code · sonnet        ~$0.10 – $0.28 fore. │
│                                                              │
│  ▾ 🧠 Thinking (4s)                                          │
│    I will add express-rate-limit …                           │
│                                                              │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ 📄 Read  src/auth.ts                 done · 42 ln   ▸ │  │
│  └───────────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ 📝 Edit  src/auth.ts            done · +18 −2      ▸ │  │
│  │   (+14) + import rateLimit from 'express-rate-limit';│  │
│  │   (+18) − app.post('/login', handler);               │  │
│  │   (+18) + app.post('/login', loginLimiter, handler); │  │
│  └───────────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ ▶ Run   npm test              running    ⟳           │  │
│  │   PASS auth.test.ts                                  │  │
│  │   FAIL rate.test.ts > limits after 5 tries           │  │
│  └───────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌─── ⚠ Test failed: rate.test.ts line 42 ─────────────┐   │
│  │  The limiter allows a 6th request …                  │   │
│  │  [✨ Fix this] [Show code] [Explain more]            │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                              │
│  Done · 1m 12s · 12,400 tokens · $0.16        [👍] [👎]      │
└─────────────────────────────────────────────────────────────┘
উপাদানসমূহের সুনির্দিষ্ট নিয়ম:

উপাদান	আচরণ
Collapsed summary	ক্লিক: পুরোনো turn গুলো expand; সবসময় token + cost দেখায়
Turn collapsing	শেষ ৫টি turn খোলা; আগেরগুলো এক লাইনে; ৩০+ হলে সব একসাথে summary block (গ্যাপ #11)
User message "who"	YOU · HH:MM, uppercase, ডানে divider লাইন
Attachment chips	thumb (gradient preview), image icon, @file accent chip
Turn meta	tier · engine · model, সাথে forecast পরিসর
Thinking block	ডিফল্টে collapsed, শুধু duration দেখায়; expand-এ italic reasoning
Tool card - Read	আইকন file-text, status done · NN ln
Tool card - Edit	আইকন file-pen, status done · +A −B, expand-এ diff line (add/remove রঙ)
Tool card - Run	আইকন play, status running/failed; live output ৫ লাইন window
Error card	--red-subtle, left border 3px red, ৩টি অ্যাকশন button
Turn footer	summary + feedback 👍👎
৭.৬ Prompt area
text
┌───────────────────────────────────────────────────────────┐
│ [⚡ Balanced · claude_code · sonnet ▾] [1 file] [12.4k ctx]│
│                                                            │
│ Queued: also add a test for this  [x]                      │
│ ┌───────────────────────────────────────────────────────┐ │
│ │ Ask or describe what you want to build…               │ │
│ │ [📎] [🖼] [@] [/]      Balanced · sonnet    [Send ⏎]   │ │
│ └───────────────────────────────────────────────────────┘ │
│       @ file · / commands · ⌘K palette                     │
└───────────────────────────────────────────────────────────┘
Model selector (§9.3)

Chips: 1 file (hash icon), 12.4k ctx (database icon)

Queued chips: সর্বোচ্চ ৩, প্রতিটির [x] বাতিল

Textarea: auto-grow, max 200px তারপর scroll

Toolbar: paperclip, image, at-sign, slash

Context hint: ডানে tier · model (৭০০px-এর নিচে লুকায়)

Send button: accent, Send + corner-down-left icon

Tip line: ৯০০px-এর নিচে লুকায়

৭.৭ Right panel — Preview ট্যাব
Toolbar: ← → ⟳ | URL bar (mono) | pop-out

Device presets: Mobile 390 · Tablet 768 · Desktop (ডিফল্ট active: Tablet)

Frame: 16:10 gradient placeholder, তার উপরে page name + path

Footer: 📷 Attach screenshot (full-width button)

৭.৮ Right panel — Console ট্যাব
text
✕ Uncaught ReferenceError: handleSubmit is not defined      3
  at LoginForm.tsx:42:11

⚠ Warning: Each child in a list should have a unique "key". 
  at UserList.tsx:18:5
─────────────────────────────────────────────────────────────
              [✨ Fix with agent]
error (red left border), warn (amber), info (blue)

প্রতিটি item ক্লিকযোগ্য → source line-এ jump

ডানে count badge (deduplicated)

নিচে fixed Fix with agent বোতাম (primary, full-width)

৭.৯ Right panel — Time Machine ট্যাব
text
┌──────────────────────────────────────┐
│ ┌────┐ now        turn 14            │
│ │img │ Added validation              │
│ └────┘                               │
│ ┌────┐ 2 min ago  turn 13   [Go back]│
│ │img │ Fixed the test                │
│ └────┘                               │
│ ┌────┐ 8 min ago  turn 12            │
│ │img │ Broke the login               │
│ └────┘                               │
│                                       │
│ [⇄ Compare two points]                │
└──────────────────────────────────────┘
Current entry: accent border + "CURRENT" badge

Hover: translateX(2px)

প্রতিটি thumbnail gradient (আসল হলে screenshot)

৭.১০ Right panel — Duel ট্যাব
দুটি pane পাশাপাশি, প্রতিটি:

text
┌──────────────────────────┬──────────────────────────┐
│ claude_code · sonnet     │ codex · default           │
│ 48s · $0.12 · PASS       │ 71s · $0.09 · FAIL        │
├──────────────────────────┼──────────────────────────┤
│ 3 files changed          │ 2 files changed           │
│ + rate.ts (new)          │ + auth.ts modified        │
│ + auth.ts modified       │ + package.json            │
│ + auth.test.ts           │                           │
├──────────────────────────┼──────────────────────────┤
│ [Diff]    [Keep]         │ [Diff]    [Keep]          │
└──────────────────────────┴──────────────────────────┘
PASS সবুজ, FAIL লাল

Keep চাপলে অন্যটি archive হয় (মুছে যায় না)

Simple মোডে Duel দেখা যায় না

৭.১১ Right panel — Verify ট্যাব
Target	আইকন	সময়
typecheck	✓/✕	2.1s
build	✓/✕	4.8s
test · rate.test.ts	✓/✕	1.2s
lint	✓/✕	0.9s
নিচে: ▶ Run verify (⌘⏎) — primary, full-width।

৭.১২ Right panel — Analytics ট্যাব
Spending chart (7d): SVG bar chart, gradient fill

Total this week: $4.12 বড় mono

By engine bars: claude_code 68%, codex 22%, gemini 10%

Limits: Claude Max ~60% (est), resets in 2h 14m

৭.১৩ Empty ও first-run states
কোথায়	হুবহু টেক্সট
No session open	"No chat open" / "Start a new chat, or pick one from the sidebar. Work across multiple chats and hosts." / chips: New chat, Connect a model, Add a VPS
No project	(v2.0-এর মতো) "Open a folder to get started" · Open folder · Try the sample project
No session on host	sidebar-এ: + Start a chat
Preview idle	"Login" / "src/routes/login.tsx"
Time Machine empty	"Checkpoints appear here after your first change."
Search no match	"No matches"
৭.১৪ Popovers ও Modals — ইনভেন্টরি
ID	ধরন	উদ্দেশ্য	সেকশন
#newChatPopover	popover	host বেছে নতুন চ্যাট	§9.6
#providerBd	modal (center)	Provider Hub	§9.10
#addHostBd	modal (center)	Add host	§9.7
#permissionBd	modal (center)	Permission	§9.8
#settingsBd	modal (center)	Settings (৭ ট্যাব)	§9.11
#searchBd	modal (top)	Search everything	§9.4
#paletteBd	modal (top)	Command palette	§9.2
#toastWrap	stack	Toast	§9.13
৭.১৫ Status bar
ক্রম (বাম থেকে ডানে):

● prod-1 · claude_code · sonnet · 3 providers · 3 chats · 3 hosts · ● ready

dot সবসময় state

৯০০px-এর নিচে: providers/chats/hosts hide

৫২০px-এর নিচে: শুধু conn

প্রতিটি item ক্লিকযোগ্য → সংশ্লিষ্ট panel/স্ক্রিন

৮. ডিজাইন সিস্টেম
৮.১ Color tokens (CSS variable — UI-এর সাথে হুবহু)
Dark (ডিফল্ট):

css
--bg-base:#0A0B0F;      --bg-raised:#101218;   --bg-overlay:#161921;
--bg-input:#06070A;     --bg-hover:#1A1E27;    --bg-active:#20252F;
--bg-glass:rgba(22,25,33,.85);

--border-subtle:#1B1F29; --border-default:#282E3B;
--border-strong:#3A4252; --border-focus:#5B9CFF;

--text-primary:#E9EEF5;  --text-secondary:#96A1B3;
--text-muted:#5C677A;    --text-faint:#3A4354;

--accent:#5B9CFF;        --accent-hover:#7CB0FF;
--accent-subtle:rgba(91,156,255,.10);
--accent-glow:rgba(91,156,255,.35);

--purple:#A78BFA;        --purple-subtle:rgba(167,139,250,.12);
--green:#3DD68C;         --green-subtle:rgba(61,214,140,.12);
--orange:#F5A524;        --orange-subtle:rgba(245,165,36,.12);
--red:#F25E68;           --red-subtle:rgba(242,94,104,.12);

--state-idle:#5C677A;    --state-running:#5B9CFF;
--state-waiting:#F5A524; --state-success:#3DD68C;
--state-error:#F25E68;

--diff-add-bg:rgba(61,214,140,.12);
--diff-remove-bg:rgba(242,94,104,.12);
--diff-add-text:#6EE7A8;
--diff-remove-text:#FF8A91;
Light ([data-theme="light"]):

css
--bg-base:#F6F8FC;       --bg-raised:#FFFFFF;  --bg-overlay:#FFFFFF;
--bg-input:#FFFFFF;      --bg-hover:#EDF1F8;   --bg-active:#E1E8F3;
--border-subtle:#E4E9F2; --border-default:#CFD7E4;
--border-strong:#A9B5CA; --border-focus:#2C7BFF;
--text-primary:#111827;  --text-secondary:#4E5A72;
--text-muted:#8A95A9;    --text-faint:#C5CDDB;
--accent:#2C7BFF;        --accent-hover:#1B6CE8;
--accent-subtle:rgba(44,123,255,.08);
--purple:#7C3AED;        --green:#16A34A;
--orange:#D97706;        --red:#DC2626;
--state-success:#16A34A; --state-error:#DC2626;
--state-waiting:#D97706; --state-running:#2C7BFF;
নিয়ম: কোনো component সরাসরি hex ব্যবহার করবে না — শুধু semantic নাম (bg-raised, state-waiting)। রঙ একা অর্থ বহন করে না (§8.6)।

৮.২ Typography
টোকেন	মান	ব্যবহার
--font-ui	Inter, system-ui, -apple-system, sans-serif	UI
--font-mono	JetBrains Mono, Menlo, Consolas, monospace	কোড, path, diff
base size	13px	root
letter-spacing	-0.005em	UI
line-height	1.5	UI
Skew: 90% / 100% / 110% / 125%। সব rem-এ।

৮.৩ Spacing ও shape
টোকেন	মান
--r-xs	3px
--r-sm	5px
--r-md	7px
--r-lg	10px
--r-xl	14px
--r-full	9999px
--shadow-sm	0 1px 2px rgba(0,0,0,.5)
--shadow-md	0 6px 20px rgba(0,0,0,.5)
--shadow-lg	0 18px 44px rgba(0,0,0,.65)
--shadow-xl	0 32px 80px rgba(0,0,0,.8)
border width	1px (hairline), 2px শুধু focus ring
ঘনত্ব: row height 28px, button sm 24px / md 28px / lg 34px, icon 16px।

৮.৪ Motion
টোকেন	মান
--fast	90ms
--base	150ms
--slow	220ms
--ease	cubic-bezier(.4,0,.2,1)
--ease-out	cubic-bezier(0,0,.2,1)
--spring	cubic-bezier(.34,1.56,.64,1)
নিয়ম: কোনো transition ১৬০ms-এর বেশি অপেক্ষা তৈরি করবে না (প্যানেল resize ছাড়া)। prefers-reduced-motion → সব 0।

৮.৫ Icons
Lucide React, 16px ডিফল্ট, stroke-width 1.5।

অর্থ-বাহী আইকনের নির্দিষ্ট ম্যাপিং (UI-তে যেভাবে ব্যবহৃত):

play running · pause waiting · check-circle-2 success · x-circle error · alert-triangle danger · clock Time Machine · terminal-square console · file-text Read · file-pen Edit · eye Preview · columns-2 split · swords Duel · bar-chart-3 Analytics · zap fast · scale balanced · brain deep · plug provider · server/laptop host · wand-2 fix · stethoscope doctor · shield-check safety · sparkles brand

৮.৬ Accessibility (ভিত্তি স্তর, বাধ্যতামূলক)
কীবোর্ড নেভিগেশন প্রতিটি কাজে (P7)

Focus ring 2px, --border-focus; outline: none নিষিদ্ধ

কনট্রাস্ট: সাধারণ টেক্সটে ৪.৫:১, বড় টেক্সটে ৩:১

রঙ একা অর্থ বহন করে না — সাথে icon বা শব্দ

Modal-এ role="dialog", streaming অংশে aria-live="polite"

Modal ও palette-এ focus trap

Esc সব overlay বন্ধ করে

prefers-reduced-motion সম্মানিত

পূর্ণ WCAG AA অডিট v1-এর পরে

৯. ইন্টারঅ্যাকশন স্পেক
৯.১ কীবোর্ড ম্যাপ
বৈশ্বিক:

শর্টকাট	কাজ
Ctrl+K	Command palette
Ctrl+P	Search everything
Ctrl+N	New session
Ctrl+B	Sidebar toggle
Ctrl+J	Right panel toggle
Ctrl+\	Split view
Ctrl+,	Settings
F1	Keyboard reference
সেশন:

শর্টকাট	কাজ
Enter	Send
Shift+Enter	Newline
Esc	Interrupt
Ctrl+Shift+Esc	Force kill
Ctrl+Z	Rewind last turn
Ctrl+Shift+Z	Redo
Ctrl+E	Time Machine
Ctrl+Enter	Run verify
Alt+M	Model tier
Alt+E	Engine
Approval dialog: Enter default, Esc deny, A allow once, Shift+A always allow (MUTATING only), S show me the file, D deny।

Timeline: J/K পরের/আগের turn, O expand/collapse, G G শুরু, Shift+G শেষ, Y summary copy।

৯.২ Command palette
text
┌───────────────────────────────────────────────────┐
│ >                                             Esc │
├───────────────────────────────────────────────────┤
│ RECENT                                            │
│   + New chat                                ⌘N   │
│   🔌 Connect a provider / model                    │
│   🖥  Add a host                                    │
│                                                   │
│ ACTIONS                                           │
│   ⧉  Toggle split view                      ⌘\   │
│   🔍 Search everything                      ⌘P   │
│   ✓  Run verify                             ⌘⏎   │
│   🩺 Run environment doctor                       │
│   ⚙  Open settings                          ⌘,   │
└───────────────────────────────────────────────────┘
cmdk-এ তৈরি

P7-এর প্রয়োগ: প্রতিটি action registry-তে নিবন্ধিত

> action, @ file, # tag, / prompt

শর্টকাট প্রতিটি সারিতে দেখায়

Fuzzy match, সাম্প্রতিক ব্যবহারে র‍্যাঙ্ক

৯.৩ Model dropdown
Prompt area-র tier chip-এ ক্লিক:

text
┌──────────────────────────────────────────┐
│ TIER                                     │
│   ⚡ Fast      quick edits                │
│   ⚖  Balanced  everyday              ✓   │
│   🧠 Deep      architecture              │
├──────────────────────────────────────────┤
│ ENGINE                                   │
│   ✨ Claude Code     Claude Max sub.      │
│   ✨ Codex CLI       ChatGPT Plus     ✓   │
│   ✨ Gemini CLI      Google AI            │
│   ⌨  Native API      API key             │
├──────────────────────────────────────────┤
│ MODEL                                    │
│   📦 Haiku 4      fast                    │
│   📦 Sonnet 4.5   balanced             ✓  │
│   📦 Opus 4       deep                    │
├──────────────────────────────────────────┤
│ + Connect more providers or models…      │
│ 🛡 Every change visible       ~$0.10–$0.28│
└──────────────────────────────────────────┘
উপরে ওঠে (bottom: calc(100% + 8px))

Width 380px

Tier বদলালে model অটো-ম্যাপ হয় (fast→Haiku, balanced→Sonnet, deep→Opus)

Engine বদলালে tier অনুযায়ী model অটো-সেট হয়

৯.৪ Search overlay (Ctrl+P)
তিনটি গ্রুপ: Sessions, Files, Prompts।

Session hit: title + host + prompt snippet, hit highlight

File hit: mono path

Prompt hit: title + body snippet

ক্লিকে: session → open; file → toast; prompt → insert

SQLite FTS5 index করে: session title, প্রতিটি prompt, প্রতিটি turn summary, ছোঁয়া file path

Empty: "No matches"

৯.৫ New chat popover
⌘N বা + New chat ক্লিকে:

text
┌─ New chat on… ──────────────────┐
│  💻 Local                        │
│     3 chats · connected          │
│  🖥  prod-1                        │
│     2 chats · connected          │
│  🖥  staging-2                     │
│     1 chat · degraded            │
└──────────────────────────────────┘
Trigger button-এর বাম-প্রান্তে aligned

ক্লিকে: সেই host-এ নতুন session, tab খোলে, focus prompt box-এ

৯.৬ Prompt box-এর ভাষা
@ → file reference, fuzzy autocomplete, নির্বাচিত হলে chip

/ → slash commands: /model, /engine, /undo, /redo, /compact, /clear, /verify, /diff, /cost, /save, /help

# → tag

Ctrl+V ছবি → thumbnail chip

Drag & drop → file chip

[📎] → file picker

[🖼] → paste hint

Preview → Attach screenshot

৯.৭ Streaming ও queued steering
পরিস্থিতি	আচরণ
ইউজার নিচে	auto-scroll follow
ইউজার উপরে	scroll থামে, নিচে 12 new pill
বড় tool output	card-এর ভিতরে ৫ লাইনের window, নিজস্ব scroll
খুব দ্রুত token	60fps throttle
Thinking block	ডিফল্টে collapsed, শুধু duration
Queued steering:

সর্বোচ্চ ৩টি

প্রতিটি বাতিলযোগ্য

Turn শেষ হলে একসাথে next prompt-এ

Esc → turn থামে, queued সাথে সাথে পাঠানো হয়

৯.৮ Notification ও sound
ঘটনা	Notification	Sound
Turn শেষ	unfocused হলে	ঐচ্ছিক
Approval দরকার	সবসময়	ঐচ্ছিক
Stuck detected	সবসময়	ঐচ্ছিক
Budget cap	সবসময়	ঐচ্ছিক
Error	unfocused হলে	না
Sound ডিফল্টে বন্ধ। Window focused থাকলে কখনো notification নয়।

৯.৯ Attachment ও screenshot
(আগের মতো — Path: Ctrl+V / drag / clip / preview / console)

৯.১০ Provider Hub
Trigger: topbar-এর plug আইকন (#openProviders) বা palette-এর "Connect a provider"

Modal: 920×680, দুই কলাম — বাম ন্যাভ 220px, ডান body।

বাম ন্যাভ (৭টি):

আইটেম	আইকন	উদ্দেশ্য
All providers	grid-3x3	সব card
Subscriptions	crown	Claude / OpenAI / Gemini OAuth
API keys	key	Direct API
Local (Ollama)	hard-drive	এ মেশিনের মডেল
Custom endpoint	plug	OpenAI-compatible
Model registry	list	সব মডেল enable/disable
Environment doctor	stethoscope	১০টি চেক
Provider card:

text
┌───────────────────────────────────┐
│  [C]  Claude                  ●   │
│       Subscription                │
│                                    │
│  Claude Pro / Max subscription    │
│  · uses your own login            │
│                                    │
│  [Connected]           [Manage]   │
└───────────────────────────────────┘
status dot: connected (green), needs-auth (amber), available (grey)

Logo: gradient circle, initial letter

Card ক্লিক → connect flow

connected → Manage; না হলে → [Connect] primary

Logo gradient (CSS class):

Class	Gradient
.claude	#D97757 → #B85D3F
.openai	#10A37F → #0D8567
.gemini	#4285F4 → #7B5CFF
.deepseek	#4D6BFE → #3B52C7
.groq	#F55036 → #D64028
.openrouter	#6366F1 → #4F46E5
.ollama	#4A4A4A → #2B2B2B
.custom	#5B9CFF → #7B5CFF
Flow ১ — API key:

Form: API key (password), label

Buttons: Back, Test connection, Save

Test → spinner → "OK · key valid · 12 models available"

Save → status connected, sk-ant-…4f8a masked

Flow ২ — Subscription:

Dialog note: "A browser window will open. X handles the login; SDC only receives a token."

Open browser login → spinner → "Authorized · token received"

Flow ৩ — Local (Ollama):

Doctor rows: daemon running, installed models

Connect → toast, close

Flow ৪ — Custom endpoint:

URL, API key, protocol select (OpenAI-compatible / Anthropic-compatible)

Save → toast

Flow ৫ — Model registry:

List of models with checkbox

প্রতিটি: mono id, ctx, cost, tier badge

ক্লিক: toggle enabled

Flow ৬ — Environment doctor:

১০টি চেক, প্রতিটি ok/warn/fail

fail/warn → [Fix] button

৯.১১ Settings modal (৭টি ট্যাব)
বাম ন্যাভ 200px, ডান কনটেন্ট।

General:

Restore last session on launch (toggle)

Confirm before closing (toggle)

Auto-verify after turn (toggle)

Default permission mode (select: Ask / Auto-edit / Full auto)

Appearance:

Color theme (Dark / Light)

Font size (90% / 100% / 110% / 125%)

Compact rows (toggle)

Keymap:

Global (১০টি শর্টকাট, ⌘K দেখায়)

Session (৬টি)

Editor mode (Default / Vim / Emacs)

Safety:

Learn allowlist per project

Show dry-run preview for DANGEROUS

bubblewrap sandbox (Linux only)

Block .env, *.pem, id_rsa

Redact secrets in logs and prompts

Notifications:

Notify when approval is needed

Notify when turn completes (if unfocused)

Notify on stuck / budget stop

Sound cues

Test sound

Backup:

Auto-backup (weekly, keep 4)

Recent backups list: sdc-backup-2026-09-19.tar.age (2.4 MB · encrypted · 12 minutes ago)

Create backup now / Restore from file

About:

Version info (App v0.4.1, sdcd v0.4.1, SDCP 0.1)

This host: Local · macOS 15.1 · arm64

Diagnostics: Run doctor / Create bundle / Check updates

Telemetry opt-in

৯.১২ Add Host modal
Type picker (2টি):

text
┌──────────────┬──────────────┐
│   💻 Local   │   🖥  SSH/VPS  │
│ This computer│   user@host  │
└──────────────┴──────────────┘
SSH নির্বাচনে field:

SSH target (user@vps.example.com)

Label (optional)

Footer: Cancel | Connect (primary)

Submit: daemon install → spinner → toast → connected

৯.১৩ Permission modal
text
┌──── ⚠ Delete a file ────────────────────────┐
│                                              │
│  Claude wants to perform a mutating action  │
│                                              │
│  ┌────────────────────────────────────────┐ │
│  │ src/database.js                         │ │
│  └────────────────────────────────────────┘ │
│                                              │
│  What this file does:                        │
│  Your database connection settings. If this  │
│  is deleted, your app will stop loading data.│
│                                              │
│  🛡 A checkpoint was saved before this turn, │
│  so you can undo it either way.              │
│                                              │
│                    [Esc · Deny] [Allow once]│
└──────────────────────────────────────────────┘
Risk-ভিত্তিক footer: DANGEROUS → Deny ডিফল্ট; MUTATING → Allow once ডিফল্ট

Always allow শুধু MUTATING-এ

ডান-ক্লিকের বিকল্প সবসময় থাকবে Show me the file

৯.১৪ Toast
text
┌──────────────────────────────────────────────────┐
│  Connected: prod-1              Got it          │
└──────────────────────────────────────────────────┘
নিচে-মাঝখানে stack (bottom: 44px)

3s hold, তারপর fade

Action chip ঐচ্ছিক, ডানে

৯.১৫ Split view
⌘\ বা #splitBtn:

main-content দুই .pane-এ ভাগ (flex: 1)

দ্বিতীয় pane-এর session: যদি আগে থেকে splitSecondary না থাকে, active বাদে প্রথম

প্রতিটি pane-এ নিজের header (host + title) দেখায়

border-right: 1px মাঝে

Split বন্ধ করলে state সংরক্ষিত (পরেরবার আগের pair ফেরে)

১০. মডিউল ক্যাটালগ — ৯২টি বিল্ডযোগ্য একক
v2.0-এর ৭৮টির সাথে যোগ হলো M-series UI modules (১১টি) এবং তিনটি সংশোধিত/নতুন — মোট ৯২।

M-series · UI ও ইন্টারঅ্যাকশন (১১)
ID	মডিউল	উদ্দেশ্য	নির্ভরতা	সম্পন্ন যখন	Phase
M1	Topbar	brand · host-pill · mode-switch · palette-btn · 6 icon-btn	A1, A6, A7	সব বাটন কাজ করে, responsive breakpoint মানে	1
M2	Sidebar (host-grouped)	new-chat, filter, host group, session row, badge	M1, B5	3 host-এ 8 session render, waiting session শীর্ষে	1
M3	Tab strip	scroll, dot, host chip, close, split, new	M2	৫টি tab-এ overflow scroll, close state সঠিক	1
M4	Turn stream	user-msg, thinking, tool-card (Read/Edit/Run), error-card, footer	M3, B5	২০-turn সেশন virtual scroll-এ ৬০fps	1
M5	Prompt area	model-selector, chips, queue, textarea, toolbar, tip	M4	Enter send, Shift+Enter newline, @ autocomplete	1
M6	Model dropdown	tier/engine/model ৩ গ্রুপ, cost footer	M5, C1	tier↔model ম্যাপিং সঠিক, cost ±৪০%	2
M7	Right panel — 6 tabs	Preview, Console, TM, Duel, Verify, Analytics	M4	৬টি tab, badge, panel width drag	1–4
M8	Status bar	host · engine · model · providers · chats · hosts · conn	M1	৭ সেগমেন্ট, ৯০০px-এ hide-sm	1
M9	Command palette	cmdk, groups, fuzzy, shortcut hint, registry	A6, A7	প্রতিটি action registered, ৫০ms-এ খোলে	1
M10	Search overlay	FTS5, ৩ group, highlight, jump-to-turn	D3, M9	৫০০ সেশনে <২০০ms	3
M11	Provider Hub	৭ nav, card, ৪ flow, registry, doctor	C1, C3	API key save, OAuth sim, registry toggle	2
সংশোধিত মডিউল
ID	পরিবর্তন
A2 (Layout Engine)	breakpoints 1200/900/640/520 যোগ, sidebar 280px, right 400px
A3 (Theme System)	v3.0 CSS token থেকে generate
A6 (Command Registry)	M9-এর palette-এর সাথে আঁটসাঁট
E1 (Permission Broker)	modal টেক্সট ও footer নিয়ম v3.0 অনুযায়ী
বাকি সব (A1, A4–A5, A7–A9, B1–B16, C1–C9, D1–D9, E2–E11, F1–F6, G1–G6, H1–H10, I1–I12)
v2.0-এর মতো অপরিবর্তিত। মোট M-series ১১ + বাকি ৮১ = ৯২।

সংযুক্তি: A–F + I1–I3 + M1–M8 = ব্যবহারযোগ্য প্রোডাক্ট। M9–M11 + G + B16(native) = উচ্চাভিলাষী স্তর। I5–I12 = অন্যদের কাছে ছাড়ার জন্য।

১১–২২. (v2.0 অপরিবর্তিত, তবে যেখানে UI-এর সাথে সংঘর্ষ ছিল সংশোধিত)
v2.0-এর §11–§22-এর সব কিছু বহাল, শুধু নিচের সংশোধনগুলো:

§11.2 Claude Code adapter
--include-partial-messages ফ্ল্যাগ বাধ্যতামূলক (streaming UX §9.7-এর জন্য)।

§11.6 Adapter VCR
fixture সংখ্যা ১২টি অপরিবর্তিত, তবে fixture 13: "User pastes screenshot" — vision path।

§12.9 Native engine reliability
Timeout হলে stuck_detected event emit হবে (§9.7-এর সাথে synchronized)।

§13.4 Provider features
UI-তে দেখানোর নিয়ম: শুধু Simple মোডে extended thinking লুকানো; Pro মোডে toggle; Auto-তে router সিদ্ধান্ত নেয়।

§13.9 তিনটি নির্বাচন মোড
v2.0-এর সাথে যোগ: mode switch topbar-এ, ডিফল্টে Pro; Simple-এ মডেল নাম কখনো দেখা যায় না, শুধু "Fast / Balanced / Deep"।

§14.5 Rewind Redo
UI-তে toast: Went back to turn 12. [Undo this] — 10 সেকেন্ড।

§16.6 Duel Mode
Right panel-এ ৬ষ্ঠ tab (Analytics-এর পরে নয়, Verify-এর পরে — UI অনুযায়ী ক্রম: Preview, Console, TM, Duel, Verify, Analytics)।

§17.9 Multi-user
socket path, DB, shadow git, worktree, blob সব $UID-namespaced।

§20.2 Setup Wizard
6টি step অপরিবর্তিত, তবে UI-তে Provider Hub-ও setup-এর অংশ — wizard থেকে সরাসরি #providerBd খোলে।

সংযুক্তি A — UI ↔ Doc ম্যাপিং টেবিল
প্রতিটি UI এলিমেন্ট কোথায় সংজ্ঞায়িত:

UI এলিমেন্ট	সেকশন
.topbar	§7.1
.brand-mark / .brand-text	§7.1
#activeHostBtn	§7.1, §17.7
.mode-switch	§7.1, §13.9
#openPalette	§7.1, §9.2
#openProviders	§7.1, §9.10
#themeToggle	§7.1, §8.1
#toggleSidebar / #toggleRight	§7.1, §7.2
#openSettings	§7.1, §9.11
.sidebar	§7.3
#newChatBtn	§7.3, §9.5
#sidebarSearch	§7.3
.host-group	§7.3
.session-item	§7.3
#addHostBtn	§7.3, §9.12
#degradedBanner	§7.4, §17.8
.tabstrip	§7.4
#splitBtn	§9.15
.turn	§7.5
.collapsed-summary	§7.5
.user-msg	§7.5
.thinking	§7.5, §9.7
.tool-card	§7.5
.error-card	§7.5, §14.9
.turn-footer	§7.5, §19.4
.prompt-area	§7.6
.model-selector	§7.6, §9.3
.queued-chips	§7.6, §9.7
.prompt-box	§7.6
.tip-line	§7.6
.rightpanel	§7.7–§7.12
[data-panel="preview"]	§7.7, §15.4
[data-panel="console"]	§7.8, §15.5
[data-panel="timemachine"]	§7.9, §14.4
[data-panel="duel"]	§7.10, §16.6
[data-panel="verify"]	§7.11, §15.6
[data-panel="analytics"]	§7.12, §7.7
.statusbar	§7.15, §13.10
#newChatPopover	§9.5
#providerBd	§9.10
#addHostBd	§9.12
#permissionBd	§9.13
#settingsBd	§9.11
#searchBd	§9.4
#paletteBd	§9.2
.toast	§9.14
app/src/strings.ts	§2.7
সংযুক্তি B — v3.0-এ যা যোগ/বিয়োগ
যোগ:

Topbar icon-btn গুলো মডিউল A6-এ registered

Provider hub (M11) এবং তার ৭টি nav

Settings-এর ৭টি ট্যাব

Right panel-এর ৬টি ট্যাব

Turn collapsing-এর সুনির্দিষ্ট রেন্ডার

Queue chips-এর UI

Split view

Light theme

Device presets

Duel-এর "Keep" behavior

বিয়োগ (UI-তে নেই):

Sidebar-এ Tags / Files / Prompts / Analytics panel

Tags → Command palette # ও Search-এ

Files → Search-এ ও @ autocomplete-এ

Prompts → Search-এ ও / command-এ

Analytics → Right panel-এর tab

Focus Mode (Ctrl+Shift+F) — v3.0-এ সরানো হলো, কারণ UI-তে এর জন্য স্পষ্ট toggle নেই

Pop-out window (§7.8 v2.0) — v3.0-এ রাখা হয়েছে ভবিষ্যতের জন্য, কিন্তু UI-তে button নেই

সংশোধন:

Design tokens (§8.1) — পুরোপুরি নতুন

Layout মাপ — 280/400/46/30

Typography base 13px

Motion duration — 90/150/220

সংযুক্তি C — v3.0-এর জন্য UI prompt templates
UI বানানোর প্রতিটি prompt-এ আগে এই ডকুমেন্টের সংশ্লিষ্ট সেকশন হুবহু পেস্ট করবে। যেমন:

text
# CONTEXT
I am building SDC's sidebar per Section 7.3 of the master spec.
Stack: React 18 + TypeScript + Tailwind + Lucide React.
Design tokens come from §8.1 (never hardcode).

# COMPONENT
Name:     SidebarHostGroup
Purpose:  Render one host and its sessions in the sidebar
Location: app/src/panels/sessions/SidebarHostGroup.tsx

# PROPS
<paste the exact structure from §7.3>

# STATES
- host.status: connected | degraded | offline | connecting
- session.state: idle | running | waiting | success | error
- session.unread: 0 | N
- collapsed: true | false
- hover: false | true

# BEHAVIOUR
- Click host header → toggle collapsed
- Click session → open
- Hover session → show rename/delete
- Click + on host → new chat on that host

# CONSTRAINTS
- No own state beyond local UI state
- Read from event store
- Never hardcode colours

# ACCEPTANCE
- 3 hosts × 4 sessions render at 280px
- Waiting sessions appear at top of their group
- Chevron rotates on collapse
সংযুক্তি D — v3.0-এ খোলা প্রশ্ন (আগের + নতুন)
আগের ৫টি (Go module path, download host, code-signing বাজেট, সাপ্তাহিক ঘণ্টা, eval task) অপরিবর্তিত। যোগ হলো:

Light theme-এর মানচিত্র — CSS-এ দেওয়া আছে, কিন্তু v2.0-এর কিছু component dark-only ছিল। প্রতিটি component-এর light variant আলাদা করে লিখতে হবে (Phase 1)।

Mobile drawer interaction — 900px-এর নিচে sidebar ও right panel overlay হয়। Touch gesture কী হবে (swipe?) — নির্দিষ্ট করা হয়নি।

Host-pill popover-এ host switch — UI-তে বাটন আছে, popover আছে কি না স্পষ্ট নয়। সম্ভবত palette-এর "Add host" + sidebar-এর host header দিয়েই যথেষ্ট।

Provider hub-এর "Manage" বোতাম — এডিট ফর্ম না কি disconnect? v1-এ শুধু status দেখাবে।

Duel-এর "Keep neither" — কোনো বাটন না থাকলে কী হবে? পাঠানো হয়েছে: Keep neither · try again।

এক অনুচ্ছেদে পুরো সূত্র (v3.0)
অফিসিয়াল CLI ও সরাসরি API-কে একটিমাত্র Engine interface-এর পেছনে বিনিময়যোগ্য ইঞ্জিন হিসেবে চালাও, আর চালাও সেই হোস্টে যেখানে কোড আছে। SDCP একটি append-only event log-এর উপর দাঁড়ায়; UI একটি pure reducer যা শুধু event থেকে আঁকে। উপরে পাঁচটি differentiator: ছবিসহ Time Machine যা ফাইল ও কথোপকথন একসাথে ফেরায় এবং redo দেয়, Error Translator যা ভয়কে এক বোতামে বদলায়, Session Bridge যা মাঝপথে ইঞ্জিন বদলেও সূত্র ধরে রাখে, Preview→Agent loop যা runtime error সরাসরি agent-এ ফেরত পাঠায়, আর Duel Mode যা দুটি ইঞ্জিনকে পাশাপাশি লড়িয়ে মানুষকে বিচারক বানায়।

UI হলো topbar-এ ৮টি বাটন, বাম দিকে host-grouped session tree, মাঝে virtualized turn stream যার প্রতিটি tool call একটি কার্ড, ডানে ৬টি panel (Preview/Console/TimeMachine/Duel/Verify/Analytics), নিচে একটি সৎ status bar — এবং উপরে একটি command palette যা প্রতিটি কাজ কীবোর্ডে আনে। সব string ইংরেজি, সব রঙ একটি token ফাইল থেকে, সব কাজ কীবোর্ডে, সব পরিবর্তন ফেরানো যায়। AI ওদের; নিয়ন্ত্রণ, নিরাপত্তা ও বোধগম্যতা তোমার।
