# Bench রিপোর্ট — `cost-v1-keychain`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-14 · tier: smoke · প্রতি task-এ run: 1 · sdcd: sdcd 1.0.0 (SDCP 0.1) · commit: `69367a2` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 3 (0) | 100.0% (3/3) | 26.1k [16.0k–93.4k] | 941 [428–2431] | 26.9k [17.8k–98.2k] | $0.0038 [$0.0025–$0.0138] | $0.020 | 8 [7–29] | 1.2 [1.1–7.8] | 0.0% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | — | 0.0% |
