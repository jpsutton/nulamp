#!/usr/bin/env bash
# Installs the system libraries needed to build and test nulamp on CI runners.
set -euo pipefail
case "$(uname -s)" in
  Linux)
    sudo apt-get update
    sudo apt-get install -y --no-install-recommends \
      pkg-config cmake \
      libfreetype-dev libfontconfig1-dev \
      libx11-dev libx11-xcb-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
      libxcursor-dev libxrandr-dev libxi-dev libwayland-dev \
      libasound2-dev libpulse-dev libdbus-1-dev \
      fonts-dejavu-core fonts-liberation
    ;;
  Darwin)
    # FreeType is built from source by freetype-sys when not installed.
    brew list freetype >/dev/null 2>&1 || brew install freetype
    ;;
  *)
    # Windows: freetype-sys builds a bundled FreeType with CMake (preinstalled on runners).
    ;;
esac
