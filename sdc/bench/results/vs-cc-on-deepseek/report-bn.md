# Bench রিপোর্ট — `vs-cc-on-deepseek`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: smoke · প্রতি task-এ run: 2 · sdcd: sdcd 2.0.1 (SDCP 0.1) · commit: `4ad54ca` · OS: macos aarch64
- প্রতিযোগী version: claude-code `2.1.296 (Claude Code)`

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| claude-code | claude-code:deepseek-chat | 6 (0) | 100.0% (6/6) | 136.7k [117.6k–207.4k] | 2434 [1642–3460] | 36.7k [34.1k–44.8k] | $0.0051 [$0.0048–$0.0063] | $0.032 | 19 [17–26] | — | 86.9% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| claude-code | claude-code:deepseek-chat | 16.7% | — | 86.9% |
