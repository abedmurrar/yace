#!/usr/bin/env bash
# Build a plugin crate into an installable folder and package.
#   scripts/build_plugin.sh PLUGIN_DIR [OUT_DIR]        (default OUT_DIR: dist/)
# PLUGIN_DIR is a crate with a yace-plugin.toml: one of YACE's own plugins
# (plugins/hello) or your own crate anywhere on disk.
# -> OUT_DIR/<id>/            yace-plugin.toml, lib/<library>, and any themes/,
#                              queries/, grammars/ or assets/ folders of the crate
# -> OUT_DIR/<id>-<version>.tar.gz   (via `yace --package-plugin`, which validates it)
# Then: "Extensions: Install from Package…" or `yace --install-plugin <package>`.
# Outside the YACE repository, packaging uses the `yace` on PATH (or $YACE).
set -euo pipefail
crate_dir="$(cd "${1:?usage: build_plugin.sh PLUGIN_DIR [OUT_DIR]}" && pwd)"
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$(mkdir -p "${2:-$root/dist}" && cd "${2:-$root/dist}" && pwd)"
manifest="$crate_dir/yace-plugin.toml"
[[ -f "$manifest" ]] || { echo "$crate_dir has no yace-plugin.toml" >&2; exit 1; }
id="$(sed -n 's/^id = "\(.*\)"/\1/p' "$manifest" | head -1)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$manifest" | head -1)"
library="$(sed -n 's/^library = "\(.*\)"/\1/p' "$manifest" | head -1)"

meta="$(cargo metadata --no-deps --format-version 1 --manifest-path "$crate_dir/Cargo.toml")"
target_dir="$(sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' <<<"$meta")"
workspace_root="$(sed -n 's/.*"workspace_root":"\([^"]*\)".*/\1/p' <<<"$meta")"
if [[ "$workspace_root" == "$root" ]]; then
  # One of YACE's own plugins: build it and the yace that packages it.
  pkg="$(sed -n 's/^name = "\(.*\)"/\1/p' "$crate_dir/Cargo.toml" | head -1)"
  cargo build --release --manifest-path "$root/Cargo.toml" -p "$pkg" -p yace
  yace="$root/target/release/yace"
else
  cargo build --release --manifest-path "$crate_dir/Cargo.toml" --lib
  yace="${YACE:-$(command -v yace || true)}"
  [[ -x "$yace" ]] || { echo "yace not found: install it or set YACE=/path/to/yace" >&2; exit 1; }
fi

dest="$out/$id"
rm -rf "$dest" && mkdir -p "$dest"
cp "$manifest" "$dest/"
if [[ -n "$library" ]]; then
  mkdir -p "$dest/$(dirname "$library")"
  cp "$target_dir/release/$(basename "$library")" "$dest/$library"
fi
for extra in themes queries grammars assets LICENSE README.md; do
  [[ -e "$crate_dir/$extra" ]] && cp -r "$crate_dir/$extra" "$dest/"
done
cd "$out"
"$yace" --package-plugin "$dest" -o "$out/$id-$version.tar.gz"
