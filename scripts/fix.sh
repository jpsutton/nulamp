#!/usr/bin/env bash
# Apply clippy suggestions, then format.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo clippy --fix --allow-dirty --allow-staged --workspace --all-targets -q || true
cargo fmt --all
