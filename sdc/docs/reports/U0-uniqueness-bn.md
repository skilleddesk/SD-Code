# U0 — Uniqueness audit (I1–I12)

তারিখ: 2026-10-14 · উৎস: শুধু official docs, changelog, vendor blog (অল্প কিছু ঘর search-snippet থেকে) · প্রায় ৩০টি search/fetch।
Phase 7-এ আবার চালানো হবে।

চিহ্ন: **has** = হুবহু আছে · **partial** = কাছাকাছি কিছু আছে · **no** = নির্দিষ্টভাবে খুঁজে পাওয়া যায়নি · **?** = প্রমাণ
পাইনি (এটা "নেই"-এর প্রমাণ নয়)। Antigravity, OpenCode, Windsurf কলাম হালকাভাবে দেখা হয়েছে, তাই সেখানে "?" বেশি।

> নোট: master prompt-এ I12-এর সংজ্ঞা নেই। এই audit-এ I12 = "journal-ভিত্তিক durable resume + E2E-encrypted cross-device
> handoff" ধরে নেওয়া হয়েছে — Gate 0-এ নিশ্চিত করতে হবে।

## সারণি

| | Claude Code | Codex | Cursor | Copilot | Windsurf/Devin | Cline/Kilo | Kiro | Antigravity | Gemini CLI | Aider | OpenCode |
|---|---|---|---|---|---|---|---|---|---|---|---|
| I1 Context Ledger | partial | ? | ? | ? | ? | ? | ? | ? | ? | ? | ? |
| I2 Skeleton reads | ? | ? | ? | ? | ? | partial | ? | ? | ? | partial | ? |
| I3 Draft–Verify | partial | ? | ? | ? | ? | ? | ? | ? | ? | partial | ? |
| I4 Recipe memory | partial | partial | partial | partial | partial | partial | ? | partial | ? | ? | ? |
| I5 Budget contract | partial | no | ? | ? | ? | no | ? | ? | ? | ? | ? |
| I6 Privacy router | partial | ? | partial | partial | ? | partial | ? | ? | ? | ? | ? |
| I7 Fleet fan-out | no | no | no | no | no | no | no | no | no | no | no |
| I8 Interlingua | no | no | no | no | no | no | no | no | no | no | no |
| I9 Provider failover | partial | ? | ? | ? | ? | ? | ? | ? | partial | ? | partial |
| I10 Token receipt | partial | partial | ? | ? | partial | partial | partial | ? | partial | ? | ? |
| I11 Self-improving core | partial | partial | partial | partial | partial | partial | ? | partial | ? | ? | ? |
| I12 Durable resume/handoff | partial | partial | partial | partial | ? | partial | partial | ? | partial | ? | ? |

## প্রমাণ (partial ঘরগুলো)

- **I1 · Claude Code** — Read tool অপরিবর্তিত ফাইল আবার পড়লে "unchanged since your last Read" stub দেয় (CHANGELOG v2.1.86)। বদলানো ফাইল পুরোটা পাঠায় (diff নয়), recall tool নেই; git checkout/rewind-এর পর stub পুরনো হয়ে যাওয়ার bug report আছে। https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- **I2 · Aider** — repo map (`--map-tokens`), পুরো repo-র জন্য; এক ফাইলের symbol expand নেই। https://aider.chat/docs/config/options.html
- **I2 · Kilo Code** — `list_code_definition_names`: directory-স্তরে, সর্বোচ্চ ৫০ ফাইল, imports নেই। https://kilo.ai/docs/automate/tools/list-code-definition-names
- **I3 · Claude Code** — advisor tool (experimental): সস্তা model দরকারে শক্তিশালী model-এর পরামর্শ নেয়; পুরো কথোপকথন দেখে, শুধু diff নয়, test-gate নেই। `opusplan` = plan Opus, কাজ Sonnet। https://code.claude.com/docs/en/advisor
- **I3 · Aider** — architect/editor mode; test-gate বা diff-only review নেই। https://aider.chat/docs/usage/modes.html
- **I4** — Claude Code auto memory (debug fix রাখে না) https://code.claude.com/docs/en/memory · Codex Memories "recurring workflows" (2026-06-16) https://developers.openai.com/codex/memories · **Antigravity (সবচেয়ে কাছে)**: "series of steps taken to successfully complete a subtask" https://antigravity.google/blog/introducing-google-antigravity/ · Cursor Memories / Generate Rules https://cursor.com/changelog/1-0 · Copilot Memory https://docs.github.com/en/copilot/concepts/agents/copilot-memory · Windsurf memories (শুধু legacy Cascade-এ) https://docs.windsurf.com/windsurf/cascade/memories · Kilo Memory https://blog.kilo.ai/p/introducing-kilo-memory
- **I5 · Claude Code** — `--max-budget-usd`, শুধু print mode-এ hard stop; adaptive কৌশল বা 80%-এ গুটিয়ে আনা নেই। https://code.claude.com/docs/en/cli-reference
- **I6** — Cursor `.cursorignore` (নিজের docs বলছে terminal/MCP এটা মানে না) https://cursor.com/docs/agent/security · Copilot content exclusion (agent mode ও CLI-তে কাজ করে না) https://docs.github.com/en/copilot/concepts/context/content-exclusion · Kilo `.kilocodeignore` + deny rules · Claude Code permission deny rules। কারো embeddings-সহ file-sensitivity policy বা "কী কোথায় গেল" audit log নেই।
- **I9** — Claude Code `--fallback-model` (শুধু Anthropic model-এর মধ্যে) · Gemini CLI Pro→Flash · OpenCode হাতে provider বদল (native checkpoint একই provider-এ আটকানো) https://opencode.ai/v2/docs/compaction/
- **I10** — Claude Code `/context`, `/cost` https://code.claude.com/docs/en/context-window · Gemini CLI `/stats` cached savings https://geminicli.com/docs/cli/token-caching/ · Devin Desktop প্রতি উত্তরে in/out/cached (Sep 2026) · Cline cost header · Kiro `/context show` · Codex account-স্তরের usage chart। কোনোটায় "কোন কৌশলে কত বাঁচল" নেই।
- **I11 · Copilot (সবচেয়ে কাছে)** — memory-তে code citation, ব্যবহারের আগে বর্তমান branch-এ যাচাই, ২৮ দিনে মেয়াদ শেষ; canary/rollback নেই। https://github.blog/changelog/2026-01-15-agentic-memory-for-github-copilot-is-in-public-preview/ · বাকিদের যাচাইহীন auto memory।
- **I12** — Claude Code `--resume`, `--teleport`, Remote Control · Codex thread handoff (2026-06-18), Codex Remote GA (2026-06-25) https://learn.chatgpt.com/docs/changelog · Cursor local→cloud handoff https://cursor.com/changelog/cli-jan-16-2026 · Copilot `/delegate` + `--resume` · Gemini CLI checkpointing · Cline checkpoints · Kiro rewind। কেউই step-journal দিয়ে crash-এর ঠিক জায়গা থেকে resume বা E2E-encrypted handoff প্রচার করে না।

## রায়

| ID | রায় | SDC-র পার্থক্য |
|---|---|---|
| I1 | আংশিক অনন্য | প্রতিটা chunk-এর hash (শুধু Read নয়), বদলে গেলে diff, `recall(Lk)`; Claude Code-এর stale-stub bug দেখায় সঠিক freshness-ই আসল মূল্য |
| I2 | আংশিক অনন্য | এক ফাইলের outline-first read + symbol ধরে expand |
| I3 | আংশিক অনন্য | test-gate + দামি model শুধু diff দেখে |
| I4 | আংশিক অনন্য | শুধু verify-সবুজ কাজ থেকে recipe, মিলে যাওয়া কাজে খুঁজে আনা |
| I5 | আংশিক অনন্য | prompt-এর ভেতরে budget, কৌশল বদল, 80%-এ গুটানো, interactive |
| I6 | আংশিক অনন্য | terminal/MCP/embeddings সব পথে প্রয়োগ + "কী কোথায় গেল" log |
| I7 | **অনন্য** (এই tool-গুলোর মধ্যে) | pdsh/dshbak-এর মতো পুরনো sysadmin tool আছে, কিন্তু model-এর আগে normalise + cluster কোনো agent করে না |
| I8 | **অনন্য** (এই tool-গুলোর মধ্যে) | শুধু তৃতীয় পক্ষের wrapper ও একটি arXiv paper |
| I9 | আংশিক অনন্য | provider-নিরপেক্ষ capsule, ভিন্ন vendor-এ স্বয়ংক্রিয় চালিয়ে যাওয়া |
| I10 | আংশিক অনন্য | প্রতি turn-এ কৌশল-ভিত্তিক সাশ্রয়ের আসল হিসাব |
| I11 | আংশিক অনন্য | Copilot যাচাই করে; canary + auto-rollback + বহু-উৎস truth check কারো নেই |
| I12 | handoff অংশ অনন্য নয়; বাকিটা আংশিক | step-journal crash-exact resume + E2E encryption |

**সারকথা:** ১২টার একটাও প্রতিযোগীদের কেউ হুবহু ship করে না। I7 ও I8 সবচেয়ে পরিষ্কারভাবে খোলা। I10 ও I12-এ প্রতিযোগিতা সবচেয়ে বেশি, তাই ওখানে পার্থক্যটা (কৌশল-ভিত্তিক সাশ্রয়, crash-exact resume, E2E) স্পষ্ট করে বলতে হবে।
