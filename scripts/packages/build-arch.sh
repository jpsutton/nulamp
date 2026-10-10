#!/usr/bin/env bash
# Builds the Arch package and a signed pacman repository database "nulamp" in dist/arch
# (CI runs it in an archlinux:base-devel container as root). The signing key comes from the
# NULAMP_SIGNING_KEY environment variable (an ASCII-armoured private key); without it the
# package and database are unsigned.
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
sign=()
if [ -n "${NULAMP_SIGNING_KEY:-}" ]; then
  export GNUPGHOME="$(mktemp -d)"
  printf '%s\n' "$NULAMP_SIGNING_KEY" | gpg --batch --quiet --import
  key=$(gpg --list-secret-keys --with-colons | awk -F: '/^fpr/{print $10; exit}')
  gpg --batch --yes --detach-sign --local-user "$key" "$pkg"
  sign=(--sign --key "$key")
fi
repo-add "${sign[@]}" nulamp.db.tar.gz "$pkg"
# Release assets cannot be symlinks: pacman fetches nulamp.db and nulamp.files.
for f in nulamp.db nulamp.files; do
  rm -f "$f" "$f.sig"
  cp "$f.tar.gz" "$f"
  [ -f "$f.tar.gz.sig" ] && cp "$f.tar.gz.sig" "$f.sig"
done
rm -f nulamp.db.tar.gz* nulamp.files.tar.gz*
ls -la
