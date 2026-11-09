#!/usr/bin/env python3
"""Compare terminal editors: time to the first frame that shows the file,
and memory once idle. Run by scripts/bench_compare.sh inside a clean
Ubuntu container, so every editor starts with its defaults, no config and
no language server.

Usage: bench_compare.py FILE MARKER NAME=COMMAND...
  e.g. bench_compare.py ui.rs Rendering yace=yace hx=hx nvim=nvim micro=micro

- startup: from starting the editor in a 120x34 tmux pane until the pane
  shows MARKER (a word from the file's first lines), polled every 2 ms
- memory: proportional set size (PSS) of the editor and every process it
  started, after 3 s idle with the file open. PSS counts shared libraries
  fairly between processes, so editors that run as two processes (Neovim's
  TUI and its server) aren't counted twice.
Each editor runs RUNS times (default 5); the median is reported.
"""
import os, statistics, subprocess, sys, time

RUNS = int(os.environ.get("RUNS", "5"))
path, marker = sys.argv[1], sys.argv[2]
editors = [a.split("=", 1) for a in sys.argv[3:]]


def tree(pid):
    """pid and all its descendants."""
    out, todo = [], [pid]
    while todo:
        p = todo.pop()
        out.append(p)
        try:
            for t in os.listdir(f"/proc/{p}/task"):
                with open(f"/proc/{p}/task/{t}/children") as f:
                    todo += [int(c) for c in f.read().split()]
        except OSError:
            pass
    return out


def pss_mib(pid):
    total = 0
    for p in tree(pid):
        try:
            with open(f"/proc/{p}/smaps_rollup") as f:
                for line in f:
                    if line.startswith("Pss:"):
                        total += int(line.split()[1])
        except OSError:
            pass
    return total / 1024


def tmux(*args):
    return subprocess.run(["tmux", *args], capture_output=True, text=True).stdout


def run(cmd):
    tmux("respawn-pane", "-k", "-t", "bench", "sleep 600")
    time.sleep(0.3)
    t0 = time.perf_counter()
    tmux("respawn-pane", "-k", "-t", "bench", f"exec {cmd} {path}")
    first = None
    while time.perf_counter() - t0 < 10:
        if marker in tmux("capture-pane", "-p", "-t", "bench"):
            first = time.perf_counter() - t0
            break
        time.sleep(0.002)
    time.sleep(3)
    pid = int(tmux("display-message", "-p", "-t", "bench", "#{pane_pid}").strip())
    return first, pss_mib(pid)


# A real terminal (tmux) answers the queries editors send at startup, so
# no editor waits on a timeout. 120x34, truecolor.
subprocess.run(["tmux", "-f", "/dev/null", "new-session", "-d", "-s", "bench", "-x", "120", "-y", "34"], check=True)
tmux("set", "-g", "default-terminal", "tmux-256color")
tmux("set", "-ga", "terminal-overrides", ",*:Tc")

print(f"file: {path}  runs: {RUNS}  terminal: 120x34")
print(f"{'editor':10} {'first frame':>12} {'memory (PSS)':>13}")
for name, cmd in editors:
    starts, mems = [], []
    for _ in range(RUNS):
        s, m = run(cmd)
        if s is None:
            print(f"{name}: marker never shown", file=sys.stderr)
            break
        starts.append(s * 1000)
        mems.append(m)
    if starts:
        print(f"{name:10} {statistics.median(starts):9.0f} ms {statistics.median(mems):9.1f} MiB")
tmux("kill-server")
