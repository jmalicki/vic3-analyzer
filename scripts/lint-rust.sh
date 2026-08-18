#!/usr/bin/env bash
# Rust lint steps from .github/workflows/ci.yml (fmt + clippy). Tests stay out.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
