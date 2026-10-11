#!/usr/bin/env bash
# Builds nulamp-<version>-x86_64.AppImage in dist/ from the dependencies made by
# build-deps.sh ($PREFIX, default ./appimage-deps).
#
# Bundled: nulamp, the LGPL FFmpeg libraries and libopenmpt (with their licences and the exact
# FFmpeg source next to the AppImage). Libraries the AppImage project expects every host to
# provide (glibc, ALSA, X11, fontconfig, FreeType, ...) are not bundled.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
. "$here/versions.sh"
PREFIX="$(realpath -m "${PREFIX:-appimage-deps}")"
TOOLS="$(realpath -m "${TOOLS:-appimage-tools}")"
cd "$root"
version=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p}' Cargo.toml)

mkdir -p "$TOOLS"
tool() { # url sha256 name
  if [ ! -x "$TOOLS/$3" ]; then
    curl -sSfL --retry 3 -o "$TOOLS/$3" "$1"
    echo "$2  $TOOLS/$3" | sha256sum -c --quiet
    chmod +x "$TOOLS/$3"
  fi
}
tool "$LINUXDEPLOY_URL" "$LINUXDEPLOY_SHA256" linuxdeploy
tool "$APPIMAGETOOL_URL" "$APPIMAGETOOL_SHA256" appimagetool
tool "$RUNTIME_URL" "$RUNTIME_SHA256" runtime-x86_64
# No FUSE in containers and CI: let the tools unpack themselves.
export APPIMAGE_EXTRACT_AND_RUN=1

export FFMPEG_DIR="$PREFIX" OPENMPT_LIB_DIR="$PREFIX/lib"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
cargo build --release --locked -p nulamp ${CARGO_TARGET:+--target "$CARGO_TARGET"}
bin="target/${CARGO_TARGET:+$CARGO_TARGET/}release/nulamp"

# The binary must need exactly the bundled libraries (a distribution FFmpeg or libopenmpt on the
# build machine could otherwise win the link).
for lib in libavformat libavcodec libavutil libswresample libswscale libopenmpt; do
  soname=$(readelf -d "$PREFIX/lib/$lib.so" | sed -n 's/.*SONAME.*\[\(.*\)\]/\1/p')
  readelf -d "$bin" | grep -q "NEEDED.*\[$soname\]" || {
    echo "nulamp does not link the bundled $soname:" >&2
    readelf -d "$bin" | grep NEEDED >&2
    exit 1
  }
done

appdir="$(mktemp -d)/nulamp.AppDir"
mkdir -p "$appdir/usr/share/doc/nulamp/third-party"
cp README.md LICENSE-MIT LICENSE-APACHE "$appdir/usr/share/doc/nulamp/"
cp "$PREFIX/src/COPYING.LGPLv2.1" "$appdir/usr/share/doc/nulamp/third-party/FFmpeg-LGPL-2.1.txt"
cat > "$appdir/usr/share/doc/nulamp/third-party/README" <<TXT
This AppImage bundles:
- FFmpeg ${FFMPEG_VERSION} (libavformat, libavcodec, libavutil, libswresample, libswscale),
  LGPL-2.1-or-later,
  built without GPL or non-free parts. Its exact source is published next to this AppImage
  (ffmpeg-${FFMPEG_VERSION}.tar.xz) and at ${FFMPEG_URL}. Configuration: scripts/appimage/build-deps.sh
  in the nulamp repository. You may replace the bundled libraries with your own build.
- libopenmpt ${OPENMPT_VERSION}, BSD-3-Clause (${OPENMPT_URL}).
TXT
LD_LIBRARY_PATH="$PREFIX/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$TOOLS/linuxdeploy" \
  --appdir "$appdir" \
  --executable "$bin" \
  --desktop-file assets/linux/nulamp.desktop \
  --icon-file assets/linux/nulamp.png \
  --exclude-library 'libbrotli*' --exclude-library 'libbz2*' --exclude-library 'libpng*' \
  > /dev/null
# Only nulamp's own libraries: everything else comes from the host.
extra=$(ls "$appdir/usr/lib" | grep -vE '^lib(avformat|avcodec|avutil|swresample|swscale|openmpt)\.so' || true)
if [ -n "$extra" ]; then
  echo "unexpected bundled libraries: $extra" >&2
  exit 1
fi
for lib in libavformat libavcodec libavutil libswresample libswscale libopenmpt; do
  ls "$appdir"/usr/lib/"$lib".so.* > /dev/null || { echo "$lib was not bundled" >&2; exit 1; }
done

mkdir -p dist
out="dist/nulamp-${version}-x86_64.AppImage"
ARCH=x86_64 "$TOOLS/appimagetool" --no-appstream --runtime-file "$TOOLS/runtime-x86_64" \
  "$appdir" "$out" > /dev/null
cp "$PREFIX/src/ffmpeg-${FFMPEG_VERSION}.tar.xz" dist/
echo "built $out"
ls -la dist
