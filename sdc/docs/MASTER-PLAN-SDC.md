# SDC Master Plan (চূড়ান্ত)

## সারসংক্ষেপ

SDC-র লক্ষ্য: নিজের PC-র 8–9B লোকাল মডেল দিয়েই বাস্তব প্রোগ্রামিং কাজ শেষ করা—দ্রুত, কম token-এ, প্রতিটি ধাপের প্রমাণসহ। এই ডক আগের Master Plan v5 আর Gemini-র ১০টি সুপারিশকে যাচাই করে একটি একক, চূড়ান্ত প্ল্যানে মিলিয়েছে; আগের সব প্ল্যান ও Prompt বই এখন থেকে শুধু ইতিহাস।

**মূল মন্ত্র:** মডেলের বুদ্ধিতে অন্ধ ভরসা নয়; কোডের শৃঙ্খলা, কম্পাইলারের প্রমাণ, আর ব্যবহারকারীর চূড়ান্ত নিয়ন্ত্রণ।

v5-এর সব মূল সিদ্ধান্ত অক্ষুণ্ণ: Manual Pin বনাম Local Rotation চুক্তি, Step Runtime, Small Model Engine, "নিয়ম নয়, কোড", গতি একটি gate, আর বর্তমান UI-র সম্প্রসারণ। Gemini-র ১০টির মধ্যে ৬টি প্রায় হুবহু, ৩টি সংশোধিত রূপে, আর ১টি অংশত নেওয়া হয়েছে (বিস্তারিত পরের অংশে)। তার সাথে যোগ হয়েছে আরও ১২টি গ্যাপ, যা দুই ডকের কোনোটিতে ছিল না।

## এই ডক ও Prompt বই কীভাবে চলবে

1. এই ডক Markdown হিসেবে export করে রেপোতে রাখুন: `sdc/docs/MASTER-PLAN-SDC.md`।
2. সঙ্গী ডক "Prompt Book SDC" export করে রাখুন: `sdc/docs/PROMPT-BOOK-SDC.md`। প্রতিটি task-এর ইংরেজি prompt সেখানে, এই ডকের রোডম্যাপের একই ID ও ক্রমে।
3. প্রথম task P0-0 রেপোতে `sdc/docs/PLAN-STATUS.md` বানাবে: প্রতিটি task-এর ID, phase, নির্ভরতা, অবস্থা (todo / in-progress / blocked / done), সুইচের নাম, শেষ রিপোর্টের লিংক।
4. এরপর প্রতি session-এ শুধু Prompt 0 আর "NEXT"। SDC নিজে `PLAN-STATUS.md` পড়ে পরের যোগ্য task বেছে নেবে, gate মেনে কাজ করবে, test চালাবে, রিপোর্ট লিখবে, status আপডেট করবে।
5. SDC নিজে প্ল্যান বদলায় না, শুধু status। প্ল্যান বদলাতে হলে আগে এই ডক বদলে আবার export।

## Gemini-র সুপারিশ: যাচাই ও রায়

**রায়:** Gemini-র ১০টির মধ্যে ৬টি উৎসে সমর্থিত ও নেওয়া হলো, ৩টি সংশোধিত রূপে নেওয়া হলো কারণ মূল দাবি গবেষণার চেয়ে বেশি আশাবাদী, আর ১টি অংশত নেওয়া হলো কারণ Ollama-তে সরাসরি পাওয়া যায় না। প্রতিটি দাবি অন্তত দুটি আলাদা উৎসে মেলানো হয়েছে (নিচের Sources অংশে লিংক)।

| # | Gemini-র সুপারিশ | উৎসে কী পাওয়া গেল | রায় | এই প্ল্যানে চূড়ান্ত রূপ |
|---|---|---|---|---|
| 1 | Dynamic Re-planning (DAG) | ADaPT দেখিয়েছে, executor ব্যর্থ হলে শুধু সেই উপ-কাজ ভাঙলে সাফল্য উল্লেখযোগ্যভাবে বাড়ে; Atomic Task Graph পুরো re-plan না করে শুধু প্রভাবিত subgraph মেরামত করে | নেওয়া | প্রতিটি ধাপে কোডে লেখা pre/post-condition; শর্ত ভাঙলে শুধু নির্ভরশীল ধাপগুলো re-wire; ব্যর্থ ধাপ "as-needed" আরও ছোট ধাপে ভাঙা |
| 2 | Logprob Entropy দিয়ে ২–৩ বার জিজ্ঞেস করা বাদ | Ollama ≥0.12 logprobs দেয়, কিন্তু Anthropic API দেয় না, Gemini শুধু Vertex-এ। একটি গবেষণায় token entropy-র সাথে কোডের সঠিকতার সম্পর্ক দুর্বল, আর uncertainty-চালিত self-correction ১২টির ১১টি কনফিগে pass@1 কমিয়েছে; উপসংহার: এটা execution-ভিত্তিক যাচাইয়ের gate হিসেবে কাজের, বিকল্প হিসেবে নয়। অন্য গবেষণায় single-pass Top-K entropy সস্তা কিন্তু প্রতিযোগিতামূলক সংকেত | সংশোধিত | Entropy শুধু "সতর্ক সংকেত": বেশি হলে আগে গভীর Verify, Ripple Check বা আগাম escalation। পাস/ফেলের সিদ্ধান্ত সবসময় compile, LSP, test-এ। logprob না দেওয়া মডেলে কাঠামোগত সংকেত (loop, schema ভাঙা, Verify ফল) |
| 3 | LSP-Guided Repair | Serena ও OpenCode-সহ বড় agent-গুলো LSP diagnostics loop-এ ঢুকিয়েছে; LSP navigation token খরচও কমায় | নেওয়া | ভাষা অনুযায়ী language server pool, গরম রাখা, diagnostics + definition + references; LSP না থাকলে compiler/linter fallback |
| 4 | Handover Bridge Note | সরাসরি গবেষণা নেই; যুক্তিসঙ্গত, কারণ capsule ছোট হলে পরের ধাপের প্রসঙ্গ হারায় | নেওয়া, শক্ত করে | নোটের "কী বদলেছে" অংশ কোড diff থেকে বানায় (বদলানো symbol, নতুন signature); মডেল শুধু "কেন" ও "পরে কী মানতে হবে" লেখে; Source Quote Verifier দিয়ে যাচাই |
| 5 | AST-Anchored Patcher | এক গবেষণায়: content-anchored ফরম্যাট (search/replace, unified diff) মিল না পেলে নিরাপদে ফেল করে, কিন্তু function-নাম ধরে বদলানো একই নামের দুটি function থাকলে নীরবে ভুল জায়গায় বসায়; fuzzy matching-এর নীরব ভুল বহু-hunk-এ বাড়ে। Diff-XYZ: ছোট মডেল সব ফরম্যাটেই প্রায় সমান দুর্বল | সংশোধিত | পূর্ণ symbol path (`file::Class::method`) + পুরনো অংশের content hash দুটোই মিলতে হবে; শুধু exact বা whitespace-normalized মিল; fuzzy নয়; না মিললে fail-closed; ছোট function-এ পুরো symbol rewrite |
| 6 | Content-hash (Merkle) Cache | Cursor প্রতিটি ফাইলের hash দিয়ে Merkle tree বানায়, শুধু বদলানো শাখা আবার প্রক্রিয়া করে, chunk-এর content hash দিয়ে cache রাখে | নেওয়া | ফাইল-স্তরের সারাংশ ও AST blob-hash দিয়ে; কিন্তু Code Graph-এর আন্তঃফাইল সংযোগ নির্ভরশীল ফাইল বদলালে আবার হিসাব |
| 7 | CPU Micro-Reranker | FlashRank-এর ONNX cross-encoder CPU-তে দ্রুত চলে, কিন্তু এক benchmark-এ CPU-তে প্রতি call কয়েক সেকেন্ড; ms-marco মডেল ইংরেজি প্রশ্নোত্তরে প্রশিক্ষিত | সংশোধিত | প্রথম ছাঁকনি শূন্য-মডেল: BM25 + Code Graph + শিরোনাম। reranker শুধু বড় ডকে, CPU-তে, যাতে 9B VRAM থেকে না নামে; বাংলা ডকে বহুভাষিক মডেল; গতি bench-এ মেপে তবেই চালু |
| 8 | Prompt Lookup Decoding | llama.cpp server draft মডেল ছাড়াই ngram speculative decoding দেয় (`--spec-type`); Ollama-তে এটা দেখানো নেই। Qwen3.5-9B-এ MTP head আছে | অংশত | Ollama ডিফল্ট থাকে; ঐচ্ছিক llama-server backend যোগ, যেখানে ngram বা MTP speculative চালু; গতি bench পাস করলে তবেই। patch-only আউটপুট আগে থেকেই বেশিরভাগ লাভ দেয় |
| 9 | Runtime Smoke Check | টেস্ট না থাকা প্রজেক্টে compile পাস মানেই সঠিক নয়: যুক্তিসঙ্গত | নেওয়া, বাড়িয়ে | smoke-এর আগে বদলের আগের আচরণ ধরে characterization test; সব চালানো sandbox-এ |
| 10 | OS-Normalizer Middleware | যুক্তিসঙ্গত; Windows/Linux পথ ও shell ভুল সাধারণ | নেওয়া | মডেল নিরপেক্ষ tool call দেয়; Rust স্তর OS অনুযায়ী path, shell, line ending ঠিক করে |

Gemini-র ডকের কিছু দাবি অতিরঞ্জিত, তাই এই প্ল্যানে বাদ: "০ মিলিসেকেন্ডে দ্বিধা ধরা", "ক্যাশ ১০০% নির্ভুল", "প্ল্যান শতভাগ নিশ্ছিদ্র"। আর Gemini-র সংস্করণে v5-এর কয়েকটি জিনিস বাদ পড়েছিল (Cursor-তুলনা, Duel review, Shortcut Ledger, Chain Health, CLI-কে chain-এর সারি হিসেবে); এখানে সেগুলো ফেরত আনা হয়েছে।

## নতুন গ্যাপ: দুই ডকের কোনোটিতেই নেই

সবচেয়ে জরুরি নতুন গ্যাপ: Ollama prompt `num_ctx` ছাড়ালে কোনো error ছাড়াই prompt-এর শুরু ফেলে দেয়, আর OpenAI-সামঞ্জস্য `/v1` পথে `num_ctx` পাঠানোই যায় না। SDC এখন ঠিক ওই পথে Ollama চালায় (Doc 2), তাই 9B-এর "ভুলে যাওয়া" ও মনগড়া উত্তরের একটা অংশ আসলে নীরব কাটা হতে পারে। এটা সহ ১২টি গ্যাপ নিচে; শেষ কলামে রোডম্যাপের task।

| # | গ্যাপ | কেন জরুরি | সমাধান | Task |
|---|---|---|---|---|
| 1 | নীরব context কাটা | Ollama overflow-এ 200 OK দেয় আর prompt-এর শুরু (system prompt, tool, লক্ষ্য) ফেলে দেয়; `/v1` পথ options বাদ দেয় | Local Runtime Guard: native `/api/chat`, প্রতি request-এ স্পষ্ট `num_ctx`, পাঠানোর আগে token গোনা, ফেরত `prompt_eval_count` মেলানো; overflow হলে কখনো পাঠানো নয়, বরং ভাগ করা | P0-8 |
| 2 | 8GB VRAM-এর আসল হিসাব | `qwen3.5:9b` (Q4_K_M) নিজেই 6.6 GB; context-এর জন্য থাকে প্রায় ১–১.৫ GB। KV cache-এর ধরন (q8_0) পুরো server-এর env, প্রতি request-এর নয় | VRAM Arbiter: GPU-তে শুধু 9B; reranker, embedding, classifier সব CPU-তে; কার্যকর context মেপে বাজেট; KV সেটিং বদলাতে server restart লাগবে, তাই অনুমতি-কার্ড | P0-16 |
| 3 | ধাপ অনুযায়ী sampling ও thinking | Qwen3.5 ডিফল্টে thinking চালু; Ollama-র ডিফল্ট temperature 1 ও presence_penalty 1.5, অথচ নির্ভুল কোডিং-এ Qwen নিজেই temperature 0.6 ও presence_penalty 0 সুপারিশ করে | Step Profile: patch, format, JSON ধাপে thinking বন্ধ ও কম temperature; plan, debug ধাপে thinking চালু, thinking token-এর সীমাসহ; প্রতিটি প্রোফাইল bench-এ যাচাই | P1-4 |
| 4 | tool call লেখা হিসেবে বেরিয়ে আসা | Ollama-র একটি সংস্করণে `qwen3.5:9b` মাঝে মাঝে tool call চালানোর বদলে লেখা হিসেবে ছাপে, আর কাজ থেমে যায়; OpenAI-সামঞ্জস্য মোডে এটা আরও বেশি | Tool-call Rescue: লেখার ভেতরের `<tool_call>` কোড ধরে, schema মিলিয়ে চালায়; native API পছন্দ; কোন Ollama সংস্করণে সমস্যা, তা প্রোফাইলে | P1-6 |
| 5 | মডেলের পরিচয় শুধু নাম দিয়ে | একই tag-এর নিচে মডেল ফাইল বা Ollama সংস্করণ বদলালে আচরণ বদলায় | প্রোফাইলের চাবি: model digest + quantization + runtime সংস্করণ; বদলালে আবার probe, পুরনো Q3 ফল অস্থায়ী | P0-9 |
| 6 | এক GPU, অনেক প্রজেক্ট | SDC-র মূল শক্তি একসাথে অনেক প্রজেক্ট ও VPS; কিন্তু লোকাল GPU একটাই, আর মডেল বদলাবদি সময় খায় | Local GPU Scheduler: অগ্রাধিকারের সারি, ছোট পড়ার ধাপ একসাথে, একই মডেল লোড রেখে কাজ, কোন প্রজেক্ট অপেক্ষায় তা UI-তে | P3-15 |
| 7 | ধাপের অর্ধেক প্রয়োগ | বহু-ফাইল ধাপে একটি hunk ব্যর্থ হলে রেপো ভাঙা অবস্থায় থাকে | Step Transaction: ধাপের সব বদল shadow worktree-তে, Verify সবুজ হলে একবারে প্রয়োগ, নইলে সম্পূর্ণ rollback; Time Machine-এ এক বিন্দু | P0-14 |
| 8 | যাচাইয়ের কোড চালানো ঝুঁকিপূর্ণ | smoke check, test, মডেলের লেখা script, অজানা রেপো: সবই কোড চালায় | Sandbox Runner: প্রজেক্টের বাইরে লেখা নিষেধ, ডিফল্টে network বন্ধ, সময় ও মেমরি সীমা, তিন OS-এ | P0-14 |
| 9 | test-হীন প্রজেক্টে "আগে কেমন ছিল" জানা নেই | smoke শুধু crash ধরে, আচরণ বদল নয় | Characterization: বদলের আগে প্রভাবিত function-এর বর্তমান আউটপুট ধরে ছোট golden test, বদলের পরে মেলানো | P3-4 |
| 10 | bench-এর ফল শব্দে ভরা | temperature-এ একই task কখনো পাস কখনো ফেল; McNemar-এর জন্য জোড়া রান দরকার; পাবলিক task মডেলের training-এ থাকতে পারে | Bench নিয়ম: স্থির seed, প্রতি task একাধিক রান, জোড়া তুলনা, noise মেপে রিপোর্ট; কঠিন tier-এ ব্যবহারকারীর আসল প্রজেক্ট থেকে গোপন task | P0-11 |
| 11 | কোথায় সময় যাচ্ছে তা দেখা যায় না | Speed Gate, router-এর শেখা, Offload Limiter সবই মাপা ডেটা চায় | Step Trace: প্রতি ধাপে মডেল, token, TTFT, tok/s, Verify ফল, খরচ, কারণ লোকাল SQLite-এ; Workbench-এ ছোট timeline | P0-12 |
| 12 | আটকে থাকা কমান্ড | ঝুলে থাকা test বা build-এ ছোট মডেল অনন্তকাল অপেক্ষা করে বা ভুল সিদ্ধান্ত নেয় | Command Runner: প্রতিটি কমান্ডে সময়সীমা, অগ্রগতি stream, "কোনো আউটপুট নেই" ধরা, kill, আর কারণসহ ফল মডেলকে | P0-13 |

## বারোটি মূল সিদ্ধান্ত

প্রতিটি নতুন ফিচারকে এই বারোটি সিদ্ধান্ত মানতে হবে; Prompt Book-এর Prompt 0-তে এগুলো ইংরেজিতে আছে।

1. **বাছা মডেলই চালক।** ক্লাউড/API/CLI মডেল বাছলে Manual Pin, কোনো স্বয়ংক্রিয় বদল নয়। লোকাল মডেল বাছলে তবেই Local Rotation, ব্যবহারকারীর সাজানো ক্রমে, ধাপ ধরে, তারপর আবার লোকালে।
2. **Step Runtime।** বড় কাজ ছোট ধাপের DAG; প্রতিটি ধাপে কোডে লেখা পূর্বশর্ত ও ফলশর্ত, যাচাই, journal; শর্ত ভাঙলে শুধু প্রভাবিত ধাপ re-plan। এক ধাপের কাজ fast path-এ।
3. **Small Model Engine।** মডেল একবারে শুধু ছোট অংশ দেখে; খোঁজা, ছাঁকা, মনে রাখা, উদ্ধৃতি মেলানো SDC-র কোড করে।
4. **কম্পাইলারই বিচারক।** LSP, compiler, test আর characterization সিদ্ধান্ত দেয়; মডেলের আত্মবিশ্বাস (entropy) শুধু কোথায় বেশি সতর্ক হতে হবে তা জানায়।
5. **নিরাপদ প্রয়োগ।** লাইন নম্বর নয়; symbol path + content hash দিয়ে patch; না মিললে fail-closed; প্রতিটি ধাপ একটি transaction, পুরোটা বা কিছুই নয়।
6. **context কখনো নীরবে কাটবে না।** প্রতিটি call-এর আগে token গোনা; বাজেট ছাড়ালে ভাগ করা, পাঠানো নয়।
7. **মান আগে, সাশ্রয় পরে (Q3)।** কোনো কৌশল কোনো মডেলে pass rate ১ পয়েন্টের বেশি কমালে সেখানে বন্ধ।
8. **নিয়ম নয়, কোড।** Pin Guard, Context Guard, Verify Gate, Quote Verifier, Cut-off Guard, Hallucination Guard সবই কোডে।
9. **গতি একটি gate।** সাধারণ কাজ baseline-এর চেয়ে ৫%-এর বেশি ধীর হলে release আটকে যায়; যে প্রশ্ন Code Graph বা git দিয়ে উত্তর হয়, সেখানে মডেল call শূন্য।
10. **লোকাল-প্রথম ও নিরাপদ চালানো।** কোড ও ডেটা ব্যবহারকারীর PC-তে; যাচাইয়ের সব কোড sandbox-এ; cloud শুধু Local Rotation-এ, অনুমতি ও সীমার ভেতরে।
11. **UI নতুন নয়, সম্প্রসারণ।** একই component, token, রং, বাংলা/ইংরেজি string।
12. **সৎ সীমা।** 9B বড় মডেলের সমান বুদ্ধিমান হবে না। SDC কাজকে 9B-এর মাপে কাটে, কোড দিয়ে ভুল ধরে, আর যে কয়েকটি ধাপ সত্যিই সাধ্যের বাইরে, শুধু সেগুলো chain-এ পাঠায়; chain না থাকলে সৎভাবে থামে, ভুয়া "শেষ" বলে না।

## মডেল-নির্বাচন চুক্তি

নিয়ম এক লাইনে: chat box-এ যে মডেল বাছা, সিদ্ধান্তও তার। ক্লাউড মডেল বাছলে SDC কখনো মডেল বদলায় না; লোকাল মডেল বাছলে তবেই ধাপে ধাপে ব্যবহারকারীর সাজানো ক্রমে বদলায় আর পরের ধাপে আবার লোকালে ফেরে।

| chat box-এ যা বাছা | মোড | কী হয় | স্বয়ংক্রিয় বদল |
|---|---|---|---|
| Claude, GPT, DeepSeek, Gemini বা যেকোনো API/CLI মডেল | Manual Pin (ডিফল্ট) | পুরো কাজ, প্রতিটি ধাপ, planner, sub-agent সব এই মডেলে | কখনো নয় |
| লোকাল মডেল + Rotation Chain-এ অন্তত একটি চালু সারি | Local Rotation | প্রতিটি ধাপ আগে লোকালে; না পারলে chain #1, তারপর #2…; পরের ধাপ আবার লোকালে | শুধু ব্যর্থ ধাপে, chain-এর ক্রমে |
| লোকাল মডেল + chain খালি/বন্ধ, অথবা `privacy=local-only` | Local-only | সব লোকালে; সাধ্যের বাইরের ধাপে সৎভাবে থেমে কারণ ও বিকল্প জানায় | কখনো নয় |

### Manual Pin-এ কঠোরভাবে বন্ধ

- Router (W6), cascade (I3), failover (I9), Verified Ladder, Planner–Executor, bandit সব বন্ধ।
- প্রতিটি model call একটি কেন্দ্রীয় `ModelGate` দিয়ে যায়; `pinned_model_id`-এর সাথে না মিললে call আটকায়, test-এ `PinViolationError`।
- provider বন্ধ বা rate limit হলে SDC অন্য মডেলে যায় না; journal সেভ করে কার্ড দেখায়: আবার চেষ্টা · অপেক্ষা · এই কাজটা অন্য মডেলে চালাও (ব্যবহারকারী বাছবে)।
- ব্যতিক্রম শুধু ব্যবহারকারীর স্পষ্ট সেটিং: `/research`-এর Final answer model, Verify-এর Duel reviewer ইত্যাদি; রসিদে আলাদা লাইনে।
- SME-র কৌশল (ভাগ করে পড়া, Working Memory, Context Guard, Cut-off Guard) Manual Pin-এও চলে, কারণ এগুলো মডেল বদলায় না।

### Rotation Chain (Settings → Agent → Local Rotation)

উপরের সারি মানে বেশি অগ্রাধিকার; ↑ ↓ বোতাম ও ডেস্কটপে টেনে সরানো, চালু/বন্ধ সুইচ, মুছে ফেলা।

| সারির সেটিং | কী করে | প্রস্তাবিত ডিফল্ট |
|---|---|---|
| মডেল | যেকোনো সংযুক্ত API বা অফিসিয়াল CLI (Claude Code, Codex CLI) | খালি; SDC প্রস্তাব দেয়, ব্যবহারকারী অনুমোদন করে |
| চালু/বন্ধ | সারি না মুছে সাময়িক বাদ | চালু |
| অনুমোদিত ধাপ | all, অথবা plan / edit / review / research / debug | all |
| প্রতি ধাপের খরচ-সীমা | আনুমানিক খরচ বেশি হলে এই সারি বাদ দিয়ে পরেরটায় | $0.50 |
| মেরামত চেষ্টা | Verify ফেল হলে এই সারিতে লক্ষ্যভেদী মেরামত কতবার | 1 |

পুরো chain-এর জন্য: দৈনিক ও মাসিক খরচ-সীমা (প্রস্তাব: দৈনিক $2.00); "প্রতিটি cloud-ধাপের আগে জিজ্ঞেস করো" (Ask/Pro-তে ডিফল্ট চালু, Auto-তে বাজেটের ভেতরে স্বয়ংক্রিয়); Offload Limiter (এক কাজে সর্বোচ্চ ৫টি cloud-ধাপ বা মোট ধাপের অর্ধেক; তার বেশি হলে থেমে জিজ্ঞেস)।

chain-এর কোনো সারি তখন অনুপলব্ধ হলে (key নেই, rate limit, CLI লগইন নেই, খরচ-সীমা শেষ, ধাপের ধরন অনুমোদিত নয়, capsule তার context-এ ধরে না) খরচ ছাড়াই সেটি বাদ দিয়ে পরের সারি; কারণ লগে ও chip-এ দেখায়। SDC chain-এর ক্রম কখনো নিজে বদলায় না; প্রয়োজন মনে হলে সাপ্তাহিক রিপোর্টে এক-ক্লিকের প্রস্তাব দেয়।

## Step Runtime

কাজ চলবে একটি সুশৃঙ্খল state machine-এ: এক ধাপের কাজ সরাসরি, বড় কাজ ছোট ধাপের DAG-এ, প্রতিটি ধাপ একটি transaction। বর্তমান agent loop-ই Step Executor-এর ভিত্তি; নতুন অংশ আলাদা মডিউলে (`runtime/`, `sme/`, `precision/`, `rotation/`), সুইচ বন্ধ থাকলে হুবহু আগের আচরণ।

### প্রবাহ

1. **Fast-path check।** প্রশ্নের উত্তর Code Graph বা git দিয়ে হলে মডেল call শূন্য। এক ধাপের ছোট বদল planner ছাড়া সরাসরি Step Executor-এ।
2. **Intent ও Decomposer।** ~৩০০ token-এর কাজের কার্ড; Code Graph দেখে ১–২টি function-এর ধাপে DAG; প্রতিটি ধাপে কোডে লেখা পূর্বশর্ত (কোন symbol, কোন signature ধরে নেওয়া) ও ফলশর্ত।
3. **প্রতিটি ধাপে:**
    1. Context Assembler: Working Memory থেকে শুধু এই ধাপের নোট ও কোড; Context Guard token গোনে।
    2. Step Router: Manual Pin হলে বাছা মডেল; Local Rotation-এ আগে লোকাল।
    3. ModelGate: অনুমোদিত মডেল ছাড়া যেকোনো call আটকায়।
    4. Step Profile: ধাপের ধরন অনুযায়ী sampling ও thinking।
    5. Patch in Transaction: shadow worktree-তে symbol-anchored patch; Cut-off Guard; OS-Normalizer।
    6. Precision Verify: LSP + compile + প্রভাবিত test/characterization + Hallucination ও secret scan, সমান্তরালে, sandbox-এ।
4. **সবুজ হলে:** transaction commit, journal, ফলশর্ত পরীক্ষা। পরের কোনো ধাপের পূর্বশর্ত ভাঙলে শুধু সেই নির্ভরশীল subgraph re-wire (সম্পূর্ণ re-plan নয়)।
5. **লাল হলে:** LSP diagnostics ও definition-সহ ১–২ বার লক্ষ্যভেদী মেরামত; তবুও লাল হলে ধাপটিকে আরও ছোট উপ-ধাপে ভাঙার একবার চেষ্টা (as-needed decomposition)।
6. **তবুও ব্যর্থ:** Manual Pin বা Local-only হলে সৎভাবে থামা (কী শেষ, কী যাচাই হলো, কী বাকি); Local Rotation হলে শুধু এই ধাপের capsule chain #1-এ।
7. **শেষে:** পুরো প্রভাবিত test suite, Proof-of-Work ব্যাজ, recipe ও lesson জমা।

journal প্রতিটি ধাপ লেখে, তাই crash, sleep বা মডেল বদলেও কাজ হারায় না; VPS-এর turn-ও একই journal পায়।

## Local Rotation: কখন chain-এ, কীভাবে ফেরা

পুরো কাজ কখনো cloud-এ যায় না; যায় শুধু সেই একটি ধাপ, যা লোকালে সত্যিই হচ্ছে না, আর যায় ব্যবহারকারীর সাজানো ক্রমে। গবেষণাও এই পথ সমর্থন করে: সহজ কাজ ছোট মডেলে আর কঠিনগুলো শক্তিশালীতে পাঠিয়ে মান না কমিয়ে খরচ অনেক কমানো যায় (RouteLLM, FrugalGPT)।

| সংকেত | কোড কীভাবে বোঝে | ধরন |
|---|---|---|
| ধাপ বাজেটে ধরছে না | ভাগ করার পরও একসাথে ৩টির বেশি ফাইলের জটিল আন্তঃসম্পর্ক বদলাতে হচ্ছে, বা capsule কার্যকর context ছাড়ায় | আগাম বিচার |
| অতীত ব্যর্থতা | Step Trace বলছে এই ধরনের ধাপে এই মডেল-digest-এর সাফল্য ৩০%-এর কম (পর্যাপ্ত নমুনায়) | আগাম বিচার |
| মেরামতের পরও Verify ফেল | LSP-সহ ২ বার মেরামত ও একবার as-needed ভাঙার পরও লাল | ফল দেখে |
| দ্বিধা বা লুপ | উচ্চ entropy-র সাথে লুপ, একই ভুল বারবার, বা schema ভাঙা; entropy একা কখনো যথেষ্ট নয় | ফল দেখে |

**কখনো chain-এ যাবে না (Local Watchdog):** লোকাল server বন্ধ, মডেল unload, VRAM শেষ হয়ে CPU-তে নেমে যাওয়া, dependency বা compiler নেই, network নেই, অনুমতির অপেক্ষা, বা `secret_paths`-এর ফাইল লাগে এমন ধাপ। এগুলো পরিবেশের সমস্যা; SDC থেমে জানায়।

### chain-এর ভেতরে ওঠা ও ফেরা

1. ধাপ প্রথম চালু ও উপযুক্ত সারিতে যায় (key/লগইন আছে, ধাপের ধরন অনুমোদিত, খরচ-সীমার ভেতরে, capsule তার context-এ ধরে)।
2. ফল একই Verify gate পার হতে হয়; ফেল হলে ওই সারিতে নির্ধারিত সংখ্যক মেরামত।
3. তবুও ফেল হলে পরের সারি, capsule ও শেষ ব্যর্থতাসহ; কখনো শূন্য থেকে নয়।
4. সব সারি ব্যর্থ হলে সৎভাবে থামা ও পরামর্শ (ছোট scope, chain-এ অন্য মডেল যোগ)।
5. যে সারিতেই সফল হোক, পরের ধাপ আবার লোকাল থেকে।

### Capsule ও Handover

- **Capsule:** শুধু Intent কার্ড, এই ধাপের লক্ষ্য, লক্ষ্য symbol-এর কোড, দরকারি যাচাই-করা নোট, হুবহু Verify error। secret redaction চালু; `secret_paths` কখনো নয়; পাঠানোর আগে Ask/Pro মোডে capsule-এর সারাংশ ও আনুমানিক খরচ কার্ডে; প্রতিটি পাঠানো `.sdc/audit/sent.jsonl`-এ।
- **Capsule Pre-warm:** লোকাল প্রথম মেরামতে ব্যর্থ হলেই capsule পটভূমিতে তৈরি, যাতে সিদ্ধান্তের পর দেরি না হয়।
- **Handover Note:** chain-এর মডেলের সমাধানের পর SDC কোড দিয়ে diff থেকে বের করে কী বদলেছে (symbol, signature, নতুন ফাইল); মডেল লেখে ≤৮০ token-এ "কেন" ও "পরের ধাপে কী মানতে হবে"; Quote Verifier মিলিয়ে Working Memory-তে রাখে। পরের লোকাল ধাপ এটা প্রম্পটের শুরুতে পায়।
- **Recipe:** সমাধানের পথ `knowledge_core`-এর প্রমাণ-দরজা পার হয়ে recipe হিসেবে জমে; পরের বার একই ধরনের ধাপে লোকাল সেটা পায়।
- **CLI সারি:** ব্যবহারকারীর নিজের Claude Code বা Codex CLI; capsule prompt হিসেবে আলাদা worktree-তে যায়, ফল diff হিসেবে ফেরে, একই Verify।
- **প্রস্তাবিত chain:** প্রথমবার লোকাল বাছলে SDC bench-ফল দেখে প্রস্তাব দেয় (#1 কঠিন ধাপে ভালো করা সবচেয়ে সস্তা মডেল, #2 সবচেয়ে বেশি pass rate, #3 ব্যবহারকারীর CLI); নীরবে কখনো চালু হয় না।

## Small Model Engine

ছোট মডেলের তিনটি প্রমাণিত সমস্যা: কার্যকর context দাবির চেয়ে অনেক ছোট (RULER), লম্বা input-এর মাঝখানের তথ্য হারায় (Lost in the Middle), আর বড় আউটপুট মাঝপথে কাটে। 8GB VRAM-এ `qwen3.5:9b` কাগজে 262k context দেখালেও বাস্তবে ১০–৩০ হাজার token-এর বাজেটে কাজ করতে হবে; আসল সংখ্যা Probe মাপবে। SME মডেল বদলায় না, তাই Manual Pin-এও চলে।

| # | উপাদান | কী করে | কোন সমস্যা ঠেকায় |
|---|---|---|---|
| 1 | Local Runtime Guard | native API, স্পষ্ট `num_ctx`, token আগে গোনা, `prompt_eval_count` মেলানো | নীরব context কাটা |
| 2 | Model Profile ও Probe | digest ধরে কার্যকর context, আউটপুট সীমা, tool-call নির্ভরতা, tok/s, TTFT, logprob আছে কি না মাপে; S/M/L শ্রেণি ও প্রতি ধাপের বাজেট | বিজ্ঞাপনের সংখ্যায় ভরসা |
| 3 | Hardware Auto-Tuner ও VRAM Arbiter | Flash Attention + KV q8_0 (প্রায় অর্ধেক KV মেমরি), সর্বোচ্চ নিরাপদ `num_ctx`, কাজ চলাকালীন `keep_alive`; GPU-তে শুধু মূল মডেল; প্রস্তাব, অনুমতি নিয়ে প্রয়োগ | VRAM overflow, CPU-তে নেমে যাওয়া, বারবার লোড |
| 4 | Step Profile | ধাপ অনুযায়ী temperature, thinking চালু/বন্ধ, thinking token-এর সীমা | অকারণ লম্বা চিন্তা, এলোমেলো patch |
| 5 | Prompt Diet ও Edge Pinning | ছোট system prompt, প্রতি ধাপে ≤৫টি tool schema, মূল লক্ষ্য শুরুতে ও শেষে, স্থির prefix | নির্দেশ ভোলা, prefix cache মিস |
| 6 | বাঁধা আউটপুট ও Tool-call Rescue | JSON schema/grammar; ভাঙা JSON কোডে মেরামত; লেখা হয়ে বেরোনো tool call ধরে চালানো | ভুল ফরম্যাট, থেমে যাওয়া কাজ |
| 7 | Working Memory | context-এর বাইরের খাতা (SQLite): কাজের কার্ড, প্ল্যান, উৎসসহ নোট, সিদ্ধান্ত, Handover Note; প্রতি ধাপে শুধু দরকারিটা | ভুলে যাওয়া, context ভরা |
| 8 | Retrieval Cascade | প্রথমে শূন্য-মডেল: Code Graph, BM25, শিরোনাম; তারপর ঐচ্ছিক CPU reranker; শেষে 9B শুধু ছাঁকা অংশ পড়ে | সব কিছু 9B দিয়ে পড়ার সময় |
| 9 | Chunked Reader | AST (function/class) ও Markdown শিরোনাম ধরে ভাগ; আগে চোখ বুলানো, তারপর গভীরে; content-hash cache | মাঝখানের তথ্য হারানো |
| 10 | Source Quote Verifier | নোটের প্রতিটি উদ্ধৃতি আসল ফাইলে খোঁজে; না মিললে নোট বাতিল ও আবার পড়া | মনগড়া সারাংশ |
| 11 | Symbol-Anchored Patcher ও Cut-off Guard | symbol path + content hash; exact/whitespace-normalized মিল; কাটা আউটপুটে শেষ বৈধ AST থেকে বাকিটা চাওয়া; নতুন বড় ফাইলে আগে কাঠামো | ভুল জায়গায় patch, অর্ধেক কোড |
| 12 | Confidence Signals | logprob থাকলে Top-K entropy; সাথে লুপ, একই ভুল, schema ভাঙা; শুধু সতর্কতা বাড়ায়, পাস দেয় না | অন্ধ আত্মবিশ্বাস |
| 13 | Bangla/Banglish → English Spec Brief | ছোট নিখুঁত ইংরেজি স্পেক মডেলকে; ব্যবহারকারীকে নিশ্চিতকরণ তার ভাষায় | ছোট মডেলের দুর্বল বাংলা বোঝা |

### উদাহরণ: "৩০ পাতার ডক আর প্রজেক্ট পড়ে login-এ two-factor যোগ করো" (লোকাল মডেল)

1. Intent ~৩০০ token-এর কার্ড বানায়, যা প্রতিটি ধাপে থাকে।
2. ডক শিরোনাম ধরে ভাগ; BM25 ও reranker CPU-তে প্রাসঙ্গিক অংশ বাছে; 9B শুধু সেগুলো পড়ে উৎসসহ নোট লেখে, কোড উদ্ধৃতি মেলায়।
3. Code Graph login কোথায় তা মডেল ছাড়া বের করে; শুধু সেই function পড়া।
4. ছোট সমাধান-নকশা Working Memory-তে; কাজ ভাগ: database, model, route, UI, test।
5. প্রতিটি ধাপ লোকালে transaction-এ; Verify লাল হলে LSP-সহ মেরামত; নকশার কোনো কঠিন সিদ্ধান্ত না পারলে শুধু সেটা chain-এ, তারপর আবার লোকালে।
6. শেষে পুরো প্রভাবিত test, Proof-of-Work ব্যাজ, recipe জমা।

একই কাজ Claude বেছে করলে: SME শুধু দরকার হলে বড় ডক ভাগ করে পড়ে, কিন্তু প্রতিটি ধাপ Claude-এই চলে।

## গতি ও Speed Gate

ছোট ধাপে কাজ করলে মোট সময় বাড়ার ঝুঁকি আছে, তাই গতি একটি gate: এক ধাপের সাধারণ কাজ baseline-এর চেয়ে ৫%-এর বেশি ধীর হলে release আটকে যায়। Pin Guard ও Context Guard শুধু তুলনা ও গণনা, তাই খরচ মাইক্রো থেকে মিলিসেকেন্ডে।

1. **শূন্য-মডেল উত্তর।** "X কোথায়", "কোন test এই ফাইল ছোঁয়", "git-এ কী বদলেছে": Code Graph, search, git থেকে মিলিসেকেন্ডে (ফাইলনাম-সার্চ এখনই ২.৫–৩.২ ms)।
2. **Fast path।** এক ধাপের কাজ planner বা SME ছাড়া, ঠিক এখনকার মতো।
3. **স্থির prefix।** System + RepoMap + Working Memory header সবসময় একই ক্রমে, যাতে Ollama/llama.cpp ও cloud API-র prefix cache হিট করে।
4. **Content-hash cache।** ফাইল ও chunk-এর hash দিয়ে সারাংশ, AST, embedding; branch বদলালে শুধু বদলানো ফাইল আবার।
5. **আগে থেকে আনা।** ধাপ ২ চলার সময় Code Graph দেখে ধাপ ৩-এর ফাইল ও AST মেমরিতে।
6. **সমান্তরাল যাচাই।** lint, type, test, scan একসাথে; LSP server গরম রাখা।
7. **ছোট আউটপুট, ছোট input।** শুধু patch; Tool Output Compressor test/log থেকে শুধু ফেল করা trace ও মূল লাইন দেয়।
8. **Step Profile।** patch ধাপে thinking বন্ধ, তাই অপ্রয়োজনীয় চিন্তার token নেই।
9. **মডেল GPU-তে রাখা।** কাজ চলাকালীন `keep_alive`; VRAM Arbiter অন্য কিছু GPU-তে বসতে দেয় না।
10. **ঐচ্ছিক speculative decoding।** llama-server backend-এ ngram বা MTP; গতি bench পাস করলে তবেই চালু।
11. **Capsule pre-warm ও latency probe।** chain-এর জন্য capsule আগে তৈরি; নতুন মডেলে region ও TTFT মাপা (আগে ভুল region-এ প্রথম token ১২–২০ সেকেন্ড, ঠিক region-এ ১.৫ সেকেন্ড ছিল)।
12. **অনুমতির অপেক্ষায় আটকে না থাকা।** ফোনে ও ডেস্কটপে নোটিফিকেশন, নীতি অনুযায়ী সময়সীমা (একটি turn রাতে ৭ ঘণ্টা ৪০ মিনিট অপেক্ষায় ছিল)।
13. **অগ্রগতি চোখে দেখা।** "পড়ছি ১২/৪০ · ধাপ ২/৫: যাচাই চলছে"।

| মাপ | কীভাবে | পাসের শর্ত |
|---|---|---|
| এক ধাপের সাধারণ কাজ | median ও p90, প্রতিটি মডেলে, আগের সংস্করণের সাথে | baseline-এর চেয়ে ৫%-এর বেশি ধীর নয় |
| শূন্য-মডেল প্রশ্ন | মডেল-call সংখ্যা ও সময় | মডেল-call ০ |
| প্রথম দৃশ্যমান সাড়া | অনুরোধ থেকে প্রথম অগ্রগতি | রিপোর্টে; লক্ষ্য প্রায় তাৎক্ষণিক |
| 9B-এ বড় কাজ | সফল হওয়া পর্যন্ত মোট সময় | রিপোর্টে; সাধারণ 9B যেখানে ব্যর্থ, সেখানে সফল হওয়াই উন্নতি |
| chain-এ যাওয়া ধাপ | সিদ্ধান্ত থেকে ফল পর্যন্ত সময় ও খরচ | রসিদে আলাদা |

## Precision Engine ও Verify gate

লোকাল 9B হোক বা chain-এর বড় মডেল, প্রতিটি কোড-বদল একই দরজাগুলো পার হয়; "কম লেখো" নিয়ম মডেল উপেক্ষা করতে পারে, তাই একই লক্ষ্য কোড দিয়ে বাধ্য।

1. **শ্রেণি ও Diff Budget।** কাজের ধরন অনুযায়ী স্বাভাবিক বদলের আকার; ছোট bug fix-এ ২০০ লাইন rewrite আটকায়।
2. **Need ও Reuse Finder।** বিদ্যমান ফিচার বা config-এ কাজ হয় কি না; Code Graph, ইনস্টল করা লাইব্রেরি ও stdlib-এ কাছাকাছি function খুঁজে ছোট আকারে মডেলকে জানায়।
3. **Hallucination ও Slopsquatting Guard।** নতুন import বা প্যাকেজ সত্যিই manifest, lockfile বা ডিস্কে আছে কি না, চালানোর আগে; গবেষণায় সুপারিশ করা প্যাকেজের ১৯.৭% আসলে নেই।
4. **LSP Diagnostics।** বদলানো ফাইলের error, warning, আর ভুল symbol-এর সঠিক definition ও signature।
5. **Test Impact।** Code Graph দিয়ে শুধু প্রভাবিত test; ভুল বাদ পড়া test শূন্য রাখতে সন্দেহে বেশি চালানো।
6. **Characterization ও Smoke।** test না থাকলে বদলের আগে বর্তমান আচরণ ধরে golden test; তারপর import/চালানো smoke; সব sandbox-এ।
7. **Quality Floor ও AST Duplicate।** খালি `catch {}`, মুছে ফেলা test, ডুপ্লিকেট function, অব্যবহৃত কোড থাকলে লাল।
8. **Security।** secret, SAST, dependency scan (বিদ্যমান Verify-এ আছে, এখন প্রতি ধাপে)।
9. **Ripple Check।** signature বদলালে আর কোথায় call ভাঙল তার তালিকা; প্রতিটি জায়গা আলাদা ছোট ধাপ।
10. **Spec-to-Test।** Intent Contract-এর শর্ত থেকে কাজের আগেই test, যাতে "ঠিক হয়েছে" মেশিনে যাচাই হয়।
11. **সিদ্ধান্ত।** সব সবুজ হলে transaction commit; লাল হলে Failure Taxonomy অনুযায়ী শুধু ব্যর্থ অংশ দিয়ে মেরামত।
12. **প্রমাণ।** Proof-of-Work ব্যাজ: "✓ ৪টি test পাস · LSP error ০ · ১৪ লাইন · ১টি বিদ্যমান function পুনর্ব্যবহৃত · qwen3.5:9b (ধাপ ১, ২, ৪) + deepseek-chat (ধাপ ৩)", সাথে Shortcut Ledger।

SDC নিজের কোড বানানোর কাজও নিজের Verify দিয়ে যাচাই করবে, আর বড় বদলে Duel review, যদি ব্যবহারকারী Duel-এর মডেল বেছে রাখে।

## Token সাশ্রয় ও মোড-নিয়ম

এখন পর্যন্ত মাপা: DeepSeek-এ সব কৌশল একসাথে খরচ −১৭%, input token −২১%, tool call −৩৫% (Doc 2)। প্রতিটি কৌশল প্রতিটি মডেলে আলাদা Q3 পাস করলে তবেই চালু; যেসব কৌশল মডেল বদলায়, সেগুলো শুধু Local Rotation-এ।

| কৌশল | কী করে | এখনকার অবস্থা | মোড | চালুর শর্ত |
|---|---|---|---|---|
| tool_pruning (W5) | ধাপ অনুযায়ী ≤৫টি tool schema | DeepSeek-এ −১৬.১% | উভয় | প্রতি মডেলে Q3 |
| repomap (W4) → Code Graph | AST-ভিত্তিক প্রজেক্ট মানচিত্র | DeepSeek-এ −৭.৯% | উভয় | Q3 |
| recipes, lessons, memory, knowledge (L0–L3) | আগের সফল পথ ও শিক্ষা | পুনরাবৃত্ত কাজে −২০% | উভয়; chain-এর সমাধান থেকেও recipe | knowledge_core যাচাই |
| stable_prefix, ledger_diff, skeleton | cache-বান্ধব শুরু, বদলানো ফাইলের diff, রূপরেখা | শুধু DeepSeek-এ বাতিল | উভয় | অন্য মডেলে আবার Q3 |
| interlingua (I8) → Spec Brief | ইংরেজি স্পেক | DeepSeek-এ সাশ্রয় নেই | উভয় | ছোট মডেলে নির্ভুলতার জন্য |
| Tool Output Compressor | log/test-এর শুধু দরকারি অংশ | নতুন | উভয়; ছোট মডেলে বাধ্যতামূলক | তথ্য না হারানোর test |
| Prompt Diet ও Context Budgeter | প্রতিটি অংশের token-সীমা | নতুন | উভয় | ছোট-মডেল bench |
| Step Profile (thinking বন্ধ patch-এ) | অপ্রয়োজনীয় চিন্তার token বাদ | নতুন | উভয় | Q3 |
| Reuse Finder ও Diff Budget | কম নতুন কোড | নতুন | উভয় | Q3 |
| Chunked Reader, Retrieval Cascade, hash cache | বড় input কখনো একবারে নয় | নতুন | উভয় | উদ্ধৃতি যাচাই ১০০% |
| Answer Cache ও turn-এর ভেতরের cache | একই প্রশ্ন বা পড়া আবার নয় | নতুন | উভয় | — |
| Semantic index | কম ফাইল খুলে সঠিক জায়গা | নতুন | উভয় | recall মাপা |
| Capsule Minimizer | chain-এ শুধু দরকারি অংশ | নতুন | শুধু Local Rotation | capsule token ও pass rate |
| Router, cascade, failover, Planner–Executor | সহজ ধাপ লোকালে, কঠিন chain-এ | কোড আছে, চলে না | শুধু Local Rotation; Manual Pin-এ নিষিদ্ধ | Pin Guard ও Gate 2 |
| Smart Router (bandit L4) | কোন ধাপে লোকাল এড়িয়ে সরাসরি chain #1 | বন্ধ | শুধু Local Rotation; ক্রম কখনো বদলায় না | পর্যাপ্ত replay ডেটা |
| fleet (I7) | অনেক server-এর একই আউটপুট একবার | synthetic-এ −৮০.৪% | উভয় | আসল Docker পরীক্ষা |

## UI, ডেটা, নিরাপত্তা ও Bug Shield

কোনো নতুন নকশা-ভাষা নয়: আগে কোড থেকে design token ও component-এর তালিকা `docs/UI-GUIDE.md`-এ, প্রতিটি screen-এর visual regression ও axe test; নতুন অংশ বিদ্যমান component, রং, font ও দুই ভাষার string ফাইল দিয়ে, ফোনের ছোট screen-এও।

### নতুন UI অংশ

| অংশ | কোথায় | কোন বিদ্যমান অংশের ধাঁচে |
|---|---|---|
| মোড ব্যাজ: Pinned · Local Rotation (3) · Local-only | prompt এলাকা | ModelSelector, TrustChip |
| Rotation Chain তালিকা (↑ ↓, টেনে সরানো, চালু/বন্ধ, সারির সেটিং) | Settings → Agent → Local Rotation | PolicyEditor-এর তালিকা ও toggle |
| প্রস্তাবিত chain কার্ড, হার্ডওয়্যার সেটিং প্রস্তাব | Provider Hub | বিদ্যমান doctor flow |
| ধাপের মডেল-chip ও কারণ | PlanCard | TrustChip |
| cloud-ধাপের অনুমতি, Offload Limiter, provider-সমস্যার কার্ড | prompt এলাকা, ফোনে নোটিফিকেশন | QuestionCard |
| মডেল শ্রেণি (S/M/L), probe অবস্থা, কার্যকর context | ModelSelector, Provider Hub | live/cached/bundled লেবেল |
| পড়ার অগ্রগতি ও ধাপের তালিকা | prompt এলাকা | ResearchCard |
| Working Memory ও Step Trace | ডান প্যানেলে নতুন tab | Time Machine, ProofTab |
| Diff Review (hunk ধরে Approve/Reject/বদলাতে বলো, কে লিখেছে) | ডান প্যানেলের Verify ও Preview | বিদ্যমান diff ও Verify |
| @ picker (@file, @symbol, @docs, @git, @server, token-আকারসহ) | prompt এলাকা | slash command মেনু |
| Select & Instruct | Preview | QuestionCard-ধাঁচের ইনপুট |
| Proof-of-Work ব্যাজ, budget gauge | turn-এর শেষে, CostCenter | TrustChip, CostMeter |
| GPU সারি (কোন প্রজেক্ট অপেক্ষায়), Plan Tracker | Workbench | বিদ্যমান parallel board |
| সাপ্তাহিক বাংলা রিপোর্ট | Workbench-এর Inbox | automation Inbox |

### ডেটা কোথায় থাকবে (লোকাল-প্রথম)

| ডেটা | কোথায় |
|---|---|
| প্রজেক্টের জ্ঞান: recipes, lessons, rules, hooks, policy, Shortcut Ledger | প্রজেক্টের `.sdc/` (চাইলে git দিয়ে টিমে ভাগ) |
| chain-এ পাঠানো সবকিছুর লগ | `.sdc/audit/sent.jsonl` |
| Rotation Chain ও মোড | `~/.sdc/rotation.toml`; প্রজেক্ট `.sdc/policy.toml`-এ local-only বাধ্য করতে পারে |
| Working Memory, journal, model profile (digest-চাবি), Step Trace, router পরিসংখ্যান | `~/.sdc/` ও বিদ্যমান SQLite |
| Code Graph, semantic index, সারাংশ (content-hash) | `~/.sdc/cache/`, কখনো git-এ নয়, আবার বানানো যায়, আকার-সীমাসহ |
| প্ল্যান ও অগ্রগতি | রেপোর `sdc/docs/MASTER-PLAN-SDC.md`, `PROMPT-BOOK-SDC.md`, `PLAN-STATUS.md` |
| API key | OS keychain (Linux-এ Secret Service); প্লেইনটেক্সট নিষিদ্ধ |
| অন্য PC-র কপি | Cloudflare relay-তে end-to-end এনক্রিপ্টেড; চাবি শুধু ব্যবহারকারীর ডিভাইসে |

### নিরাপত্তা

- **Pin Guarantee:** Manual Pin-এ বাছা provider-এর বাইরে কোনো ডেটা গেলে সেটা বাগ, test ফেল করে।
- **Claude সাবস্ক্রিপশন:** প্রতিটি ব্যবহারকারী নিজের সাবস্ক্রিপশন বা key; SDC শুধু অফিসিয়াল `claude` CLI চালায় যেখানে ব্যবহারকারী নিজে লগইন করেছে; token পড়ে না বা প্রক্সি করে না।
- **Daemon:** `sdcd` শুধু 127.0.0.1-এ, প্রতি client আলাদা token, Origin যাচাই, DNS rebinding সুরক্ষা; এটা ছাড়া MCP server বা extension চালু নয়।
- **Sandbox:** যাচাইয়ের সব কোড প্রজেক্টের বাইরে লিখতে পারে না, ডিফল্টে network বন্ধ, সময় ও মেমরি সীমা।
- **Untrusted content:** README, issue, web page, tool ও MCP আউটপুট সবই ডেটা, নির্দেশ নয়; উৎস-চিহ্নসহ প্রম্পটে; কোনো লেখা মোড বা chain বদলাতে পারে না।
- **শেখা জ্ঞান:** `knowledge_core` দরজা (পরীক্ষায় ৫০টি বিষাক্ত প্রস্তাবের ৫০টিই আটকানো গেছে)।

### Bug Shield: দশটি নিয়ম

1. প্রতিটি নতুন আচরণ আলাদা সুইচে; বন্ধ করলে হুবহু আগের আচরণ, test দিয়ে প্রমাণ।
2. test আগে, কোড পরে।
3. প্রতিটি ঠিক করা বাগের regression test।
4. তিন OS-এ পুরো CI সবুজ ছাড়া merge নয়।
5. এক prompt, এক ফিচার; অপ্রাসঙ্গিক refactor নিষেধ।
6. ঝুঁকির জায়গায় বিশেষ test: journal-এ `kill -9`, patcher-এ fuzz ও duplicate-name, parser-এ golden file, UI-তে visual regression, Pin Guard-এ "অনুমতিহীন call নয়", Context Guard-এ overflow।
7. token কৌশলে Q3 gate; মান কমলে drift monitor নিজে বন্ধ করে।
8. nightly → beta → stable।
9. কোড-ছাড়া crash report (সম্মতিতে) ও এক ক্লিকে আগের সংস্করণ।
10. Kill switch ও `v3.maxQuality` প্রতিটি release-এ test।

## দিনে দিনে শেখা

SDC শেখে শুধু যাচাই-করা ফল থেকে, মডেলের দাবি থেকে নয়; শেখা জ্ঞান `knowledge_core`-এর প্রমাণ-দরজা পার হয়ে ঢোকে, ভুল প্রমাণিত হলে rollback। SDC নিজের কোড নিজে বদলায় না, chain-এর ক্রমও নয়; নতুন সংস্করণ আসে সাইন করা updater দিয়ে।

| কখন | কী শেখে | কী বদলায় |
|---|---|---|
| প্রতিটি ধাপে | recipe, lesson, ব্যর্থতার ধরন, আসল diff-এর আকার, Step Trace | একই ধরনের পরের কাজ কম ধাপে |
| প্রতিটি chain-ধাপে | chain-এর সমাধানের পথ, কোন সারিতে সফল | পরের বার লোকাল নিজেই পারে |
| প্রতি রাতে (চালু করলে, বাজেট-সীমায়) | Replay দিয়ে পুরনো recipe আবার যাচাই, router-এর ডেটা | ভুল recipe বাদ |
| প্রতি সপ্তাহে | প্রতি মডেল-digest-এর pass rate, খরচ ও গতির drift; chain-এর প্রতিটি সারির সাফল্য | মান কমলে কৌশল নিজে বন্ধ; chain-এর ক্রম বদলের এক-ক্লিক প্রস্তাব; বাংলা সাপ্তাহিক রিপোর্ট |
| প্রতি মাসে বা নতুন মডেল/সংস্করণে | পর্যাপ্ত ডেটায় অস্থায়ী থেকে প্রমাণিত প্রোফাইল; Model Watch | আরও কৌশল আরও মডেলে চালু |

## রোডম্যাপ, Phase Gate ও Plan Tracker

মোট ৬৬টি task, ছয়টি phase; আগের phase-এর gate পাস না করলে পরেরটা শুরু হবে না। তারিখ নেই, কারণ সময় bench-এর ফলের ওপর নির্ভর করবে। প্রতিটি task-এর ইংরেজি prompt Prompt Book-এ একই ID ও ক্রমে (Prompt 0 ও NEXT সহ মোট ৬৮টি)। সবার আগে Context Guard ও Pin Guard, কারণ নীরব কাটা আর বাছা মডেলের প্রতি নিশ্চয়তা বাকি সবকিছুর ভিত্তি।

### Phase 0: ভিত্তি, নিরাপত্তা ও মাপ (১৮টি)

- P0-0 Plan Tracker ও NEXT runner
- P0-1 characterization test ও baseline
- P0-2 OS keychain ও Claude CLI compliance
- P0-3 বড় ফাইল মডিউলে ভাঙা (আচরণ অপরিবর্তিত)
- P0-4 লোকাল ও VPS crash-resilient journal
- P0-5 ডেটা-স্তর (`rotation.toml`, `sent.jsonl`, content-hash store)
- P0-6 `sdcd` daemon সুরক্ষা
- P0-7 UI-GUIDE, visual regression ও axe baseline
- P0-8 Local Runtime Guard (native API, `num_ctx`, overflow ধরা)
- P0-9 Model Identity ও Capability Probe
- P0-10 মডেল-নির্বাচন চুক্তি, ModelGate ও মোড ব্যাজ
- P0-11 bench নিয়ম, কঠিন tier ও ছোট-মডেল bench
- P0-12 Speed baseline ও Step Trace
- P0-13 Tool Output Compressor, OS-Normalizer ও Command Runner
- P0-14 Sandbox Runner ও Step Transaction
- P0-15 Proof-of-Work ব্যাজ ও রসিদ
- P0-16 Hardware Auto-Tuner ও VRAM Arbiter
- P0-17 Phase 0 release ও Gate 0 অডিট

**Gate 0:** তিন OS-এ সব test সবুজ; Manual Pin-এ অনুমতিহীন model call ০; overflow test-এ নীরব কাটা ০; প্লেইনটেক্সট key ০; baseline, speed baseline, Step Trace ও দুই bench প্রস্তুত।

### Phase 1: Step Runtime ও SME (১৫টি)

- P1-1 Tree-sitter Code Graph, Merkle cache ও শূন্য-মডেল query
- P1-2 Step Runtime (DAG, শর্ত, journal, আংশিক re-plan)
- P1-3 Small Model Profile ও token budgeter
- P1-4 Step Profile (sampling ও thinking)
- P1-5 Prompt Diet, Edge Pinning, স্থির prefix
- P1-6 বাঁধা আউটপুট ও Tool-call Rescue
- P1-7 Working Memory ও UI tab
- P1-8 Retrieval Cascade ও Chunked Reader
- P1-9 Source Quote Verifier
- P1-10 Task Decomposer, prefetch ও as-needed ভাঙা
- P1-11 Symbol-Anchored Patcher ও Cut-off Guard
- P1-12 LSP Bridge ও guided repair
- P1-13 Confidence Signals ও loop detector
- P1-14 Bangla/Banglish Spec Brief
- P1-15 Fast path ও Speed Gate যাচাই

**Gate 1:** একই 9B-এ (Local-only) SME-র pass rate সাধারণ prompt-এর চেয়ে বেশি, McNemar p < 0.05; cut-off, ভুয়া উদ্ধৃতি ও নীরব ভুল-patch ০; সাধারণ কাজের গতি baseline-এর ৫%-এর ভেতরে।

### Phase 2: Local Rotation (৮টি)

- P2-1 Rotation Policy Engine
- P2-2 Rotation Chain Settings UI
- P2-3 Capsule Minimizer, Redactor, অডিট ও preview
- P2-4 Chain Escalation Runner (API ও CLI worktree)
- P2-5 Handover Bridge Note
- P2-6 Planner–Executor
- P2-7 chain থেকে লোকালে শেখা ও Failure Taxonomy
- P2-8 Watchdog, Offload Limiter, Chain Health ও প্রস্তাবিত chain

**Gate 2:** Local Rotation-এর pass rate শুধু chain #1 মডেলের Q3 সীমার ভেতরে, কিন্তু cloud খরচ উল্লেখযোগ্যভাবে কম; ক্রমের বাইরে offload ০; লোকালে ফেরা ১০০%; capsule-এ secret ০।

### Phase 3: Precision ও Cursor-মানের UX (১৫টি)

- P3-1 Reuse Finder
- P3-2 Diff Budget
- P3-3 Hallucination ও Slopsquatting Guard
- P3-4 Test Impact, Characterization ও Smoke
- P3-5 Quality Floor ও AST Duplicate
- P3-6 Ripple Check
- P3-7 Spec-to-Test
- P3-8 সব মডেলে কৌশলের Q3 প্রোফাইলিং
- P3-9 Diff Review UI
- P3-10 @ Context Picker
- P3-11 Select & Instruct
- P3-12 লোকাল Semantic Index (CPU)
- P3-13 Scoped Rules, Hooks ও নিয়ম import
- P3-14 যাচাই-করা MCP Installer
- P3-15 Local GPU Scheduler (বহু প্রজেক্ট)

**Gate 3:** প্রতিটি চালু কৌশল Q3 পাস; Test Impact-এ ভুল বাদ ০; সব নতুন UI-তে visual regression ও axe পাস; তিনটি প্রজেক্ট একসাথে চালালে কোনো প্রজেক্টের ধাপ হারায় না।

### Phase 4: শেখা ও ইকোসিস্টেম (৫টি)

- P4-1 Replay Lab
- P4-2 Smart Router
- P4-3 Night Learning, Drift Monitor ও সাপ্তাহিক বাংলা রিপোর্ট
- P4-4 SDC as MCP Server
- P4-5 হালকা VS Code Companion

**Gate 4:** একই ধরনের কাজে chain-ধাপের হার কমছে, pass rate না কমিয়ে; MCP ও extension-এর নিরাপত্তা test পাস।

### Phase 5: বহু-মেশিন ও অটোমেশন (৫টি)

- P5-1 Continuity Sync
- P5-2 DevOps Dry-run ও Playbook Verifier
- P5-3 নিজের VPS-এ Background Agent
- P5-4 Self-hosted PR Review Bot
- P5-5 GitHub Issues, Telegram ও Slack

### Plan Tracker কীভাবে চলে

1. P0-0 রেপোতে `PLAN-STATUS.md` বানায়: ID, শিরোনাম, phase, নির্ভরতা, অবস্থা, সুইচ, রিপোর্ট লিংক।
2. প্রতি session-এ Prompt 0, তারপর NEXT। SDC ক্রমে প্রথম todo task নেয়, যার নির্ভরতা done আর phase-gate পাস।
3. Prompt Book থেকে সেই task-এর prompt পড়ে, Prompt 0-এর workflow মেনে কাজ করে, সব check চালায়।
4. শেষে status done বা blocked (কারণসহ), রিপোর্ট `docs/reports/<ID>.md`-এ বাংলা সারাংশসহ, CHANGELOG-এ মাপা সংখ্যা।
5. phase-এর সব task done হলে gate-এর মাপ চালিয়ে `docs/GATES.md`-এ ফল; ফেল হলে পরের phase-এ যায় না, কোন task আবার দেখতে হবে তা জানায়।

## সফলতার মাপকাঠি, ঝুঁকি ও সৎ সীমা

প্রতিটি লক্ষ্য মাপা baseline-এর তুলনায়, প্রতিটি মডেল-digest-এ আলাদা; কাল্পনিক শতাংশ লক্ষ্য নেই।

| মাপ | কীভাবে মাপা হবে | পাসের শর্ত |
|---|---|---|
| বাছা মডেলের নিশ্চয়তা | Manual Pin-এ সব bench task, প্রতিটি call-এর provider লগ | অনুমতিহীন call ০, তিন OS-এ |
| context-এর সততা | overflow test, `prompt_eval_count` বনাম পাঠানো token | নীরব কাটা ০ |
| 9B-এ কাজ শেষ (Local-only) | ছোট-মডেল bench, একই digest, SME বনাম সাধারণ, জোড়া রান | SME-তে বেশি, McNemar p < 0.05 |
| 9B + Local Rotation | একই bench, Local Rotation বনাম শুধু chain #1 | pass rate Q3 সীমার ভেতরে; cloud খরচ উল্লেখযোগ্যভাবে কম (মাপা সংখ্যা রিপোর্টে) |
| chain-এর শৃঙ্খলা ও শেখা | offload লগ ও replay | ক্রমের বাইরে offload ০; লোকালে ফেরা ১০০%; একই ধরনের কাজে chain-হার কমছে |
| নির্ভুলতা | cut-off, উদ্ধৃতি, patcher fuzz, import hallucination | কাটা কোড, ভুয়া উদ্ধৃতি, নীরব ভুল-patch, ভুয়া প্যাকেজ সব ০ |
| মান | প্রতি মডেলে bench pass rate, কঠিন tier | ১ পয়েন্টের বেশি কমা নয়; কঠিন tier-এ বাড়া প্রমাণিত |
| গতি | এক ধাপের median ও p90; শূন্য-মডেল প্রশ্ন | ৫%-এর বেশি ধীর নয়; মডেল-call ০ |
| নিরাপত্তা | `kill -9` resume, বিষাক্ত জ্ঞান, লোকাল API আক্রমণ, sandbox escape, capsule-এ secret | কোনো রিগ্রেশন নয়; সব আক্রমণ আটকানো |
| প্ল্যান অনুসরণ | NEXT-এ task বাছাই | ক্রম ও gate ভাঙা ০ |

### ঝুঁকি ও প্রতিরোধ

| ঝুঁকি | প্রতিরোধ |
|---|---|
| Step Runtime পুরনো আচরণ ভাঙে | পুরনো loop-ই ভিত্তি; সুইচ বন্ধে হুবহু আগের আচরণ; characterization test আগে |
| লুকানো পথে বাছা মডেলের বাইরে call | সব call ModelGate দিয়ে; Pin Guard test; রসিদে মডেল তালিকা |
| 8GB-তে কার্যকর context খুব ছোট | Probe দিয়ে আসল সংখ্যা; ধাপ আরও ছোট; reranker ও embedding CPU-তে |
| Ollama-র সংস্করণ বদলে আচরণ বদল | digest + runtime সংস্করণ চাবি; বদলালে আবার probe; পরিচিত সমস্যার তালিকা |
| ছোট ধাপে মোট সময় বাড়ে | fast path, শূন্য-মডেল উত্তর, সমান্তরাল যাচাই, Speed Gate |
| chain বেশি ব্যবহার হয়ে খরচ বাড়ে | সীমা, Offload Limiter, কারণ দেখানো, recipe শেখা |
| লোকাল server সমস্যায় সব cloud-এ | Local Watchdog; পরিবেশের ত্রুটি কখনো offload ঘটায় না |
| entropy-কে বেশি বিশ্বাস | শুধু সতর্কতা সংকেত; পাস/ফেল সবসময় execution-এ |
| একক প্রতিষ্ঠাতার সময় | আলাদা মডিউল, এক prompt এক ফিচার, phase-gate, Plan Tracker |
| provider-দের নিয়ম বদল | নিয়মিত নজর; Claude-এর জন্য শুধু অফিসিয়াল CLI বা API key |

### সবচেয়ে বড় সৎ সীমা

9B বড় মডেলের সমান বুদ্ধিমান হবে না। "যেকোনো কাজ শেষ" নিশ্চিত হয় Local Rotation-এ chain থাকলে; Local-only-তে সবচেয়ে কঠিন ধাপে SDC সৎভাবে থামবে। বাজারে দাবিও ঠিক এভাবেই করতে হবে।
