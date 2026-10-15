# Bench রিপোর্ট — `q-one-interlingua`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-15 · tier: medium · প্রতি task-এ run: 1 · sdcd: sdcd 2.0.1 (SDCP 0.1) · commit: `d630f81` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 20 (0) | 100.0% (20/20) | 28.4k [14.9k–55.6k] | 902 [680–1256] | 11.0k [8309–15.4k] | $0.0015 [$0.0012–$0.0022] | $0.035 | 10 [7–17] | 1.4 [1.2–1.5] | 80.7% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 22.3% | 43.6% | 8.3% | 5.9% | 19.2% | 0.8% | 65.8% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | 0.0% | 80.7% |
