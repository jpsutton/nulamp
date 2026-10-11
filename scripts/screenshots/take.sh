#!/usr/bin/env bash
# Regenerates docs/screenshots/{player,library}.png at 2x in a container (no window or sound on
# the host). Usage: scripts/screenshots/take.sh <folder with the music to show>
# Uses target/release/nulamp: build it first (cargo build --release -p nulamp).
set -euo pipefail
cd "$(dirname "$0")/../.."
music="$(realpath "$1")"
docker build -q -t nulamp-screenshots scripts/screenshots > /dev/null
docker run --rm -u "$(id -u):$(id -g)" -e HOME=/tmp -v "$music":/work/music:ro -v "$PWD/docs/screenshots":/work/out \
  -v "$PWD/scripts/screenshots/shoot.sh":/shoot.sh:ro \
  -v "$PWD/target/release/nulamp":/usr/local/bin/nulamp:ro nulamp-screenshots bash /shoot.sh
ls -la docs/screenshots
