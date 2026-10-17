# Bench রিপোর্ট — `gate0-pin`

> এই ফাইল `sdcd bench report` স্ক্রিপ্ট দিয়ে `bench/results/` থেকে তৈরি; হাতে লেখা কোনো সংখ্যা নেই।

- তারিখ: 2026-10-17 · tier: full · প্রতি task-এ run: 1 · sdcd: sdcd 2.0.4 (SDCP 0.1) · commit: `44fb448` · OS: macos aarch64
- **Noise (চান্স কতটা):** 40 টি task-এর মধ্যে 0 টি task রানে রানে ফল বদলেছে (flaky); রানগুলোর pass rate-এর ব্যবধান 0.0 শতাংশ-পয়েন্ট (100.0%). seed: 20261017, temperature: মডেলের নিজের

মান = median [bootstrap 95% CI]। billable = uncached_in + 0.1×cached_in + out×(price_out/price_in)।

## প্রতি model-এর ফল

| agent | model | run (skip) | pass | input tokens | output tokens | billable | USD/run | মোট USD | সময় (s) | TTFT (s) | cached |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 40 (0) | 100.0% (40/40) | 33.2k [19.4k–38.2k] | 981 [690–1201] | 10.7k [8218–12.5k] | $0.0015 [$0.0012–$0.0017] | $0.065 | 10 [7–11] | 0.9 [0.9–1.0] | 81.7% |

## Input কী দিয়ে তৈরি (ledger, A-02)

প্রতিটা অংশের ভাগ provider-এর মোট input-এর সাথে মেলানো (ledger-এর যোগফল = provider-এর গণনা)।

| agent | model | system | tools | context | history | tool_results | user | system+tools |
|---|---|---|---|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | 23.9% | 33.4% | 12.4% | 8.3% | 21.1% | 1.0% | 57.3% |

## অনুমান যাচাইয়ের কাঁচা সংখ্যা

| agent | model | input CV (A-03) | একই ফাইল আবার পড়া (A-10) | cached ভাগ (A-08/A-09) |
|---|---|---|---|---|
| sdc | deepseek:deepseek-chat | — (একই task-এ ≥২ run লাগে) | 1.7% | 81.7% |
