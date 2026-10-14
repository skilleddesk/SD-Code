# Bench রিপোর্ট — `2026-10-14-resume`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-14 · tier: ? · প্রতি task-এ run: null · sdcd: 1.0.0 · commit: `1abe43a` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|

## হঠাৎ বন্ধ হওয়ার পর resume (A-19, local)

| kill | turn শুরুর আগেই | resume হয়েছে | সঠিক | হার | ফাইল নষ্ট | command দুবার চলেছে |
|---|---|---|---|---|---|---|
| 100 | 0 | 85 | 100 | 100.0% | 0 | 0 |
