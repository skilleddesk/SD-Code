# Bench রিপোর্ট — `q-repeat-nolearn-1`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: repeat · প্রতি task-এ run: 1 · sdcd: sdcd 2.0.1 (SDCP 0.1) · commit: `d630f81` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 4 (0) | 100.0% (4/4) | 44.1k [36.8k–47.4k] | 1183 [882–1331] | 12.6k [11.9k–13.7k] | $0.0018 [$0.0017–$0.0019] | $0.007 | 13 [12–15] | 1.0 [0.9–1.2] | 84.2% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 24.1% | 40.9% | 12.0% | 5.9% | 16.1% | 1.0% | 65.1% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | 0.0% | 84.2% |
