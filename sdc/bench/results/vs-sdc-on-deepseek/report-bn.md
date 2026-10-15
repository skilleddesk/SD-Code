# Bench রিপোর্ট — `vs-sdc-on-deepseek`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: smoke · প্রতি task-এ run: 2 · sdcd: sdcd 2.0.1 (SDCP 0.1) · commit: `4ad54ca` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 6 (0) | 100.0% (6/6) | 26.3k [18.6k–81.3k] | 863 [571–2504] | 9842 [8963–22.1k] | $0.0014 [$0.0013–$0.0031] | $0.011 | 9 [7–41] | 0.9 [0.8–20.0] | 82.1% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 20.6% | 40.4% | 7.4% | 8.4% | 22.6% | 0.5% | 61.1% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 2.3% | 3.9% | 82.1% |
