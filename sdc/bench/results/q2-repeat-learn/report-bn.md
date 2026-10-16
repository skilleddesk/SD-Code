# Bench রিপোর্ট — `q2-repeat-learn`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-16 · tier: repeat · প্রতি task-এ run: 5 · sdcd: sdcd 2.0.2 (SDCP 0.1) · commit: `bbe3d29` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 20 (0) | 100.0% (20/20) | 30.6k [30.5k–37.0k] | 994 [774–1150] | 10.9k [10.5k–11.7k] | $0.0015 [$0.0015–$0.0016] | $0.032 | 10 [8–11] | 1.0 [0.8–1.0] | 80.5% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 23.2% | 38.8% | 15.7% | 6.3% | 15.0% | 1.0% | 62.0% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 13.4% | 0.0% | 80.5% |
