"""SDC against Claude Code on the same model (DeepSeek), from the two runs.jsonl files. Reproduce:
   python3 -I compare-vs-cc.py            (run from bench/results)
Pairs are (task, run); a saving is 1 - sdc/cc. The interval is a percentile bootstrap (fixed seed)."""
import json, random, statistics

def load(label):
    return {(r["task"], r["run"]): r for r in map(json.loads, open(f"{label}/runs.jsonl"))}

cc, sdc = load("vs-cc-on-deepseek"), load("vs-sdc-on-deepseek")
keys = sorted(set(cc) & set(sdc))

def total(runs, field):
    return sum(runs[k][field] for k in keys)

def billable(r):  # uncached in + 0.1 * cached in + out * (price_out / price_in), DeepSeek chat $0.14 / $0.28
    return r["input_tokens"] - r["cached_tokens"] + 0.1 * r["cached_tokens"] + r["output_tokens"] * 2.0

print(f"{'task/run':<16}{'CC in':>9}{'SDC in':>9}{'CC $':>9}{'SDC $':>9}{'saving':>9}")
for k in keys:
    a, b = cc[k], sdc[k]
    print(f"{k[0]+'/'+str(k[1]):<16}{a['input_tokens']:>9}{b['input_tokens']:>9}{a['usd']:>9.4f}{b['usd']:>9.4f}{100*(1-b['usd']/a['usd']):>8.0f}%")

print()
for field, name in [("input_tokens", "input tokens"), ("cached_tokens", "cached tokens"), ("output_tokens", "output tokens"), ("usd", "USD")]:
    a, b = total(cc, field), total(sdc, field)
    print(f"{name:<14} Claude Code {a:>10,.4f}  SDC {b:>10,.4f}  SDC is {100*(1-b/a):.0f}% lower")

savings = [1 - sdc[k]["usd"] / cc[k]["usd"] for k in keys]
random.seed(7)
boots = sorted(statistics.median(random.choices(savings, k=len(savings))) for _ in range(5000))
print(f"\nper-pair USD saving: median {100*statistics.median(savings):.0f}%  95% interval [{100*boots[125]:.0f}%, {100*boots[4875]:.0f}%]  over {len(keys)} pairs")
print("pass: Claude Code", sum(cc[k]["pass"] for k in keys), "/", len(keys), "  SDC", sum(sdc[k]["pass"] for k in keys), "/", len(keys))
print("sdc wins on", sum(1 for s in savings if s > 0), "of", len(savings), "pairs")
