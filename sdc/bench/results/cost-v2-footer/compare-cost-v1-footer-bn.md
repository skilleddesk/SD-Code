# Bench রিপোর্ট — `cost-v2-footer`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: smoke · প্রতি task-এ run: 1 · sdcd: sdcd 2.0.1 (SDCP 0.1) · commit: `558e010` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 3 (0) | 100.0% (3/3) | 27.7k [22.2k–78.0k] | 790 [510–1517] | 10.0k [9511–19.1k] | $0.0014 [$0.0013–$0.0027] | $0.005 | 8 [7–29] | 1.3 [1.0–11.2] | 82.5% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 22.1% | 43.2% | 7.1% | 4.4% | 22.7% | 0.5% | 65.3% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | 0.0% | 82.5% |

## তুলনা: `cost-v1-footer` (আগে) বনাম `cost-v2-footer` (এখন)

Q3 নিয়ম: McNemar p ≥ 0.1, pass rate কমবে ≤ 1 point, আর billable সাশ্রয়ের 95% CI > 0।

| model | জোড়া | pass পরিবর্তন | McNemar p | input সাশ্রয় | billable সাশ্রয় | USD সাশ্রয় | রায় |
|---|---|---|---|---|---|---|---|
| deepseek:deepseek-chat | 3 | +0.0 pt | 1.000 | 36.2% [-47.0%–55.2%] | 35.4% [-23.6%–40.1%] | 35.4% [-23.6%–40.1%] | ❌ saving CI lower bound -23.6% is not above 0 |
