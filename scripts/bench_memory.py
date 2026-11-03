#!/usr/bin/env python3
"""Memory benchmark (spec §23.2): resident memory of the editor process
after Quick Open indexing and a workspace search, then after idling.

Usage: scripts/bench_memory.py WORKSPACE_OR_FOLDER [SEARCH_TERM]
Set YACE_MALLOC_TRIM=0 to measure without heap trimming (A/B).
"""
import os, pty, select, subprocess, sys, tempfile, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
target = sys.argv[1]
term = sys.argv[2] if len(sys.argv) > 2 else "import"
subprocess.run(["cargo", "build", "--release", "-q", "-p", "yace"], cwd=ROOT, check=True)
binary = os.path.join(ROOT, "target", "release", "yace")
tmp = tempfile.mkdtemp()
os.makedirs(f"{tmp}/config")
with open(f"{tmp}/config/config.toml", "w") as f:
    f.write("[keyboard]\nenhancedProtocol = false\n" + os.environ.get("YACE_BENCH_EXTRA_CONFIG", ""))
env = dict(os.environ, YACE_CONFIG_DIR=f"{tmp}/config", YACE_STATE_DIR=f"{tmp}/state",
           YACE_CACHE_DIR=f"{tmp}/cache", TERM="xterm-256color")
env.pop("DISPLAY", None); env.pop("WAYLAND_DISPLAY", None)
pid, fd = pty.fork()
if pid == 0:
    os.execve(binary, [binary, "--no-restore", target], env)

def drain(t):
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try: os.read(fd, 1 << 16)
            except OSError: return

def rss():
    with open(f"/proc/{pid}/status") as f:
        for l in f:
            if l.startswith("VmRSS:"):
                return int(l.split()[1]) / 1024
    return 0.0

def hwm():
    with open(f"/proc/{pid}/status") as f:
        for l in f:
            if l.startswith("VmHWM:"):
                return int(l.split()[1]) / 1024
    return 0.0

drain(1.5)
print(f"after startup:            {rss():6.1f} MiB")
os.write(fd, b"\x10"); drain(0.3)          # Ctrl+P
os.write(fd, b"index"); drain(6.0)         # let indexing finish
os.write(fd, b"\x1b"); drain(0.5)
print(f"after Quick Open index:   {rss():6.1f} MiB")
os.write(fd, b"\x1bOP"); drain(0.3)       # F1: command palette
os.write(fd, b"Find in Files"); drain(0.3)
os.write(fd, b"\r"); drain(0.3)
# Type the query slowly, like a person: search-as-you-type restarts.
for ch in term:
    os.write(fd, ch.encode()); drain(0.25)
os.write(fd, b"\x7f" * len(term)); drain(0.3)
for ch in term:
    os.write(fd, ch.encode()); drain(0.25)
os.write(fd, b"\r"); drain(6.0)            # let search finish
print(f"after search '{term}': {rss():6.1f} MiB")
drain(3.0)
print(f"after 3 s idle:           {rss():6.1f} MiB   (peak {hwm():.1f} MiB)")
os.write(fd, b"\x11"); drain(0.5); os.write(fd, b"d")
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass
