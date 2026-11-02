#!/usr/bin/env bash
# Startup and memory benchmark (spec §23.1, §23.7).
#
# Runs `yace --startup-time` N times inside a pseudo-terminal (via `script`)
# and reports time-to-first-frame and resident memory of the editor process
# only (no language servers, shells or plugins exist yet).
#
# The enhanced-keyboard capability probe is disabled for the benchmark
# because `script` does not answer terminal queries; in a real terminal the
# probe adds one query/response round trip.
#
# Usage: scripts/bench.sh [RUNS] [PATH_TO_OPEN]
set -euo pipefail
RUNS="${1:-20}"
TARGET="${2:-$(pwd)}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cargo build --release --manifest-path "$ROOT/Cargo.toml" -p yace >/dev/null
BIN="$ROOT/target/release/yace"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/config"
printf '[keyboard]\nenhancedProtocol = false\n' > "$TMP/config/config.toml"
export YACE_CONFIG_DIR="$TMP/config" YACE_STATE_DIR="$TMP/state" YACE_CACHE_DIR="$TMP/cache"
times=()
rss=()
for _ in $(seq "$RUNS"); do
  out="$(script -qec "stty cols 120 rows 40; '$BIN' --startup-time --no-restore '$TARGET'" /dev/null | tr -d '\r')"
  times+=("$(grep -o 'first_frame_ms=[0-9.]*' <<<"$out" | cut -d= -f2)")
  rss+=("$(grep -o 'rss_kib=[0-9]*' <<<"$out" | cut -d= -f2)")
done
printf '%s\n' "${times[@]}" | sort -n | awk '
  { a[NR]=$1; s+=$1 }
  END {
    p95 = int(NR * 0.95 + 0.5); if (p95 > NR) p95 = NR; if (p95 < 1) p95 = 1
    printf "startup (first frame), %d runs: min %.2f ms  median %.2f ms  p95 %.2f ms  mean %.2f ms\n", NR, a[1], a[int((NR+1)/2)], a[p95], s/NR
  }'
printf '%s\n' "${rss[@]}" | sort -n | awk '{ a[NR]=$1 } END { printf "resident memory at first frame: median %.1f MiB (min %.1f, max %.1f)\n", a[int((NR+1)/2)]/1024, a[1]/1024, a[NR]/1024 }'
echo "binary size: $(du -h "$BIN" | cut -f1)"
uname -srm
