#!/usr/bin/env bash
# Builds dist/nulamp_<version>-1_ubuntu<release>_amd64.deb on the Ubuntu release it runs on
# (CI runs it in an ubuntu:<release> container). Dependencies are the distribution's own.
set -euo pipefail
cd "$(dirname "$0")/../.."
./scripts/ci-deps.sh
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends curl ca-certificates \
  build-essential dpkg-dev
if ! command -v cargo > /dev/null; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  . "$HOME/.cargo/env"
fi
cargo install --locked --quiet cargo-deb@3.8.0
. /etc/os-release
version=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p}' Cargo.toml)
mkdir -p dist
# No "~" in the file name: GitHub replaces it with "." in release assets, which breaks
# SHA256SUMS. The package version inside keeps "1~ubuntu<release>".
deb="dist/nulamp_${version}-1_ubuntu${VERSION_ID}_amd64.deb"
cargo deb --locked -p nulamp --deb-revision "1~ubuntu${VERSION_ID}" --output "$deb"
echo "$deb: $(dpkg-deb -f "$deb" Depends)"
