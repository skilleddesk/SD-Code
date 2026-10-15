# Bench রিপোর্ট — `q-repeat-off-2`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: repeat · প্রতি task-এ run: 1 · sdcd: sdcd 2.0.1 (SDCP 0.1) · commit: `d630f81` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 4 (0) | 100.0% (4/4) | 39.4k [33.6k–48.5k] | 1089 [759–1386] | 12.4k [10.7k–13.9k] | $0.0017 [$0.0015–$0.0020] | $0.007 | 11 [9–16] | 1.0 [0.7–1.5] | 82.9% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 23.4% | 45.8% | 7.9% | 5.2% | 16.6% | 1.0% | 69.2% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | 0.0% | 82.9% |
