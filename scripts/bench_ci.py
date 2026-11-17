#!/usr/bin/env python3
"""The benchmark suite CI tracks (spec §23.7, §23.8).

  scripts/bench_ci.py run [--out bench.json] [--skip-criterion]
      Build the release binary and measure: binary size, startup and
      memory at first frame (bench.sh), idle CPU and typing/render latency
      (bench_interactive.py), memory after Quick Open and a search on this
      repository (bench_memory.py), and the criterion benches whose numbers
      don't depend on the repository's contents.
  scripts/bench_ci.py compare bench.json HISTORY.jsonl [--summary FILE]
      Compare with the median of the last 10 entries of the history.
      Size and memory are steady on shared runners: a regression past
      their limit fails (exit 1). Timings are noisy there: a regression
      past their limit only warns.
  scripts/bench_ci.py append bench.json HISTORY.jsonl
      Add the result to the history (the newest 300 are kept).

YACE_BENCH_INFLATE="metric=factor,…" multiplies measured values, to check
that a regression is caught (e.g. binary_size_bytes=1.1).
"""

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target/release/yace"
WINDOW = 10
KEEP = 300

# unit -> (limit as a fraction over the baseline, gate: fail instead of warn)
GATES = {"bytes": (0.03, True), "MiB": (0.10, True), "ms": (0.30, False), "µs": (0.30, False), "%": (None, False)}
# Smaller changes than this are noise, whatever the percentage (sub-ms latencies).
MIN_DELTA = {"ms": 0.25}

CRITERION = [
    ("yace-buffer", "buffer", ""),
    ("yace-syntax", "syntax", ""),
    ("yace-search", "search", "quick_open"),
    ("yace-git", "diff", ""),
]


def sh(cmd, **kw):
    print("+", " ".join(map(str, cmd)), file=sys.stderr, flush=True)
    return subprocess.run(cmd, cwd=ROOT, check=True, text=True, capture_output=True, **kw).stdout


def num(pattern, text):
    m = re.search(pattern, text)
    if not m:
        sys.exit(f"bench_ci: no match for {pattern!r} in:\n{text}")
    return float(m.group(1))


def run(args):
    sh(["cargo", "build", "--release", "--locked", "-q", "-p", "yace"])
    env = dict(os.environ, YACE_BENCH_BIN=str(BIN))
    m = {}

    def put(name, value, unit):
        m[name] = {"value": round(value, 3), "unit": unit}

    put("binary_size_bytes", BIN.stat().st_size, "bytes")

    out = sh(["scripts/bench.sh", "20", str(ROOT / "crates/yace/src/ui.rs")], env=env)
    put("startup_median_ms", num(r"median ([\d.]+) ms", out), "ms")
    put("startup_p95_ms", num(r"p95 ([\d.]+) ms", out), "ms")
    put("rss_first_frame_mib", num(r"memory at first frame: median ([\d.]+) MiB", out), "MiB")

    out = sh(["python3", "scripts/bench_interactive.py", "300"], env=env)
    put("idle_cpu_percent", num(r"idle CPU over 5 s: ([\d.]+)%", out), "%")
    put("typing_input_p50_ms", num(r"input handling p50 ([\d.]+) ms", out), "ms")
    put("typing_input_p95_ms", num(r"input handling p50 [\d.]+ ms, p95 ([\d.]+) ms", out), "ms")
    put("render_p50_ms", num(r"render: p50 ([\d.]+) ms", out), "ms")
    put("render_p95_ms", num(r"render: p50 [\d.]+ ms, p95 ([\d.]+) ms", out), "ms")

    out = sh(["python3", "scripts/bench_memory.py", str(ROOT), "import"], env=env)
    put("rss_startup_mib", num(r"after startup:\s+([\d.]+) MiB", out), "MiB")
    put("rss_quick_open_mib", num(r"after Quick Open index:\s+([\d.]+) MiB", out), "MiB")
    put("rss_after_search_idle_mib", num(r"after 3 s idle:\s+([\d.]+) MiB", out), "MiB")

    if not args.skip_criterion:
        for crate, bench, flt in CRITERION:
            cmd = ["cargo", "bench", "--locked", "-q", "-p", crate, "--bench", bench, "--"]
            sh(cmd + ([flt] if flt else []) + ["--noplot"])
        for est in sorted((ROOT / "target/criterion").glob("*/new/estimates.json")):
            name = est.parent.parent.name
            if name.startswith(("walk_", "content_search_")):
                continue
            ns = json.loads(est.read_text())["median"]["point_estimate"]
            put(f"criterion_{name}_us", ns / 1000, "µs")

    for item in filter(None, os.environ.get("YACE_BENCH_INFLATE", "").split(",")):
        name, factor = item.split("=")
        m[name]["value"] = round(m[name]["value"] * float(factor), 3)

    commit = sh(["git", "rev-parse", "HEAD"]).strip()
    result = {"commit": commit, "date": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "metrics": m}
    Path(args.out).write_text(json.dumps(result, indent=2) + "\n")
    print(table(result, None))


def load_history(path):
    p = Path(path)
    return [json.loads(line) for line in p.read_text().splitlines() if line.strip()] if p.exists() else []


def fmt(value, unit):
    return f"{value:,.0f} {unit}" if unit == "bytes" else f"{value:g} {unit}"


def table(result, baseline):
    rows = ["| Metric | Value | Baseline (median of last runs) | Change |", "|---|---:|---:|---:|"]
    for name, v in result["metrics"].items():
        base = baseline.get(name) if baseline else None
        change = f"{(v['value'] / base - 1) * 100:+.1f}%" if base else "–"
        b = fmt(base, v["unit"]) if base else "–"
        rows.append(f"| {name} | {fmt(v['value'], v['unit'])} | {b} | {change} |")
    return "\n".join(rows)


def compare(args):
    result = json.loads(Path(args.result).read_text())
    history = load_history(args.history)[-WINDOW:]
    baseline = {}
    for name in result["metrics"]:
        values = [h["metrics"][name]["value"] for h in history if name in h["metrics"]]
        if values:
            baseline[name] = statistics.median(values)
    failures, warnings = [], []
    for name, v in result["metrics"].items():
        base = baseline.get(name)
        limit, gate = GATES[v["unit"]]
        if not base or limit is None:
            continue
        if v["value"] > base * (1 + limit) and v["value"] - base > MIN_DELTA.get(v["unit"], 0):
            msg = f"{name}: {fmt(v['value'], v['unit'])} is {(v['value'] / base - 1) * 100:+.1f}% over the baseline {fmt(base, v['unit'])} (limit +{limit * 100:g}%)"
            (failures if gate else warnings).append(msg)
    text = [f"## Benchmarks for {result['commit'][:12]}", ""]
    text.append(f"Compared with the median of the last {len(history)} runs on main." if history else "No history yet: this run is the first baseline.")
    text += ["", table(result, baseline), ""]
    for f in failures:
        print(f"::error title=Benchmark regression::{f}")
        text.append(f"- **Regression:** {f}")
    for w in warnings:
        print(f"::warning title=Slower (timings are noisy on shared runners)::{w}")
        text.append(f"- Slower: {w}")
    if args.summary:
        with open(args.summary, "a") as f:
            f.write("\n".join(text) + "\n")
    print("\n".join(text))
    if failures:
        sys.exit(1)


def append(args):
    result = json.loads(Path(args.result).read_text())
    history = [h for h in load_history(args.history) if h["commit"] != result["commit"]]
    history = (history + [result])[-KEEP:]
    Path(args.history).write_text("".join(json.dumps(h, separators=(",", ":")) + "\n" for h in history))
    print(f"history: {len(history)} entries")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--out", default="bench.json")
    r.add_argument("--skip-criterion", action="store_true")
    c = sub.add_parser("compare")
    c.add_argument("result")
    c.add_argument("history")
    c.add_argument("--summary")
    a = sub.add_parser("append")
    a.add_argument("result")
    a.add_argument("history")
    args = ap.parse_args()
    {"run": run, "compare": compare, "append": append}[args.cmd](args)


if __name__ == "__main__":
    main()
