# Bench রিপোর্ট — `2026-10-14-search-v3`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-14 · tier: ? · প্রতি task-এ run: null · sdcd: 1.0.0 · commit: `835822b` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|

## Search-এর গতি (A-04/A-05)

ফোল্ডার: 52501 ফাইল · প্রথম call-এ OS cache খালি করা হয়নি (সত্যিকারের cold নয়)।

| ধরন | query | ফল | প্রথম (ms) | p50 (ms) | p95 (ms) |
|---|---|---|---|---|---|
| filename | `**/*.rs` | 100 | 150.5 | 2.5 | 3.1 |
| filename | `**/module_7*/**/*.ts` | 100 | 3.2 | 3.2 | 3.4 |
| text | `fn compute_total` | 100 | 4.5 | 3.3 | 3.6 |
| text | `TODO\(perf\)` | 100 | 2.0 | 2.0 | 2.3 |
| text | `[A-Z][a-z]+Service::new` | 100 | 3.8 | 3.3 | 3.9 |
| text | `no_such_symbol_anywhere` | 0 | 7.5 | 6.6 | 7.4 |
