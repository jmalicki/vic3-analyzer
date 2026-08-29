#!/usr/bin/env bash
# Pack rust-cache-like paths for the compression bake-off.
set -euo pipefail
: "${CARGO_HOME:?}"
: "${GITHUB_WORKSPACE:?}"

rm -rf "${CARGO_HOME}/registry/src"
find "${GITHUB_WORKSPACE}/target" -type d -name incremental -prune -exec rm -rf {} + 2>/dev/null || true

OUT="${RUNNER_TEMP}/corpus"
mkdir -p "$OUT"
# Fast transport compression (not a bake-off candidate).
tar -cf - \
  --exclude='registry/src' \
  -C "$(dirname "$CARGO_HOME")" "$(basename "$CARGO_HOME")" \
  -C "$GITHUB_WORKSPACE" target \
  | zstd -1 --long=31 -T0 -o "$OUT/corpus.tar.zst"

ls -lh "$OUT/corpus.tar.zst" >&2
echo "$OUT/corpus.tar.zst"
