#!/usr/bin/env bash
# Point this clone at .githooks/ (fmt, clippy, web tsc on commit).
set -euo pipefail
cd "$(dirname "$0")/.."
git config core.hooksPath .githooks
echo "core.hooksPath=$(git config --get core.hooksPath)"
