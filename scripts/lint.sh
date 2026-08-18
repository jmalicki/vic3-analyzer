#!/usr/bin/env bash
# Local lint that matches CI's fmt/clippy plus the web typecheck CI runs in `npm run build`.
set -euo pipefail
cd "$(dirname "$0")/.."
./scripts/lint-rust.sh
if [[ ! -d web/node_modules ]]; then
  echo "web/node_modules missing; run: (cd web && npm ci)" >&2
  exit 1
fi
(cd web && npm run typecheck)
