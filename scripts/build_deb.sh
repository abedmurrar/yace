#!/usr/bin/env bash
# Build the Debian package of an already built release binary.
#   cargo build --release --locked -p yace && scripts/build_deb.sh [OUT_DIR]
# Needs cargo-deb (`cargo install cargo-deb --locked`). Writes
# yace_<version>-1_<arch>.deb into OUT_DIR (default: dist/).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
out="${1:-dist}"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
# The binary must be the one this version builds (not a stale build).
built="$(target/release/yace --version | awk '{print $2}')"
[ "$built" = "$version" ] || { echo "target/release/yace is $built, Cargo.toml says $version: rebuild first" >&2; exit 1; }
extra="target/deb-extra"
rm -rf "$extra"
mkdir -p "$extra" "$out"

# Debian changelog (one entry per package version), and the upstream
# changelog under the name Debian expects.
date="$(date -R -u -d "@${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}")"
cat > "$extra/changelog.Debian" <<EOF
yace (${version}-1) stable; urgency=medium

  * YACE ${version}. See /usr/share/doc/yace/changelog.gz for what changed.

 -- Abed Murrar <abedjmurrar@gmail.com>  ${date}
EOF
gzip -9n < CHANGELOG.md > "$extra/changelog.gz"
gzip -9n < CREDITS.md > "$extra/CREDITS.md.gz"
gzip -9n < THIRD_PARTY_LICENSES.md > "$extra/THIRD_PARTY_LICENSES.md.gz"
gzip -9n < README.md > "$extra/README.md.gz"

cargo deb -p yace --no-build --no-strip -o "$out/"
