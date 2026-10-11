#!/usr/bin/env bash
# Builds the codec libraries of the Windows builds with vcpkg (MSVC, x64, DLLs):
#
# - FFmpeg (libavformat, libavcodec, libavutil, libswresample, libswscale) as an LGPL build:
#   vcpkg's port only enables GPL or non-free code through its `gpl` and `nonfree` features,
#   which are not requested.
# - libopenmpt (BSD-3-Clause) with its dependencies mpg123 (LGPL-2.1), libogg, libvorbis and
#   zlib.
#
# Usage: scripts/windows-codecs.sh <work dir> [<dist folder>]
#   Builds into <work dir> (vcpkg checkout, binary cache in <work dir>/cache) and prints the
#   FFMPEG_DIR and OPENMPT_LIB_DIR to build nulamp with, and the DLL folder to add to PATH. In
#   GitHub Actions it sets them for the following steps.
#   With <dist folder>, copies the DLLs and their licences there and writes the sources of the
#   LGPL libraries to <dist folder>/../<name>-lgpl-sources.tar.gz.
# Run from Git Bash on Windows.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/windows-codecs-versions.sh"
work="$(mkdir -p "$1" && cd "$1" && pwd)"
vcpkg="$work/vcpkg"
if [ ! -x "$vcpkg/vcpkg.exe" ]; then
  rm -rf "$vcpkg"
  git clone -q --depth 1 --branch "$VCPKG_TAG" https://github.com/microsoft/vcpkg "$vcpkg"
  cmd //c "$(cygpath -w "$vcpkg/bootstrap-vcpkg.bat")" -disableMetrics > /dev/null
fi
mkdir -p "$work/cache"
export VCPKG_BINARY_SOURCES="clear;files,$(cygpath -w "$work/cache"),readwrite"
export VCPKG_DISABLE_METRICS=1
ports=("ffmpeg[core,avcodec,avformat,swresample,swscale,zlib]" libopenmpt)
args=(--triplet x64-windows --x-install-root="$(cygpath -w "$work/installed")"
  --clean-after-build)
"$vcpkg/vcpkg.exe" install "${args[@]}" "${ports[@]}"
inst="$work/installed/x64-windows"

# The licence FFmpeg was configured with is compiled into libavcodec.
avcodec=$(ls "$inst"/bin/avcodec-*.dll)
if ! grep -aq "LGPL version 2.1 or later" "$avcodec"; then
  echo "FFmpeg is not an LGPL build: $avcodec" >&2
  exit 1
fi

env_lines="FFMPEG_DIR=$(cygpath -w "$inst")
OPENMPT_LIB_DIR=$(cygpath -w "$inst/lib")"
echo "$env_lines"
echo "add to PATH: $(cygpath -w "$inst/bin")"
if [ -n "${GITHUB_ENV:-}" ]; then
  echo "$env_lines" >> "$GITHUB_ENV"
  cygpath -w "$inst/bin" >> "$GITHUB_PATH"
fi

[ $# -ge 2 ] || exit 0
dist="$(mkdir -p "$2" && cd "$2" && pwd)"
cp "$inst"/bin/*.dll "$dist/"
third="$dist/third-party"
mkdir -p "$third"
for port in ffmpeg libopenmpt mpg123 libogg libvorbis zlib; do
  cp "$inst/share/$port/copyright" "$third/$port-copyright.txt"
done
ffmpeg_version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$vcpkg/ports/ffmpeg/vcpkg.json")
mpg123_version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$vcpkg/ports/mpg123/vcpkg.json")
cat > "$third/README.txt" <<TXT
nulamp for Windows includes these libraries, built with vcpkg ${VCPKG_TAG} (MSVC, x64):

- FFmpeg ${ffmpeg_version} (avformat, avcodec, avutil, swresample, swscale DLLs),
  LGPL-2.1-or-later, built without GPL or non-free parts.
- mpg123 ${mpg123_version}, LGPL-2.1.
- libopenmpt, BSD-3-Clause; libogg and libvorbis, BSD-3-Clause; zlib, zlib licence.

The exact sources of FFmpeg and mpg123, with the vcpkg build scripts and patches used, are
published next to this build as "$(basename "$dist")-lgpl-sources.tar.gz". You may replace
these DLLs with your own builds of the same libraries.
TXT
# Corresponding source of the LGPL libraries: the source archives vcpkg used and its ports.
"$vcpkg/vcpkg.exe" install "${args[@]}" --only-downloads "${ports[@]}" > /dev/null
src="$work/lgpl-sources"
rm -rf "$src"
mkdir -p "$src/downloads" "$src/ports"
cp "$vcpkg"/downloads/*FFmpeg* "$vcpkg"/downloads/*mpg123* "$src/downloads/"
cp -r "$vcpkg/ports/ffmpeg" "$vcpkg/ports/mpg123" "$src/ports/"
tar -C "$src" -czf "$dist/../$(basename "$dist")-lgpl-sources.tar.gz" downloads ports
