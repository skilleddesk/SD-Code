# SDC 1.0 - why the agent was slow, and what was measured

The report: the same prompt that Claude Code finishes in about an hour took SDC ten to twelve hours. Everything
below was measured on the owner's machine (Windows 11, Bangladesh, Alibaba Model Studio key on `dashscope-us`)
on 2026-10-09, from the owner's own event log and from a fixed benchmark task.

## Where the hours went (the owner's real turns, from `sdc.db`)

| Turn | Wall | Steps | Model wait | Tools | Notes |
| --- | --- | --- | --- | --- | --- |
| turn-210368 | 11.5 h | 1,101 | 3.4 h | 0.4 h | **7 h 40 min waiting on one permission** at night |
| turn-29992 | 7.0 h | 1,181 | 5.6 h | 1.4 h | 180 M input tokens; folded on 503 of 1,181 steps |
| turn-291536 (0.22) | 1 h 52 min (never closed) | 148 | 77 min | 34 min | first token 21-38 s per step, growing with context |

Causes, largest first:

1. **The model was served outside the key's region.** `deepseek-v4.1-flash` has no US deployment on Model
   Studio: every step waited 12-20 s for its first token. Its US-served sibling `deepseek-v4-flash-us` answers in
   1.5 s.
2. **Only the system prompt was cached** on the OpenAI-compatible path, so every step re-processed the whole
   conversation: a step's wait grew from 22 s at 10k tokens to 38 s at 70k.
3. **The window was assumed to be 128K** (the model has 393K), so a long turn folded old output on almost
   every step; each fold rewrote the start of the conversation, which breaks the provider's prefix cache, and
   files had to be read again (one file 12 times in one turn).
4. **`parallel_tool_calls` was never asked for** - Model Studio's default is one tool call per reply: 60% of the
   1,181 steps carried a single call.
5. **DeepSeek V4 always thought at `high`** - SDC sent `reasoning_effort` only to OpenAI models.
6. Sub-agents ran their reads one after another.

## The benchmark

A fixed Node project with five planted bugs and 13 tests (7 failing). Prompt: "The tests in this project fail.
Find the cause of every failing test, fix the code (not the tests), and run the tests until they all pass."
Same key, autonomy auto, a separate daemon and data folder per run. Every run below ended with all tests passing.

| Build | Model | Wall | Steps | First token per step |
| --- | --- | --- | --- | --- |
| 0.22 | deepseek-v4.1-flash | 255 s | 13 | 3-38 s |
| 1.0 (effort auto → low) | deepseek-v4.1-flash | 159-163 s | 7 | 12-20 s |
| 0.22 | deepseek-v4-flash-us | 42 s | 7 | 1.5 s |
| 0.22 | qwen3.6-flash-us | 27-47 s (4 runs, mean 35 s) | 6-12 | 1.5 s |
| 1.0 | qwen3.6-flash-us | 28-49 s (4 runs, mean 37 s) | 6-19 | 1.5 s |

On a short task 0.22 and 1.0 are within the run-to-run noise on the same model; what 1.0 changes on it is the
cache - 79% of input read from the cache in a 1.0 run against 38-39% in 0.22 runs of the same length - and that
is what decides a long turn, where every step re-reads the whole history. The model is the largest single
factor: **6-9x** between a model served in the key's region and one that is not.

## What 1.0 changed

- Model Studio requests carry `parallel_tool_calls`, a cache mark on the newest message (as the Anthropic path
  has since 0.20), and the turn's effort for DeepSeek V4; a level the endpoint refuses is stepped down from
  and remembered (`agent::ask_model`).
- Effort `auto` on DeepSeek V4 is `low` except for a large many-part task.
- Folding happens at 60% of the window, at most 160K tokens, and goes down to half of that in one go, so the
  cached start of the conversation stays the same for many steps (`agent::keep_small`); the threshold uses
  the provider's own token count from the previous step.
- Known model families get their real window (`context::family_window`), and a request the model finds too
  long is folded hard and asked again instead of ending the turn.
- Sub-agents read in parallel and think at `low`.
- On a US key, a model's `-us` copy is used when Model Studio offers one; otherwise the nearest US-served model
  is suggested once (`providers::models::region_twin`).

## Opening SDC

| | 0.22 | 1.0 |
| --- | --- | --- |
| Events in the owner's log | 303,909 | 43,195 |
| `event.list` at every start (debug build) | 22.2 s, 67 MB | 3.3 s, 21 MB |
| Database file | 73.6 MB | 30.1 MB |

A finished turn's streamed pieces are joined into one event per run (`sdcp::events::compaction`); the first
1.0 start compacts the existing log once (5.5 s on the owner's). The window folds the backlog in one state
change. A turn left `running` by a daemon that closed mid-turn is closed as Interrupted at the next start - it
kept the sidebar's spinner turning forever.
