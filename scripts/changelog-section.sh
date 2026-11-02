#!/usr/bin/env bash
# Print the CHANGELOG.md section for a version (used for tag messages and
# GitHub release notes).  Usage: scripts/changelog-section.sh 0.1.0
set -euo pipefail
v="${1:?usage: changelog-section.sh VERSION}"
root="$(cd "$(dirname "$0")/.." && pwd)"
awk -v v="$v" '
  $0 ~ "^## \\[" v "\\]" { on = 1; next }
  on && /^## \[/ { exit }
  on && /^\[[^]]+\]: / { exit }
  on { print }
' "$root/CHANGELOG.md" | sed -e '/./,$!d' | sed -e ':a' -e '/^\n*$/{$d;N;ba' -e '}'
