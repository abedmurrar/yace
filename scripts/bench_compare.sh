#!/usr/bin/env bash
# Compare YACE with Helix, Micro and Neovim in one clean Ubuntu 24.04
# container: time to first frame and idle memory, each editor with its
# defaults, no config and no language server. See bench_compare.py.
#   scripts/bench_compare.sh            (needs Docker; downloads the editors)
# The versions are pinned here; update them to compare newer releases.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
HELIX=25.07.1
MICRO=2.0.15
NEOVIM=v0.12.6
docker run --rm -i -v "$root:/yace:ro" ubuntu:24.04 bash -s <<EOF
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq curl ca-certificates xz-utils python3 tmux >/dev/null
cd /opt
curl -fsSL https://abedmurrar.github.io/yace/install.sh | sh >/dev/null 2>&1
curl -fsSL https://github.com/helix-editor/helix/releases/download/$HELIX/helix-$HELIX-x86_64-linux.tar.xz | tar xJ
ln -s /opt/helix-$HELIX-x86_64-linux/hx /usr/local/bin/hx
export HELIX_RUNTIME=/opt/helix-$HELIX-x86_64-linux/runtime
curl -fsSL https://github.com/zyedidia/micro/releases/download/v$MICRO/micro-$MICRO-linux64.tar.gz | tar xz
ln -s /opt/micro-$MICRO/micro /usr/local/bin/micro
curl -fsSL https://github.com/neovim/neovim/releases/download/$NEOVIM/nvim-linux-x86_64.tar.gz | tar xz
ln -s /opt/nvim-linux-x86_64/bin/nvim /usr/local/bin/nvim
echo "yace \$(yace --version | cut -d' ' -f2), helix \$(hx --version | cut -d' ' -f2), micro \$(micro -version | sed -n 's/Version: //p'), neovim \$(nvim --version | head -1 | cut -d' ' -f2)"
cp /yace/crates/yace/src/ui.rs /tmp/ui.rs
cd /tmp && python3 /yace/scripts/bench_compare.py ui.rs Rendering yace=yace helix=hx micro=micro neovim=nvim
EOF
