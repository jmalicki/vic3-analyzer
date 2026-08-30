#!/usr/bin/env bash
# Rust lint steps from .github/workflows/ci.yml (fmt + clippy). Tests stay out.
set -euo pipefail
cd "$(dirname "$0")/.."
profile_args=()
if [[ -n "${CARGO_PROFILE:-}" && "${CARGO_PROFILE}" != dev ]]; then
  profile_args=(--profile "$CARGO_PROFILE")
fi
cargo fmt --all -- --check
cargo clippy --workspace --all-targets "${profile_args[@]}" -- -D warnings
