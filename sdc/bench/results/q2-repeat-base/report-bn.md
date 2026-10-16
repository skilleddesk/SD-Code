# Bench রিপোর্ট — `q2-repeat-base`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-16 · tier: repeat · প্রতি task-এ run: 5 · sdcd: sdcd 2.0.2 (SDCP 0.1) · commit: `bbe3d29` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 20 (0) | 100.0% (20/20) | 41.8k [35.0k–47.3k] | 1056 [856–1370] | 12.5k [11.3k–13.4k] | $0.0017 [$0.0016–$0.0019] | $0.034 | 12 [9–13] | 1.0 [0.9–1.0] | 83.4% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 23.9% | 40.3% | 12.2% | 6.4% | 16.2% | 1.0% | 64.2% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 30.2% | 1.5% | 83.4% |
