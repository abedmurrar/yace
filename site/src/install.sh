#!/bin/sh
# Install YACE from its signed APT repository (Ubuntu 22.04+, Debian 12+):
#
#   curl -fsSL https://abedmurrar.github.io/yace/install.sh | sudo sh
#
# It adds the repository's key and source (the same files the yace package
# installs, so `sudo apt upgrade` keeps YACE up to date), then installs yace.
set -eu

site=https://abedmurrar.github.io/yace

if [ "$(id -u)" -ne 0 ]; then
    echo "Run it with sudo:  curl -fsSL $site/install.sh | sudo sh" >&2
    exit 1
fi
if ! command -v apt-get >/dev/null 2>&1; then
    echo "This installer needs apt (Ubuntu, Debian). Other systems: $site/installation.html" >&2
    exit 1
fi
arch="$(dpkg --print-architecture)"
case "$arch" in
    amd64 | arm64) ;;
    *)
        echo "There is no YACE package for $arch yet. See $site/installation.html" >&2
        exit 1
        ;;
esac
fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "$1"
    else
        echo "Needs curl or wget." >&2
        exit 1
    fi
}

echo "Adding the YACE APT repository..."
install -d -m 755 /usr/share/keyrings /etc/apt/sources.list.d
fetch "$site/apt/yace.gpg" > /usr/share/keyrings/yace.gpg.new
mv /usr/share/keyrings/yace.gpg.new /usr/share/keyrings/yace.gpg
chmod 644 /usr/share/keyrings/yace.gpg
cat > /etc/apt/sources.list.d/yace.sources <<'EOF'
# YACE's APT repository, installed by the yace package so that
# `sudo apt upgrade` keeps YACE up to date. https://abedmurrar.github.io/yace/
Types: deb
URIs: https://abedmurrar.github.io/yace/apt
Suites: stable
Components: main
Architectures: amd64 arm64
Signed-By: /usr/share/keyrings/yace.gpg
EOF
rm -f /etc/apt/sources.list.d/yace.list

echo "Installing yace..."
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y yace
echo
yace --version
echo "Done. Start it with: yace .   (updates come with: sudo apt upgrade)"
