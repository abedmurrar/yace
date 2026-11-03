#!/usr/bin/env python3
"""Idle CPU and typing latency benchmark (spec §23.1).

Runs the release binary in a pseudo-terminal, measures CPU time consumed
while idle, then types N characters and reads the editor's own per-event
input-handling and render latencies (YACE_METRICS_FILE).

Usage: scripts/bench_interactive.py [N_KEYS] [FILE]

FILE (default: a generated 6,000-line Rust file) is copied and opened, so
syntax highlighting is exercised. YACE_BENCH_BIN overrides the binary
(e.g. to compare with an older release).
"""
import os, pty, sys, tempfile, time, select, subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
N = int(sys.argv[1]) if len(sys.argv) > 1 else 300
binary = os.environ.get("YACE_BENCH_BIN")
if not binary:
    subprocess.run(["cargo", "build", "--release", "-q", "-p", "yace"], cwd=ROOT, check=True)
    binary = os.path.join(ROOT, "target", "release", "yace")

tmp = tempfile.mkdtemp()
os.makedirs(f"{tmp}/config")
with open(f"{tmp}/config/config.toml", "w") as f:
    f.write("[keyboard]\nenhancedProtocol = false\n")
if len(sys.argv) > 2:
    import shutil
    target = os.path.join(tmp, os.path.basename(sys.argv[2]))
    shutil.copy(sys.argv[2], target)
else:
    target = f"{tmp}/sample.rs"
    with open(target, "w") as f:
        f.write("fn main() {\n    println!(\"hello\");\n}\n" * 2000)
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
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try: os.read(fd, 65536)
            except OSError: return

def cpu_ticks():
    with open(f"/proc/{pid}/stat") as f:
        parts = f.read().rsplit(")", 1)[1].split()
    return int(parts[11]) + int(parts[12])  # utime + stime

drain(1.5)
hz = os.sysconf("SC_CLK_TCK")
t0, c0 = time.time(), cpu_ticks()
drain(5.0)
idle_cpu = (cpu_ticks() - c0) / hz / (time.time() - t0) * 100

for i in range(N):
    os.write(fd, b"x" if i % 20 else b"\r")
    drain(0.01)
drain(0.5)
os.write(fd, b"\x11")  # Ctrl+Q
drain(0.5)
os.write(fd, b"d")     # Don't Save
deadline = time.time() + 10
while time.time() < deadline:
    done, _ = os.waitpid(pid, os.WNOHANG)
    if done: break
    drain(0.1)

print(f"idle CPU over 5 s: {idle_cpu:.2f}% of one core")
with open(metrics) as f:
    vals = dict(l.strip().split("=") for l in f if "=" in l)
print(f"typing ({N} keys into {os.path.basename(target)}): input handling p50 {vals['input_p50_ms']} ms, p95 {vals['input_p95_ms']} ms, max {vals['input_max_ms']} ms")
print(f"render: p50 {vals['render_p50_ms']} ms, p95 {vals['render_p95_ms']} ms, max {vals['render_max_ms']} ms ({vals['frames']} frames)")
