#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo fmt --manifest-path "$ROOT/core/Cargo.toml" --check
cargo clippy --manifest-path "$ROOT/core/Cargo.toml" --all-targets -- -D warnings
cargo test --manifest-path "$ROOT/core/Cargo.toml"
