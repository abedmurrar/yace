#!/usr/bin/env bash
# Build a plugin crate into an installable folder and package.
#   scripts/build_plugin.sh plugins/hello [OUT_DIR]     (default OUT_DIR: dist/)
# -> OUT_DIR/<id>/            yace-plugin.toml, lib/<library>, and any themes/,
#                              queries/, grammars/ or assets/ folders of the crate
# -> OUT_DIR/<id>-<version>.tar.gz   (via `yace --package-plugin`, which validates it)
# Then: "Extensions: Install from Package…" or `yace --install-plugin <package>`.
set -euo pipefail
crate_dir="$(cd "${1:?usage: build_plugin.sh PLUGIN_DIR [OUT_DIR]}" && pwd)"
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$(mkdir -p "${2:-$root/dist}" && cd "${2:-$root/dist}" && pwd)"
manifest="$crate_dir/yace-plugin.toml"
id="$(sed -n 's/^id = "\(.*\)"/\1/p' "$manifest" | head -1)"
pkg="$(sed -n 's/^name = "\(.*\)"/\1/p' "$crate_dir/Cargo.toml" | head -1)"
library="$(sed -n 's/^library = "\(.*\)"/\1/p' "$manifest" | head -1)"
cargo build --release --manifest-path "$root/Cargo.toml" -p "$pkg" -p yace
dest="$out/$id"
rm -rf "$dest" && mkdir -p "$dest"
cp "$manifest" "$dest/"
if [[ -n "$library" ]]; then
  mkdir -p "$dest/$(dirname "$library")"
  cp "$root/target/release/$(basename "$library")" "$dest/$library"
fi
for extra in themes queries grammars assets LICENSE README.md; do
  [[ -e "$crate_dir/$extra" ]] && cp -r "$crate_dir/$extra" "$dest/"
done
cd "$out"
"$root/target/release/yace" --package-plugin "$dest" -o "$out/$id-$(sed -n 's/^version = "\(.*\)"/\1/p' "$manifest" | head -1).tar.gz"
