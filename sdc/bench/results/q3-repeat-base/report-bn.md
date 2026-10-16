# Bench রিপোর্ট — `q3-repeat-base`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-16 · tier: repeat · প্রতি task-এ run: 5 · sdcd: sdcd 2.0.2 (SDCP 0.1) · commit: `bbe3d29` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 20 (0) | 100.0% (20/20) | 41.4k [35.8k–42.5k] | 980 [906–1222] | 12.5k [11.5k–13.3k] | $0.0017 [$0.0016–$0.0019] | $0.035 | 11 [9–12] | 0.9 [0.7–1.0] | 83.1% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 23.5% | 40.2% | 12.2% | 5.7% | 17.4% | 1.0% | 63.7% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 25.8% | 1.1% | 83.1% |
