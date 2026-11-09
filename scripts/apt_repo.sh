#!/usr/bin/env bash
# Build a signed APT repository from Debian packages.
#   scripts/apt_repo.sh DEBS_DIR OUT_DIR KEY_ID
# DEBS_DIR holds yace_*.deb files (any architectures). The repository is
# written to OUT_DIR:
#   pool/main/y/yace/*.deb
#   dists/stable/main/binary-{amd64,arm64}/Packages(.gz)
#   dists/stable/Release, InRelease, Release.gpg
#   yace.gpg (public key, binary, for `signed-by`) and yace.asc (armored)
# KEY_ID is the gpg key that signs (it must be in the keyring). A key with
# a passphrase reads it from GPG_PASSPHRASE.
set -euo pipefail
debs="${1:?usage: apt_repo.sh DEBS_DIR OUT_DIR KEY_ID}"
out="${2:?usage: apt_repo.sh DEBS_DIR OUT_DIR KEY_ID}"
key="${3:?usage: apt_repo.sh DEBS_DIR OUT_DIR KEY_ID}"
suite=stable
component=main
archs=(amd64 arm64)

shopt -s nullglob
files=("$debs"/yace_*.deb)
[ ${#files[@]} -gt 0 ] || { echo "no yace_*.deb in $debs" >&2; exit 1; }

rm -rf "$out"
mkdir -p "$out/pool/$component/y/yace"
cp "${files[@]}" "$out/pool/$component/y/yace/"
cd "$out"
for arch in "${archs[@]}"; do
    dir="dists/$suite/$component/binary-$arch"
    mkdir -p "$dir"
    apt-ftparchive --arch "$arch" packages pool > "$dir/Packages"
    gzip -9nk "$dir/Packages"
done
apt-ftparchive \
    -o "APT::FTPArchive::Release::Origin=YACE" \
    -o "APT::FTPArchive::Release::Label=YACE" \
    -o "APT::FTPArchive::Release::Suite=$suite" \
    -o "APT::FTPArchive::Release::Codename=$suite" \
    -o "APT::FTPArchive::Release::Components=$component" \
    -o "APT::FTPArchive::Release::Architectures=${archs[*]}" \
    -o "APT::FTPArchive::Release::Description=YACE, a fast terminal code editor" \
    release "dists/$suite" > "dists/$suite/Release"

sign=(gpg --batch --yes --local-user "$key" --digest-algo SHA512)
if [ -n "${GPG_PASSPHRASE:-}" ]; then
    sign+=(--pinentry-mode loopback --passphrase-fd 0)
fi
"${sign[@]}" --clearsign -o "dists/$suite/InRelease" "dists/$suite/Release" <<<"${GPG_PASSPHRASE:-}"
"${sign[@]}" --armor --detach-sign -o "dists/$suite/Release.gpg" "dists/$suite/Release" <<<"${GPG_PASSPHRASE:-}"
gpg --batch --export "$key" > yace.gpg
gpg --batch --armor --export "$key" > yace.asc
echo "APT repository in $out: $(ls pool/$component/y/yace | wc -l) package(s)"
