#!/usr/bin/env bash
# The bundled FreeType (freetype-sys "bundled", used on Windows) includes <zlib.h> but does not
# add zlib's include folder. Build libz-sys first and point the C compiler at its headers.
# Usage: scripts/windows-zlib.sh [cargo build flags, e.g. --release --target <triple>]
# Prints `CFLAGS=...` and appends it to $GITHUB_ENV when that is set.
set -euo pipefail
cargo build -q -p libz-sys "$@"
inc=$(ls -d target/build/libz-sys-*/out/include target/*/build/libz-sys-*/out/include \
  target/*/*/build/libz-sys-*/out/include 2>/dev/null | head -1 || true)
if [ -z "$inc" ]; then
  echo "libz-sys found a system zlib; nothing to do"
  exit 0
fi
# Absolute: the C compiler runs in each -sys crate's own folder.
inc=$(cd "$inc" && pwd)
if command -v cygpath >/dev/null; then inc=$(cygpath -w "$inc"); fi
echo "CFLAGS=-I$inc"
if [ -n "${GITHUB_ENV:-}" ]; then echo "CFLAGS=-I$inc" >> "$GITHUB_ENV"; fi
