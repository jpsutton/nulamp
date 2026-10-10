#!/usr/bin/env bash
# Run the checks every commit must pass.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# The opt-in audio CD backend (libcdio, GPL) is not in the default build.
cargo clippy -p nulamp-decoders --all-targets --features cdio -- -D warnings
cargo test -p nulamp-decoders --features cdio --lib cd::
