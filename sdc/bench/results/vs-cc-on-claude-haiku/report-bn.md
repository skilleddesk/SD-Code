# Bench রিপোর্ট — `vs-cc-on-claude-haiku`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: smoke · প্রতি task-এ run: 1 · sdcd: sdcd 2.0.1 (SDCP 0.1) · commit: `4ad54ca` · OS: macos aarch64
- প্রতিযোগী version: claude-code `2.1.296 (Claude Code)`

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| claude-code | claude-code:haiku | 3 (0) | 100.0% (3/3) | 104.7k [80.9k–134.2k] | 1652 [1147–2563] | 30.3k [25.1k–39.2k] | $0.0047 [$0.0040–$0.0059] | $0.015 | 15 [11–26] | — | 85.7% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| claude-code | claude-code:haiku | — (একই task-এ ≥২ run লাগে) | — | 85.7% |
