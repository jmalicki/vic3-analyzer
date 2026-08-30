#!/usr/bin/env bash
# Local lint that matches CI's fmt/clippy plus the web typecheck CI runs in `pnpm --filter web run build`.
set -euo pipefail
cd "$(dirname "$0")/.."
./scripts/lint-rust.sh
if [[ ! -d node_modules ]]; then
  echo "node_modules missing; run: pnpm install" >&2
  exit 1
fi
pnpm --filter web run typecheck
