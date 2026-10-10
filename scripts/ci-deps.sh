#!/usr/bin/env bash
# Installs the system libraries needed to build and test nulamp on CI runners.
set -euo pipefail
# Containers run as root without sudo.
sudo=""
if [ "$(id -u)" != 0 ]; then sudo=sudo; fi
case "$(uname -s)" in
  Linux)
    $sudo apt-get update
    DEBIAN_FRONTEND=noninteractive $sudo apt-get install -y --no-install-recommends \
      pkg-config cmake \
      libfreetype-dev libfontconfig1-dev \
      libx11-dev libx11-xcb-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
      libxcursor-dev libxrandr-dev libxi-dev libwayland-dev \
      libasound2-dev libpulse-dev libdbus-1-dev libopenmpt-dev libcdio-dev \
      libavformat-dev libavcodec-dev libavutil-dev libswresample-dev ffmpeg \
      fonts-dejavu-core fonts-liberation
    ;;
  Darwin)
    # FreeType is built from source by freetype-sys when not installed.
    brew list freetype >/dev/null 2>&1 || brew install freetype
    brew list libopenmpt >/dev/null 2>&1 || brew install libopenmpt
    # libcdio is only used by the opt-in `cdio` feature (GPL; docs/decisions.md CD1).
    brew list libcdio >/dev/null 2>&1 || brew install libcdio
    brew list ffmpeg >/dev/null 2>&1 || brew install ffmpeg
    ;;
  *)
    # Windows: FreeType is built from the bundled sources (nulamp-text enables freetype-rs
    # "bundled"); run scripts/windows-zlib.sh after this for the zlib headers it needs.
    # libopenmpt and FFmpeg are not installed; their decoders are left out of the build.
    ;;
esac
