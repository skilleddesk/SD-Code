# Bench রিপোর্ট — `v3-learn-default-check`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-16 · tier: repeat · প্রতি task-এ run: 1 · sdcd: sdcd 2.0.2 (SDCP 0.1) · commit: `bbe3d29` · OS: macos aarch64

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 4 (0) | 75.0% (3/4) | 31.7k [0–42.3k] | 960 [0–1497] | 10.6k [0–13.6k] | $0.0015 [$0.0000–$0.0019] | $0.005 | 48 [7–621] | 0.9 [0.9–70.6] | 81.7% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 24.7% | 39.3% | 11.6% | 6.6% | 16.7% | 1.1% | 64.0% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | 0.0% | 81.7% |

## যেসব run fail করেছে

| agent | model | task | run | কারণ |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | edit-03 | 0 | stopped at the 600 s time cap; `node --test test/*.test.ts` failed:       at formatPrice (file:///private/var/folders/rc/5s79xsld04d71c6n36tgxg5w0000gn/T/sdc-bench/v3-learn-default-check/edit-03-deepseek_deepseek-chat-0/src/format.ts:11:21) |
