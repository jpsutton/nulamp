#!/usr/bin/env bash
# Builds the libraries bundled in the Linux AppImage into $PREFIX (default ./appimage-deps):
#
# - FFmpeg as an LGPL-only build (no --enable-gpl, no --enable-nonfree) with just the decoders,
#   demuxers and parsers nulamp's FFmpeg decoder uses. Ubuntu's FFmpeg packages are GPL builds,
#   which nulamp must not bundle.
# - libopenmpt (BSD-3-Clause) without mpg123 (LGPL) and libvorbis, so it brings no other
#   bundled libraries.
#
# Needs: a C/C++ toolchain, nasm, pkg-config, curl.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/versions.sh"
PREFIX="$(realpath -m "${PREFIX:-appimage-deps}")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
jobs="$(nproc)"

fetch() { # url sha256 file
  curl -sSfL --retry 3 -o "$work/$3" "$1"
  echo "$2  $work/$3" | sha256sum -c --quiet
}

fetch "$FFMPEG_URL" "$FFMPEG_SHA256" ffmpeg.tar.xz
mkdir -p "$PREFIX/src"
cp "$work/ffmpeg.tar.xz" "$PREFIX/src/ffmpeg-${FFMPEG_VERSION}.tar.xz"
tar -xf "$work/ffmpeg.tar.xz" -C "$work"
decoders="aac,aac_latm,alac,ape,flac,mp1,mp1float,mp2,mp2float,mp3,mp3float,mp3adu,mpc7,mpc8,\
opus,tta,vorbis,wavpack,wmalossless,wmapro,wmav1,wmav2,wmavoice,\
pcm_alaw,pcm_f32be,pcm_f32le,pcm_f64be,pcm_f64le,pcm_mulaw,pcm_s16be,pcm_s16le,pcm_s24be,\
pcm_s24le,pcm_s32be,pcm_s32le,pcm_s8,pcm_u8,adpcm_ima_wav,adpcm_ms,gsm,gsm_ms"
demuxers="aac,aiff,ape,asf,flac,mov,mp3,mpc,mpc8,ogg,tta,wav,wv"
parsers="aac,aac_latm,flac,mpegaudio,opus,vorbis"
(
  cd "$work/ffmpeg-${FFMPEG_VERSION}"
  ./configure --prefix="$PREFIX" --enable-shared --disable-static \
    --disable-programs --disable-doc --disable-network --disable-autodetect \
    --disable-avdevice --disable-avfilter --disable-swscale \
    --disable-everything --enable-protocol=file \
    --enable-decoder="$decoders" --enable-demuxer="$demuxers" --enable-parser="$parsers" \
    > "$work/ffmpeg-configure.log"
  if ! grep -q "^License: LGPL" "$work/ffmpeg-configure.log"; then
    echo "FFmpeg is not configured as LGPL:" >&2
    grep "^License:" "$work/ffmpeg-configure.log" >&2
    exit 1
  fi
  make -j"$jobs" > /dev/null
  make install > /dev/null
)
grep -h "^License:" "$work/ffmpeg-configure.log"
cp "$work/ffmpeg-${FFMPEG_VERSION}/COPYING.LGPLv2.1" "$PREFIX/src/"

fetch "$OPENMPT_URL" "$OPENMPT_SHA256" openmpt.tar.gz
cp "$work/openmpt.tar.gz" "$PREFIX/src/libopenmpt-${OPENMPT_VERSION}+release.autotools.tar.gz"
tar -xzf "$work/openmpt.tar.gz" -C "$work"
(
  cd "$work"/libopenmpt-"${OPENMPT_VERSION}"+release.autotools
  ./configure --prefix="$PREFIX" --disable-static --disable-openmpt123 --disable-examples \
    --disable-tests --disable-doxygen-doc \
    --without-mpg123 --without-ogg --without-vorbis --without-vorbisfile \
    --without-portaudio --without-portaudiocpp --without-pulseaudio --without-sndfile \
    --without-flac > /dev/null
  make -j"$jobs" > /dev/null
  make install > /dev/null
)
echo "dependencies installed in $PREFIX"
