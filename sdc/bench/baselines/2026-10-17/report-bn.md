# Bench রিপোর্ট — `baseline-2026-10-17-deepseek`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-17 · tier: full · প্রতি task-এ run: 3 · sdcd: sdcd 2.0.4 (SDCP 0.1) · commit: `4d39612` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 120 (0) | 99.2% (119/120) | 21.2k [16.8k–27.3k] | 920 [782–985] | 8646 [7744–10.0k] | $0.0012 [$0.0011–$0.0014] | $0.202 | 8 [7–9] | 1.0 [0.9–1.0] | 83.5% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 21.4% | 30.2% | 11.0% | 12.3% | 24.2% | 0.8% | 51.6% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 11.7% | 4.1% | 83.5% |

## যেসব run fail করেছে

| agent | model | task | run | কারণ |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | ml-bn | 1 | answer lacks /1250\|১২৫০/ |
