#!/usr/bin/env bash
# Builds the Arch package and its detached signature in dist/arch (CI runs it in an
# archlinux:base-devel container as root). The signing key comes from the NULAMP_SIGNING_KEY
# environment variable (an ASCII-armoured private key); without it the package is unsigned.
# The jpsutton pacman repository (jpsutton/arch-repo) takes the package from the release and
# accepts it only with a valid signature.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
pacman -Syu --noconfirm --needed base-devel rust pkgconf cmake ffmpeg libopenmpt freetype2 \
  fontconfig alsa-lib libx11 libxcursor libxrandr libxi libxkbcommon-x11 git > /dev/null
# makepkg refuses to run as root.
id builder > /dev/null 2>&1 || useradd -m builder
chown -R builder "$root"
su builder -c "cd '$root/packaging/arch' && PKGDEST='$root/dist/arch' makepkg --noconfirm --cleanbuild"
out="$root/dist/arch"
cd "$out"
pkg=$(ls nulamp-*.pkg.tar.zst)
if [ -n "${NULAMP_SIGNING_KEY:-}" ]; then
  export GNUPGHOME="$(mktemp -d)"
  printf '%s\n' "$NULAMP_SIGNING_KEY" | gpg --batch --quiet --import
  key=$(gpg --list-secret-keys --with-colons | awk -F: '/^fpr/{print $10; exit}')
  gpg --batch --yes --detach-sign --local-user "$key" "$pkg"
fi
ls -la
