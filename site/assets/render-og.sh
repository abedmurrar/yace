#!/bin/sh
# Render the share image site/src/og.png from site/assets/og.svg (needs
# rsvg-convert and DejaVu fonts, e.g. Debian's librsvg2-bin fonts-dejavu-core).
#   docker run --rm -v "$PWD/site:/site" debian:12 sh -c 'apt-get update -qq && \
#     apt-get install -y -qq librsvg2-bin fonts-dejavu-core && /site/assets/render-og.sh'
set -eu
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
rsvg-convert -w 1160 ../src/media/overview.svg -o "$tmp/overview.png"
svg="$(cat og.svg)"
{
    printf '%s' "${svg%%OVERVIEW_PNG*}data:image/png;base64,"
    base64 -w0 "$tmp/overview.png"
    printf '%s\n' "${svg#*OVERVIEW_PNG}"
} > "$tmp/og.svg"
rsvg-convert "$tmp/og.svg" -o ../src/og.png
rm -rf "$tmp"
echo "wrote site/src/og.png"
