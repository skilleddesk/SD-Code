# Bench রিপোর্ট — `q-repeat-on-1`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: repeat · প্রতি task-এ run: 1 · sdcd: sdcd 2.0.1 (SDCP 0.1) · commit: `d630f81` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 4 (0) | 100.0% (4/4) | 42.9k [41.1k–46.1k] | 1182 [927–1871] | 12.9k [12.5k–14.1k] | $0.0018 [$0.0017–$0.0020] | $0.007 | 14 [11–15] | 1.0 [0.6–1.0] | 84.2% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 24.0% | 40.4% | 11.5% | 6.7% | 16.3% | 1.0% | 64.4% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | 5.2% | 84.2% |
