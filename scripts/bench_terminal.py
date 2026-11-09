#!/usr/bin/env python3
"""Integrated terminal benchmark (spec §11.7, §23.2).

Runs `seq 1 N` in the panel and types into the editor *while* the
terminal floods, then reports the editor's own input/render latency and
resident memory of the editor process (shells are separate processes).

Usage: scripts/bench_terminal.py [N_LINES]
YACE_BENCH_SPLIT=1 splits the terminal first and floods the new pane (two
panes drawn); YACE_BENCH_BIN overrides the binary.
"""
import os, pty, select, subprocess, sys, tempfile, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
N = int(sys.argv[1]) if len(sys.argv) > 1 else 2_000_000
binary = os.environ.get("YACE_BENCH_BIN")
if not binary:
    subprocess.run(["cargo", "build", "--release", "-q", "-p", "yace"], cwd=ROOT, check=True)
    binary = os.path.join(ROOT, "target", "release", "yace")
tmp = tempfile.mkdtemp()
os.makedirs(f"{tmp}/config")
with open(f"{tmp}/config/config.toml", "w") as f:
    f.write('[keyboard]\nenhancedProtocol = false\n[terminal]\nshell = "/bin/sh"\n[terminal.env]\nPS1 = "$ "\nENV = ""\n')
target = f"{tmp}/notes.txt"
open(target, "w").write("notes\n")
metrics = f"{tmp}/metrics.txt"
env = dict(os.environ, YACE_CONFIG_DIR=f"{tmp}/config", YACE_STATE_DIR=f"{tmp}/state",
           YACE_CACHE_DIR=f"{tmp}/cache", YACE_METRICS_FILE=metrics, TERM="xterm-256color")
env.pop("DISPLAY", None); env.pop("WAYLAND_DISPLAY", None)
pid, fd = pty.fork()
if pid == 0:
    os.execve(binary, [binary, "--no-restore", target], env)

def drain(t):
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.02)
        if r:
            try: os.read(fd, 1 << 16)
            except OSError: return

def rss():
    for l in open(f"/proc/{pid}/status"):
        if l.startswith("VmRSS:"):
            return int(l.split()[1]) / 1024
    return 0.0

def palette(cmd):
    os.write(fd, b"\x1bOP"); drain(0.3)
    os.write(fd, cmd.encode()); drain(0.3)
    os.write(fd, b"\r"); drain(0.6)

drain(1.5)
base = rss()
os.write(fd, b"\x1b`"); drain(1.0)               # Alt+`: open terminal
if os.environ.get("YACE_BENCH_SPLIT"):
    os.write(fd, b"\x1b\\"); drain(1.0)          # Alt+\\: split, the new pane has focus
one = rss()
start = time.time()
os.write(fd, f"seq 1 {N}; echo FLOOD-DONE\r".encode())
drain(0.3)
os.write(fd, b"\x1b[17~"); drain(0.2)             # F6: focus the editor
typed = 0
while time.time() - start < 6.0:                  # type during the flood
    os.write(fd, b"x"); typed += 1
    drain(0.02)
flood_rss = rss()
for _ in range(3):
    palette("Create New Terminal")
four = rss()
os.write(fd, b"\x1b[17~"); drain(0.3)
palette("Quit")
drain(0.5)
os.write(fd, b"k"); drain(0.5)                    # Kill and Continue (if asked)
os.write(fd, b"d"); drain(0.5)                    # Don't Save
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass
vals = dict(l.strip().split("=") for l in open(metrics) if "=" in l)
print(f"RSS: idle {base:.1f} MiB · 1 terminal {one:.1f} MiB · during flood {flood_rss:.1f} MiB · 4 terminals {four:.1f} MiB (editor process only)")
print(f"typed {typed} keys into the editor while `seq 1 {N}` flooded the terminal:")
print(f"  input handling p50 {vals['input_p50_ms']} ms · p95 {vals['input_p95_ms']} ms · max {vals['input_max_ms']} ms")
print(f"  render p50 {vals['render_p50_ms']} ms · p95 {vals['render_p95_ms']} ms · max {vals['render_max_ms']} ms · {vals['frames']} frames")
