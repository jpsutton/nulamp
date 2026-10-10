#!/usr/bin/env bash
# Cross-checks the Windows build from Linux with Docker: clippy (-D warnings) for
# x86_64-pc-windows-gnu, then the whole test suite run under Wine.
# Usage: scripts/windows-check.sh [clippy|test] [cargo test args]
# Build output is kept in the Docker volume `nulamp-wintarget`.
set -euo pipefail
cd "$(dirname "$0")/.."
mode=${1:-all}
shift || true
docker build -q -t nulamp-wincheck scripts/wincheck >/dev/null
inner='set -euo pipefail
T=x86_64-pc-windows-gnu
cargo build -q -p libz-sys --target $T
inc=$(ls -d /target/$T/debug/build/libz-sys-*/out/include | head -1)
export CFLAGS_x86_64_pc_windows_gnu="-I$inc"
# libopus is built with stack protection under mingw.
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C link-arg=-lssp"
mode=$1; shift
if [ "$mode" != test ]; then
  cargo clippy --workspace --all-targets --target $T -- -D warnings
fi
if [ "$mode" != clippy ]; then
  cargo test --target $T "${@:---workspace}"
fi'
docker run --rm -v "$PWD":/src:ro -v nulamp-wintarget:/target \
  -v nulamp-cargo:/usr/local/cargo/registry -e CARGO_TARGET_DIR=/target -w /src \
  nulamp-wincheck bash -c "$inner" bash "$mode" "$@"
