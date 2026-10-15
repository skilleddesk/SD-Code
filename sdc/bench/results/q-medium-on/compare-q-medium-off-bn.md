# Bench রিপোর্ট — `q-medium-on`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: medium · প্রতি task-এ run: 2 · sdcd: sdcd 2.0.1 (SDCP 0.1) · commit: `d630f81` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 40 (0) | 100.0% (40/40) | 18.3k [8944–28.1k] | 754 [594–906] | 7839 [6833–11.3k] | $0.0011 [$0.0010–$0.0016] | $0.061 | 7 [5–9] | 1.0 [1.0–1.0] | 79.8% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 22.7% | 33.4% | 12.9% | 8.6% | 21.6% | 0.8% | 56.1% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 1.6% | 4.2% | 79.8% |

## তুলনা: `q-medium-off` (আগে) বনাম `q-medium-on` (এখন)

Q3 নিয়ম: McNemar p ≥ 0.1, pass rate কমবে ≤ 1 point, আর billable সাশ্রয়ের 95% CI > 0।

| model | জোড়া | pass পরিবর্তন | McNemar p | input সাশ্রয় | billable সাশ্রয় | USD সাশ্রয় | রায় |
|---|---|---|---|---|---|---|---|
| deepseek:deepseek-chat | 40 | +0.0 pt | 1.000 | 42.0% [35.1%–42.9%] | 22.5% [18.4%–27.1%] | 22.5% [18.4%–27.1%] | ✅ গ্রহণযোগ্য |
