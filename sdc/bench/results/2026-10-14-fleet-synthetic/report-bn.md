# Bench রিপোর্ট — `2026-10-14-fleet-synthetic`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-14 · tier: ? · প্রতি task-এ run: null · sdcd: 1.0.0 · commit: `ccf7458` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|

## Fleet clustering (A-16, synthetic fleet)

| server | সত্যিই আলাদা | সব output (tokens) | clustered (tokens) | কম | group | লুকানো diff |
|---|---|---|---|---|---|---|
| 5 | 1 | 386 | 214 | 44.6% | 2 | 0 |
| 10 | 2 | 764 | 242 | 68.3% | 2 | 0 |
| 20 | 3 | 1519 | 297 | 80.4% | 2 | 0 |
