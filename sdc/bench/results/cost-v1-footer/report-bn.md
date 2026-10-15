# Bench রিপোর্ট — `cost-v1-footer`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: smoke · প্রতি task-এ run: 1 · sdcd: sdcd 1.0.0 (SDCP 0.1) · commit: `558e010` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 3 (0) | 100.0% (3/3) | 61.9k [15.1k–122.2k] | 1257 [617–3838] | 15.9k [8110–29.6k] | $0.0022 [$0.0011–$0.0041] | $0.007 | 16 [6–34] | 0.7 [0.6–0.9] | 87.6% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | — | 87.6% |
