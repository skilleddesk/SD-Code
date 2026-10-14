# Bench রিপোর্ট — `cost-smoke-v2`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-14 · tier: smoke · প্রতি task-এ run: 1 · sdcd: sdcd 2.0.0 (SDCP 0.1) · commit: `69367a2` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 3 (0) | 100.0% (3/3) | 26.5k [22.1k–71.8k] | 994 [522–2216] | 10.5k [9323–20.5k] | $0.0015 [$0.0013–$0.0029] | $0.006 | 9 [9–27] | 1.0 [0.9–6.8] | 80.8% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 21.0% | 41.2% | 7.4% | 7.0% | 22.8% | 0.5% | 62.2% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | 0.0% | 80.8% |
